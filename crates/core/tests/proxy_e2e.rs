//! End-to-end test for the built-in MITM proxy engine: a real HTTPS client
//! routed through the loopback engine must have its inference request
//! rewritten to the Gate gateway with the `X-Gate-Api-Key` /
//! `X-Gate-Upstream-Url` headers injected, while its own credential is left
//! intact.
//!
//! This drives `proxy::engine::start` directly - the same engine the CLI's
//! `proxy enable` and the menubar app boot - so it exercises the only path in
//! the app that emits real gateway-bound traffic. It is fully hermetic: a
//! throwaway CA, a loopback mock gateway, and clients that never leave
//! loopback - an in-process `reqwest`, plus a real `curl` subprocess routed
//! solely by the `https_proxy` env var, proving the engine also intercepts a
//! black-box external client it did not construct (the config-less-app case).
//! Nothing touches the OS trust store or system proxy, so no elevation is
//! needed and it runs identically on macOS, Windows, and Linux.
//!
//! Deliberately out of scope here: a full `gate-connect proxy enable` run that
//! wires the real OS system proxy and installs the engine's MITM CA into the OS
//! trust store, then drives a naive client that validates the leaf via the OS
//! store . That path can't be exercised hermetically against a
//! loopback mock, because the engine's upstream connector (hudsucker's
//! webpki-roots) validates the gateway cert against Mozilla roots only - it
//! won't trust a private test CA on the engine->gateway leg - and
//! `account::save` rejects a plain-http gateway that would otherwise sidestep
//! that TLS. Closing it would take an engine change (a configurable upstream
//! root) or bypassing the account guard, so the real-enable + OS-trust wiring
//! stays unverified by automated tests for now; `ci/e2e/run.sh` covers the
//! config-file integration path instead, not the built-in proxy.

use std::sync::{Arc, Mutex};

use bytes::Bytes;
use http_body_util::Empty;
use hyper::body::Incoming;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Request, Response};
use hyper_util::rt::TokioIo;
use rcgen::{BasicConstraints, CertificateParams, DnType, IsCa, KeyPair, KeyUsagePurpose};
use tokio::net::TcpListener;

use gate_connect_core::proxy::default_domains;
use gate_connect_core::proxy::engine::{self, EngineConfig};

/// Serializes the tests in this file. Loopback ports are a shared resource:
/// the restart tests free a port and expect to re-bind it, and any
/// concurrently running test can defeat that by (re)allocating from the same
/// ephemeral range in the gap - listeners and outbound client connections
/// alike. Every test takes this guard first, so the file runs one test at a
/// time. A tokio mutex, not std: the guard is held across await points
/// (clippy's await_holding_lock), and it releases cleanly when a failing
/// test panics instead of poisoning.
static SERIAL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// One request the mock gateway received, reduced to what we assert on.
#[derive(Clone)]
struct Captured {
    method: String,
    path: String,
    headers: Vec<(String, String)>,
}

impl Captured {
    /// Header lookup by lowercase name (hyper normalises names to lowercase).
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
    }
}

/// A loopback HTTP server standing in for the Gate gateway. Records every
/// request it receives and answers 200 with an empty body.
struct MockGateway {
    base_url: String,
    captured: Arc<Mutex<Vec<Captured>>>,
}

async fn start_mock_gateway() -> MockGateway {
    let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let captured: Arc<Mutex<Vec<Captured>>> = Arc::new(Mutex::new(Vec::new()));

    let cap = Arc::clone(&captured);
    tokio::spawn(async move {
        loop {
            let Ok((stream, _)) = listener.accept().await else {
                break;
            };
            let cap = Arc::clone(&cap);
            tokio::spawn(async move {
                let service = service_fn(move |req: Request<Incoming>| {
                    let cap = Arc::clone(&cap);
                    async move {
                        cap.lock().unwrap().push(Captured {
                            method: req.method().to_string(),
                            path: req.uri().path().to_string(),
                            headers: req
                                .headers()
                                .iter()
                                .map(|(k, v)| {
                                    (k.as_str().to_string(), v.to_str().unwrap_or("").to_string())
                                })
                                .collect(),
                        });
                        Ok::<_, std::convert::Infallible>(Response::new(Empty::<Bytes>::new()))
                    }
                });
                let _ = http1::Builder::new()
                    .serve_connection(TokioIo::new(stream), service)
                    .await;
            });
        }
    });

    MockGateway {
        base_url: format!("http://127.0.0.1:{port}"),
        captured,
    }
}

/// Mint a throwaway, unconstrained root CA and return its cert + key as PEM.
/// The engine builds its per-host leaf signer from these; the test client
/// trusts the cert so the MITM handshake to `api.anthropic.com` succeeds.
fn mint_ca() -> (String, String) {
    let mut params =
        CertificateParams::new(Vec::<String>::new()).expect("building CA certificate params");
    params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    params.key_usages = vec![
        KeyUsagePurpose::KeyCertSign,
        KeyUsagePurpose::CrlSign,
        KeyUsagePurpose::DigitalSignature,
    ];
    params
        .distinguished_name
        .push(DnType::CommonName, "Gate Connect Test CA");
    let key = KeyPair::generate().expect("generating CA key pair");
    let cert = params.self_signed(&key).expect("self-signing CA cert");
    (cert.pem(), key.serialize_pem())
}

#[tokio::test]
async fn proxy_rewrites_intercepted_request_to_gateway() {
    let _serial = SERIAL.lock().await;
    // 1. Mock gateway on loopback. The rewritten request must land here.
    let gateway = start_mock_gateway().await;

    // 2. Boot the real engine. `default_domains()` already ships Anthropic
    //    enabled, so api.anthropic.com /v1/ is intercepted and rewritten.
    let (ca_cert_pem, ca_key_pem) = mint_ca();
    let engine = engine::start(
        EngineConfig {
            gateway_base_url: gateway.base_url.clone(), // http://127.0.0.1:<port>
            api_key: "sk-gw-test".into(),
            oauth_token: String::new(), // legacy API-key path
            domains: default_domains(),
            ca_cert_pem: ca_cert_pem.clone(),
            ca_key_pem,
            preferred_port: None,
            preferred_pac_port: None,
            preferred_relay_port: None,
            owner_uid: None,
            upstream_proxy: None,
        },
        || {},
    )
    .expect("proxy engine should start");

    // 3. A client routed through the engine, trusting our throwaway CA so the
    //    engine's MITM leaf for api.anthropic.com validates.
    let client = reqwest::Client::builder()
        .proxy(reqwest::Proxy::all(format!("http://127.0.0.1:{}", engine.port())).unwrap())
        .add_root_certificate(reqwest::Certificate::from_pem(ca_cert_pem.as_bytes()).unwrap())
        .build()
        .unwrap();

    let resp = client
        .post("https://api.anthropic.com/v1/messages")
        .header("authorization", "Bearer app-token")
        .json(&serde_json::json!({ "model": "claude", "messages": [] }))
        .send()
        .await
        .expect("request should reach the gateway through the proxy");
    assert!(
        resp.status().is_success(),
        "gateway returned {}",
        resp.status()
    );

    engine.stop();

    // 4. The rewrite landed on the gateway: original path preserved, Gate
    //    headers injected, the client's own bearer left intact.
    let reqs = gateway.captured.lock().unwrap().clone();
    assert_eq!(
        reqs.len(),
        1,
        "gateway should have received exactly one request"
    );
    let r = &reqs[0];
    assert_eq!(r.method, "POST");
    assert_eq!(r.path, "/v1/messages");
    assert_eq!(r.header("x-gate-api-key"), Some("sk-gw-test"));
    assert_eq!(
        r.header("x-gate-upstream-url"),
        Some("https://api.anthropic.com")
    );
    assert_eq!(
        r.header("authorization"),
        Some("Bearer app-token"),
        "the client's own credential must be forwarded untouched"
    );
}

/// The same rewrite guarantee, but for a **real external process** routed
/// through the engine solely by the `https_proxy` environment variable - the
/// mechanism config-less, env-honoring apps use. Unlike the tests above, the
/// engine never constructs this client: `curl` speaks CONNECT to the loopback
/// port, does a real TLS handshake against the engine's MITM leaf for
/// api.anthropic.com , and the engine must
/// still rewrite `/v1/messages` to the gateway with the Gate headers injected.
#[tokio::test]
async fn proxy_intercepts_external_process_routed_by_proxy_env() {
    let _serial = SERIAL.lock().await;
    // 1. Mock gateway on loopback (plain HTTP, so the engine->gateway hop needs
    //    no upstream trust and the test stays hermetic).
    let gateway = start_mock_gateway().await;

    // 2. Boot the real engine. `default_domains()` ships Anthropic enabled.
    let (ca_cert_pem, ca_key_pem) = mint_ca();
    let engine = engine::start(
        EngineConfig {
            gateway_base_url: gateway.base_url.clone(),
            api_key: "sk-gw-test".into(),
            oauth_token: String::new(), // legacy API-key path
            domains: default_domains(),
            ca_cert_pem: ca_cert_pem.clone(),
            ca_key_pem,
            preferred_port: None,
            preferred_pac_port: None,
            preferred_relay_port: None,
            owner_uid: None,
            upstream_proxy: None,
        },
        || {},
    )
    .expect("proxy engine should start");

    // 3. Persist the CA so an out-of-process client can trust the MITM leaf.
    let ca_path = std::env::temp_dir().join(format!(
        "gate-proxy-e2e-ca-{}-{}.pem",
        std::process::id(),
        engine.port()
    ));
    std::fs::write(&ca_path, &ca_cert_pem).expect("writing CA to temp file");

    // 4. Drive `curl` - a client the engine did not build - routed only by the
    //    proxy env vars. spawn_blocking keeps the (single-threaded) test runtime
    //    free to serve the mock while curl runs. NO_PROXY is cleared so an
    //    inherited exclusion can't exempt the host.
    let proxy_url = format!("http://127.0.0.1:{}", engine.port());
    let ca_arg = ca_path.clone();
    let output = tokio::task::spawn_blocking(move || {
        std::process::Command::new("curl")
            .arg("-sS")
            // (schannel/Windows) the MITM leaf has no CRL/OCSP endpoint, so
            // schannel returns CERT_TRUST_REVOCATION_STATUS_UNKNOWN and fails
            // verification (exit 60). No-op on OpenSSL/SecureTransport builds.
            .arg("--ssl-no-revoke")
            .arg("--fail") // nonzero exit unless the gateway answers 2xx
            .arg("--cacert")
            .arg(&ca_arg)
            .arg("-X")
            .arg("POST")
            .arg("-H")
            .arg("authorization: Bearer app-token")
            .arg("-H")
            .arg("content-type: application/json")
            .arg("--data")
            .arg(r#"{"model":"claude","messages":[]}"#)
            .arg("https://api.anthropic.com/v1/messages")
            .env("https_proxy", &proxy_url)
            .env("HTTPS_PROXY", &proxy_url)
            .env("no_proxy", "")
            .env("NO_PROXY", "")
            .output()
    })
    .await
    .expect("joining the curl task")
    .expect("curl must be installed to run this test");

    engine.stop();
    let _ = std::fs::remove_file(&ca_path);

    assert!(
        output.status.success(),
        "curl through the proxy failed: status={:?}\nstdout={}\nstderr={}",
        output.status.code(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );

    // 5. Same rewrite guarantees as the in-process test, now proven for an
    //    external process configured only via proxy env vars.
    let reqs = gateway.captured.lock().unwrap().clone();
    assert_eq!(
        reqs.len(),
        1,
        "gateway should have received exactly one request"
    );
    let r = &reqs[0];
    assert_eq!(r.method, "POST");
    assert_eq!(r.path, "/v1/messages");
    assert_eq!(r.header("x-gate-api-key"), Some("sk-gw-test"));
    assert_eq!(
        r.header("x-gate-upstream-url"),
        Some("https://api.anthropic.com")
    );
    assert_eq!(
        r.header("authorization"),
        Some("Bearer app-token"),
        "the external client's own credential must be forwarded untouched"
    );
}

#[tokio::test]
async fn proxy_rewrites_openrouter_request_to_gateway() {
    let _serial = SERIAL.lock().await;
    // 1. Mock gateway on loopback. The rewritten request must land here.
    let gateway = start_mock_gateway().await;

    // 2. Boot the engine with OpenRouter opted in. `default_domains()` ships
    //    it `enabled: false` (proxy-only, opt-in), so flip it on so
    //    openrouter.ai /api/ is intercepted and rewritten.
    let (ca_cert_pem, ca_key_pem) = mint_ca();
    let domains = default_domains()
        .into_iter()
        .map(|mut d| {
            if d.slug == "openrouter" {
                d.enabled = true;
            }
            d
        })
        .collect();
    let engine = engine::start(
        EngineConfig {
            gateway_base_url: gateway.base_url.clone(), // http://127.0.0.1:<port>
            api_key: "sk-gw-test".into(),
            // Exercises the OAuth path end-to-end: a stored token is injected
            // as x-gate-authorization instead of the API key.
            oauth_token: "cognito-access-token".into(),
            domains,
            ca_cert_pem: ca_cert_pem.clone(),
            ca_key_pem,
            preferred_port: None,
            preferred_pac_port: None,
            preferred_relay_port: None,
            owner_uid: None,
            upstream_proxy: None,
        },
        || {},
    )
    .expect("proxy engine should start");

    // 3. A client routed through the engine, trusting our throwaway CA so the
    //    engine's MITM leaf for openrouter.ai validates.
    let client = reqwest::Client::builder()
        .proxy(reqwest::Proxy::all(format!("http://127.0.0.1:{}", engine.port())).unwrap())
        .add_root_certificate(reqwest::Certificate::from_pem(ca_cert_pem.as_bytes()).unwrap())
        .build()
        .unwrap();

    let resp = client
        .post("https://openrouter.ai/api/v1/chat/completions")
        .header("authorization", "Bearer app-token")
        .json(&serde_json::json!({ "model": "openai/gpt-4o", "messages": [] }))
        .send()
        .await
        .expect("request should reach the gateway through the proxy");
    assert!(
        resp.status().is_success(),
        "gateway returned {}",
        resp.status()
    );

    engine.stop();

    // 4. The rewrite landed on the gateway: original path preserved (OpenRouter
    //    nests its API under /api/v1/), the OAuth token injected as
    //    x-gate-authorization (not the API key), and the client's own bearer
    //    forwarded untouched.
    let reqs = gateway.captured.lock().unwrap().clone();
    assert_eq!(
        reqs.len(),
        1,
        "gateway should have received exactly one request"
    );
    let r = &reqs[0];
    assert_eq!(r.method, "POST");
    assert_eq!(r.path, "/api/v1/chat/completions");
    assert_eq!(
        r.header("x-gate-authorization"),
        Some("Bearer cognito-access-token")
    );
    assert_eq!(
        r.header("x-gate-api-key"),
        None,
        "the API key must not be sent when an OAuth token is present"
    );
    assert_eq!(
        r.header("x-gate-upstream-url"),
        Some("https://openrouter.ai")
    );
    assert_eq!(
        r.header("authorization"),
        Some("Bearer app-token"),
        "the client's own credential must be forwarded untouched"
    );
}

/// The restart contract the stable-port persistence relies on (Linux login
/// sessions freeze the proxy pointer; macOS/Windows clients may resolve the
/// proxy once at their own launch): an engine restarted with the
/// previously-bound port as `preferred_port` must come back on the same
/// address and still rewrite to the gateway, and a taken preferred port must
/// fall back to an ephemeral one instead of failing the start.
#[tokio::test]
async fn engine_restart_reuses_preferred_port_and_falls_back_when_taken() {
    let _serial = SERIAL.lock().await;
    let gateway = start_mock_gateway().await;
    let (ca_cert_pem, ca_key_pem) = mint_ca();
    let config = |preferred_port: Option<u16>| EngineConfig {
        gateway_base_url: gateway.base_url.clone(),
        api_key: "sk-gw-test".into(),
        oauth_token: String::new(), // legacy API-key path
        domains: default_domains(),
        ca_cert_pem: ca_cert_pem.clone(),
        ca_key_pem: ca_key_pem.clone(),
        preferred_port,
        preferred_pac_port: None,
        preferred_relay_port: None,
        owner_uid: None,
        upstream_proxy: None,
    };

    // First run: ephemeral bind, note where it landed.
    let engine = engine::start(config(None), || {}).expect("first engine start");
    let port = engine.port();
    engine.stop();

    // Restart preferring that port: same address, and it still routes.
    let engine = engine::start(config(Some(port)), || {}).expect("restarted engine start");
    assert_eq!(engine.port(), port, "restart must reuse the preferred port");

    let client = reqwest::Client::builder()
        .proxy(reqwest::Proxy::all(format!("http://127.0.0.1:{port}")).unwrap())
        .add_root_certificate(reqwest::Certificate::from_pem(ca_cert_pem.as_bytes()).unwrap())
        .build()
        .unwrap();
    let resp = client
        .post("https://api.anthropic.com/v1/messages")
        .header("authorization", "Bearer app-token")
        .json(&serde_json::json!({ "model": "claude", "messages": [] }))
        .send()
        .await
        .expect("request should route through the restarted engine");
    assert!(
        resp.status().is_success(),
        "gateway returned {}",
        resp.status()
    );
    engine.stop();

    // Preferred port taken by someone else: fall back to an ephemeral port
    // rather than failing the start.
    let blocker = std::net::TcpListener::bind(("127.0.0.1", port)).expect("occupying the port");
    let engine = engine::start(config(Some(port)), || {}).expect("fallback engine start");
    assert_ne!(
        engine.port(),
        port,
        "a taken preferred port must fall back to an ephemeral one"
    );
    engine.stop();
    drop(blocker);
}

/// Same restart contract for the PAC listener (PAC-driven platforms only):
/// the `AutoConfigURL` a client captured bakes the PAC port in, so a restart
/// must serve a fresh PAC - pointing at the live engine port - from the same
/// address.
#[cfg(any(target_os = "windows", target_os = "macos"))]
#[tokio::test]
async fn pac_restart_reuses_preferred_port_and_serves_live_engine_port() {
    let _serial = SERIAL.lock().await;
    let gateway = start_mock_gateway().await;
    let (ca_cert_pem, ca_key_pem) = mint_ca();
    let config = |preferred_pac_port: Option<u16>| EngineConfig {
        gateway_base_url: gateway.base_url.clone(),
        api_key: "sk-gw-test".into(),
        domains: default_domains(),
        ca_cert_pem: ca_cert_pem.clone(),
        ca_key_pem: ca_key_pem.clone(),
        preferred_port: None,
        preferred_pac_port,
        owner_uid: None,
        upstream_proxy: None,
    };

    // First run: note the PAC port.
    let engine = engine::start(config(None), || {}).expect("first engine start");
    let pac_port = engine.pac_port();
    engine.stop();

    // Restart preferring it: same address, and the served PAC points at the
    // *new* engine port, not a stale one.
    let engine = engine::start(config(Some(pac_port)), || {}).expect("restarted engine start");
    assert_eq!(
        engine.pac_port(),
        pac_port,
        "restart must reuse the preferred PAC port"
    );
    let pac = reqwest::get(format!("http://127.0.0.1:{pac_port}/proxy.pac"))
        .await
        .expect("fetching the PAC")
        .text()
        .await
        .expect("reading the PAC body");
    assert!(
        pac.contains(&format!("PROXY 127.0.0.1:{}", engine.port())),
        "PAC must route to the live engine port; got:\n{pac}"
    );
    engine.stop();

    // Taken PAC port: fall back rather than failing the start.
    let blocker =
        std::net::TcpListener::bind(("127.0.0.1", pac_port)).expect("occupying the PAC port");
    let engine = engine::start(config(Some(pac_port)), || {}).expect("fallback engine start");
    assert_ne!(
        engine.pac_port(),
        pac_port,
        "a taken preferred PAC port must fall back to an ephemeral one"
    );
    engine.stop();
    drop(blocker);
}
