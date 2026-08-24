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
//! loopback mock. The engine's upstream connector used to validate the gateway
//! cert against Mozilla roots only, so it would not trust a private test CA on
//! the engine->gateway leg; that half is now closed - `engine::upstream_tls_config`
//! adds a root named by `GATE_CONNECT_TEST_CA`, the same seam the relay reads.
//! What remains is `account::save` rejecting a plain-http gateway, so a test
//! wanting the real-enable path must serve HTTPS from that CA rather than reuse
//! the http mock below. `ci/e2e/run.sh` exercises it end to end that way; a
//! hermetic version belongs here and does not exist yet.

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
            org_id: String::new(),      // no org on the legacy path
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

/// Whether the `curl` on PATH was built with HTTP/2, read off its `Features:`
/// line. Windows ships a Schannel build with no nghttp2, which rejects
/// `--http2` outright.
fn curl_supports_http2() -> bool {
    std::process::Command::new("curl")
        .arg("-V")
        .output()
        .map(|out| {
            String::from_utf8_lossy(&out.stdout)
                .lines()
                .filter_map(|line| line.strip_prefix("Features:"))
                .any(|features| features.split_whitespace().any(|f| f == "HTTP2"))
        })
        .unwrap_or(false)
}

/// A request whose header block exceeds 16 KiB must still be intercepted and
/// rewritten, not answered `431` by our own listener.
///
/// The engine's h2 server is where this regresses, and it regresses by
/// OMISSION: hyper's HTTP/2 side inherits the `h2` crate's
/// `SETTINGS_MAX_HEADER_LIST_SIZE` default of 16 KiB, and hudsucker's fallback
/// server builder configures only `http1()`, so unless `engine::start` supplies
/// its own builder the limit is silently ours. It cost us the browser surfaces:
/// chatgpt.com's web client sends ~8.3 KB of its own headers
/// (`x-oai-is-pending-updates` alone measured 5446 B, and it grows until the
/// server acks it) plus a session cookie jar, so every call - including the
/// `/f/conversation/prepare` that precedes the first chat turn - came back a
/// bare `431`, no headers and no body. The chat wedges before it sends a turn,
/// and because the refusal is ours nothing reaches the gateway to explain why.
///
/// HTTP/2 is forced and then verified, rather than assumed. The same request
/// passes over h1 on hyper's far more generous limits (measured: 20 KB fine), so
/// a client that quietly negotiated HTTP/1.1 would keep this test green while
/// the regression was back for every browser. That rules out the in-process
/// `reqwest` client the tests above use: this workspace pins it with
/// `default-features = false` and no `http2` feature, so it cannot speak h2 at
/// all. `curl` can, and `%{http_version}` is asserted so a curl built without
/// HTTP/2 fails loudly instead of silently covering nothing.
///
/// Windows is the one exception: its bundled Schannel `curl.exe` has no nghttp2
/// and cannot drive h2 at all, so the test skips there rather than report a
/// limit it never reached. What is under test is the engine's own
/// `ServerBuilder` config, which is not platform-specific, and the Linux and
/// macOS legs still cover it.
#[tokio::test]
async fn proxy_accepts_oversized_h2_request_headers() {
    if !curl_supports_http2() {
        // Only Windows gets the pass. Anywhere else, a curl without HTTP/2
        // would negotiate h1, sail past a limit h1 does not have, and leave the
        // test green over nothing - the exact hole the version assert guards.
        assert!(
            cfg!(windows),
            "curl must be built with HTTP/2 for this test to cover anything; an \
             h1 run would pass without ever reaching the header limit"
        );
        eprintln!(
            "skipping proxy_accepts_oversized_h2_request_headers: this curl has \
             no HTTP/2 support (expected on Windows, whose bundled Schannel \
             build has no nghttp2)"
        );
        return;
    }

    let _serial = SERIAL.lock().await;
    let gateway = start_mock_gateway().await;

    let (ca_cert_pem, ca_key_pem) = mint_ca();
    let engine = engine::start(
        EngineConfig {
            gateway_base_url: gateway.base_url.clone(),
            api_key: "sk-gw-test".into(),
            oauth_token: String::new(),
            org_id: String::new(),
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

    let ca_path = std::env::temp_dir().join(format!(
        "gate-proxy-e2e-bighdr-ca-{}-{}.pem",
        std::process::id(),
        engine.port()
    ));
    std::fs::write(&ca_path, &ca_cert_pem).expect("writing CA to temp file");

    // 20 KB in one header: over the 16 KiB default, under the 64 KiB the engine
    // now sets. Sized from the traffic that broke this rather than to the
    // boundary - the ChatGPT state blob grows across a session.
    let big = "a".repeat(20 * 1024);
    let proxy_url = format!("http://127.0.0.1:{}", engine.port());
    let ca_arg = ca_path.clone();
    let big_header = format!("x-big: {big}");
    let output = tokio::task::spawn_blocking(move || {
        std::process::Command::new("curl")
            .arg("-sS")
            .arg("--ssl-no-revoke") // (schannel) same reason as the test above
            .arg("--http2") // the whole point: h1 does not exercise the limit
            // No `--fail`: the refusal we are guarding against IS an HTTP
            // status, so it has to be read rather than turned into an exit code.
            // The mock gateway answers with an empty body, so stdout is exactly
            // this template.
            .arg("-w")
            .arg("%{http_code} %{http_version}")
            .arg("--cacert")
            .arg(&ca_arg)
            .arg("-X")
            .arg("POST")
            .arg("-H")
            .arg("authorization: Bearer app-token")
            .arg("-H")
            .arg(&big_header)
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

    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    assert_eq!(
        stdout,
        "200 2",
        "expected 200 over HTTP/2; got `{stdout}` (a `431 2` here is the engine \
         refusing the header block, `200 1.1` means curl fell back to h1 and the \
         test no longer covers the limit)\nstderr={}",
        String::from_utf8_lossy(&output.stderr),
    );

    // The rewrite still happened, and the oversized header rode along rather
    // than being dropped somewhere in the middle.
    let reqs = gateway.captured.lock().unwrap().clone();
    assert_eq!(
        reqs.len(),
        1,
        "gateway should have received exactly one request"
    );
    let r = &reqs[0];
    assert_eq!(r.path, "/v1/messages");
    assert_eq!(r.header("x-gate-api-key"), Some("sk-gw-test"));
    assert_eq!(
        r.header("x-big").map(str::len),
        Some(big.len()),
        "the oversized header must reach the gateway intact"
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
            org_id: String::new(),      // no org on the legacy path
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

/// The variables the system proxy actually *exports* must, on their own, be
/// enough to route a black-box process through the engine.
///
/// The test above proves the engine honours `https_proxy`; this one proves the
/// set we ship honours it, which is a different claim. It takes the pairs
/// straight from [`gate_connect_core::proxy::proxy_env_vars`] - the single
/// source the Linux drop-in, macOS `launchctl setenv` and the Windows
/// `HKCU\Environment` writer all feed from - and hands *only* those to `curl`.
/// A typo in a variable name, a proxy pointed at the PAC port instead of the
/// engine, a `NO_PROXY` that accidentally exempts the target, or a
/// `NODE_EXTRA_CA_CERTS` pointing somewhere the CA isn't would each fail here
/// while every unit test still passed.
///
/// This is the mechanism OpenCode depends on entirely: it has no proxy or CA
/// setting in its config schema, so these variables are the only way to route
/// it. `curl` stands in for its Bun HTTP client - the one difference is that
/// curl has no `NODE_EXTRA_CA_CERTS` equivalent, so the exported value is fed
/// to `--cacert`, which still asserts the contract that matters: the path we
/// export is a file that validates the engine's leaf.
#[tokio::test]
async fn exported_proxy_env_routes_an_external_process() {
    let _serial = SERIAL.lock().await;

    // 1. Redirect every per-user path into a throwaway dir *before* anything
    //    resolves one. `proxy_env_vars` derives NODE_EXTRA_CA_CERTS from
    //    `app_support_dir`, so without this the test would write its own CA
    //    over the developer's real one.
    let temp_home = std::env::temp_dir().join(format!(
        "gate-proxy-env-e2e-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let prev_home = std::env::var_os("GATE_CONNECT_TEST_HOME");
    std::env::set_var("GATE_CONNECT_TEST_HOME", &temp_home);

    let gateway = start_mock_gateway().await;

    // 2. Boot the real engine. `default_domains()` ships Anthropic enabled.
    let (ca_cert_pem, ca_key_pem) = mint_ca();
    let engine = engine::start(
        EngineConfig {
            gateway_base_url: gateway.base_url.clone(),
            api_key: "sk-gw-test".into(),
            oauth_token: String::new(),
            org_id: String::new(),
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

    // 3. The production export, for this engine's port.
    let vars = gate_connect_core::proxy::proxy_env_vars(engine.port())
        .expect("resolving the exported proxy environment");

    // The proxy must point at the engine itself. An env-var proxy has no PAC
    // equivalent, so exporting the PAC port here would route every request into
    // a listener that only serves a .pac file.
    let expected_endpoint = format!("http://127.0.0.1:{}", engine.port());
    for name in ["http_proxy", "https_proxy", "HTTP_PROXY", "HTTPS_PROXY"] {
        let value = vars
            .iter()
            .find(|(k, _)| *k == name)
            .map(|(_, v)| v.as_str());
        assert_eq!(
            value,
            Some(expected_endpoint.as_str()),
            "{name} must point at the engine port"
        );
    }

    // 4. Put the engine's CA where *production* keeps it, then point curl at
    //    whatever the exported variable claims. Writing to the exported path
    //    instead would be circular - it would pass even if the export named a
    //    file nothing ever writes - so the two must come from different places
    //    for this to assert anything.
    let real_ca_path =
        gate_connect_core::proxy::ca_cert_path().expect("resolving the production CA path");
    std::fs::create_dir_all(real_ca_path.parent().unwrap()).expect("creating the CA directory");
    std::fs::write(&real_ca_path, &ca_cert_pem).expect("writing the CA where the app keeps it");

    let exported_ca = vars
        .iter()
        .find(|(k, _)| *k == "NODE_EXTRA_CA_CERTS")
        .map(|(_, v)| std::path::PathBuf::from(v))
        .expect("NODE_EXTRA_CA_CERTS must be exported");

    // 5. Drive curl with *only* the exported variables. `env_clear` is
    //    deliberate: an ambient https_proxy or no_proxy in the developer's or
    //    CI's shell could otherwise carry the test, or exempt the target and
    //    sink it, either way telling us nothing about what we export. PATH is
    //    restored so curl itself is still findable.
    let ca_arg = exported_ca.clone();
    let vars_for_curl = vars.clone();
    let path_var = std::env::var_os("PATH").unwrap_or_default();
    let output = tokio::task::spawn_blocking(move || {
        let mut cmd = std::process::Command::new("curl");
        cmd.env_clear().env("PATH", path_var);
        // Windows loads Winsock, DNS and schannel out of the system directory,
        // and finds it through SystemRoot. Clearing that leaves curl unable to
        // open a socket at all: it fails in single-digit milliseconds with
        // "Could not connect to server", which is indistinguishable from the
        // engine being down and is exactly how this test read on CI. Neither
        // variable carries proxy configuration, so restoring them keeps the
        // isolation this `env_clear` exists for.
        #[cfg(windows)]
        for key in ["SystemRoot", "SystemDrive"] {
            if let Some(value) = std::env::var_os(key) {
                cmd.env(key, value);
            }
        }
        for (name, value) in &vars_for_curl {
            cmd.env(name, value);
        }
        cmd.arg("-sS")
            // (schannel/Windows) the MITM leaf has no CRL/OCSP endpoint.
            .arg("--ssl-no-revoke")
            .arg("--fail")
            // curl has no NODE_EXTRA_CA_CERTS; feed it the exported value.
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
            .output()
    })
    .await
    .expect("joining the curl task")
    .expect("curl must be installed to run this test");

    engine.stop();
    let _ = std::fs::remove_dir_all(&temp_home);
    match prev_home {
        Some(v) => std::env::set_var("GATE_CONNECT_TEST_HOME", v),
        None => std::env::remove_var("GATE_CONNECT_TEST_HOME"),
    }

    assert!(
        output.status.success(),
        "curl routed by the exported proxy env failed: status={:?}\nstdout={}\nstderr={}",
        output.status.code(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );

    // 6. The request reached the gateway rewritten - so the exported variables
    //    routed it, and the exported CA path let TLS validate.
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
            // as x-gate-authorization instead of the API key, with the selected
            // org on x-gate-org-id.
            oauth_token: "cognito-access-token".into(),
            org_id: "org-uuid-1".into(),
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

    // 4. The rewrite landed on the gateway with the `/api` moved OFF the request
    //    line and INTO the upstream header. Gate's ALB routes `/api/*` to the
    //    dashboard API, so forwarding `/api/v1/chat/completions` would 404 out
    //    of a service that has no such route - Gate re-joins upstream + path, so
    //    OpenRouter still receives /api/v1/chat/completions. Also: the OAuth
    //    token injected as x-gate-authorization (not the API key), and the
    //    client's own bearer forwarded untouched.
    let reqs = gateway.captured.lock().unwrap().clone();
    assert_eq!(
        reqs.len(),
        1,
        "gateway should have received exactly one request"
    );
    let r = &reqs[0];
    assert_eq!(r.method, "POST");
    assert_eq!(
        r.path, "/v1/chat/completions",
        "the forwarded path must not begin with Gate's reserved /api/ prefix"
    );
    assert_eq!(
        r.header("x-gate-authorization"),
        Some("Bearer cognito-access-token")
    );
    assert_eq!(
        r.header("x-gate-org-id"),
        Some("org-uuid-1"),
        "the selected org must ride alongside the OAuth token"
    );
    assert_eq!(
        r.header("x-gate-api-key"),
        None,
        "the API key must not be sent when an OAuth token is present"
    );
    assert_eq!(
        r.header("x-gate-upstream-url"),
        Some("https://openrouter.ai/api"),
        "the /api segment must travel in the upstream header, not the path"
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
/// fall back to another port instead of failing the start.
#[tokio::test]
async fn engine_restart_reuses_preferred_port_and_falls_back_when_taken() {
    let _serial = SERIAL.lock().await;
    let gateway = start_mock_gateway().await;
    let (ca_cert_pem, ca_key_pem) = mint_ca();
    let config = |preferred_port: Option<u16>| EngineConfig {
        gateway_base_url: gateway.base_url.clone(),
        api_key: "sk-gw-test".into(),
        oauth_token: String::new(), // legacy API-key path
        org_id: String::new(),      // no org on the legacy path
        domains: default_domains(),
        ca_cert_pem: ca_cert_pem.clone(),
        ca_key_pem: ca_key_pem.clone(),
        preferred_port,
        preferred_pac_port: None,
        preferred_relay_port: None,
        owner_uid: None,
        upstream_proxy: None,
    };

    // First run on a port of our own choosing (see `seed_preferred_port`).
    let port = seed_preferred_port();
    let engine = engine::start(config(Some(port)), || {}).expect("first engine start");
    assert_eq!(engine.port(), port, "first run must take the seeded port");
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

    // Preferred port taken by someone else: fall back to another port rather
    // than failing the start. The blocker holds a port of its own instead of
    // the one just freed above: a freed port can be picked up by anything else
    // on the machine (including this suite's other test binaries) between the
    // stop and the rebind, which would make this bind flaky.
    let blocker = std::net::TcpListener::bind(("127.0.0.1", 0)).expect("occupying a port");
    let taken = blocker.local_addr().unwrap().port();
    let engine = engine::start(config(Some(taken)), || {}).expect("fallback engine start");
    assert_ne!(
        engine.port(),
        taken,
        "a taken preferred port must fall back to another one"
    );
    engine.stop();
    drop(blocker);
}

/// A port from the OS's ephemeral range, released before it is returned, for
/// the restart tests to hand back as a *preferred* port.
///
/// They cannot let the engine pick its own: a fresh pick comes from
/// `engine::bind_fresh`'s 100-port band, and a band port freed between the stop
/// and the restart is fair game for anything else scanning that band - a live
/// Gate Connect install on the same machine, or another engine-starting test
/// binary under a runner that executes binaries in parallel (plain `cargo
/// test` runs them one at a time, but nothing pins the project to that).
/// Seeding from the ephemeral range instead keeps these tests out
/// of that shared namespace while still exercising the real reclaim path
/// (`bind_preferred`, including the `SO_REUSEADDR` rebind over the first run's
/// TIME_WAIT remnants).
fn seed_preferred_port() -> u16 {
    let probe = std::net::TcpListener::bind(("127.0.0.1", 0)).expect("seeding a preferred port");
    probe.local_addr().unwrap().port()
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
        oauth_token: String::new(), // legacy API-key path
        org_id: String::new(),      // no org on the legacy path
        domains: default_domains(),
        ca_cert_pem: ca_cert_pem.clone(),
        ca_key_pem: ca_key_pem.clone(),
        preferred_port: None,
        preferred_pac_port,
        preferred_relay_port: None,
        owner_uid: None,
        upstream_proxy: None,
    };

    // First run on a PAC port of our own choosing (see `seed_preferred_port`).
    let pac_port = seed_preferred_port();
    let engine = engine::start(config(Some(pac_port)), || {}).expect("first engine start");
    assert_eq!(
        engine.pac_port(),
        pac_port,
        "first run must take the seeded PAC port"
    );
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

    // Taken PAC port: fall back rather than failing the start. As above, the
    // blocker holds a port of its own rather than racing to reclaim the one
    // just freed.
    let blocker = std::net::TcpListener::bind(("127.0.0.1", 0)).expect("occupying a port");
    let taken = blocker.local_addr().unwrap().port();
    let engine = engine::start(config(Some(taken)), || {}).expect("fallback engine start");
    assert_ne!(
        engine.pac_port(),
        taken,
        "a taken preferred PAC port must fall back to another one"
    );
    engine.stop();
    drop(blocker);
}
