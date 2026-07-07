//! End-to-end test for the built-in MITM proxy engine: a real HTTPS client
//! routed through the loopback engine must have its inference request
//! rewritten to the Gate gateway with the `X-Gate-Api-Key` /
//! `X-Gate-Upstream-Url` headers injected, while its own credential is left
//! intact.
//!
//! This drives `proxy::engine::start` directly - the same engine the CLI's
//! `proxy enable` and the menubar app boot - so it exercises the only path in
//! the app that emits real gateway-bound traffic. It is fully hermetic: a
//! throwaway CA, a loopback mock gateway, and an in-process client. Nothing
//! touches the OS trust store or system proxy, so no elevation is needed and
//! it runs identically on macOS, Windows, and Linux.

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
    // 1. Mock gateway on loopback. The rewritten request must land here.
    let gateway = start_mock_gateway().await;

    // 2. Boot the real engine. Every catalog domain ships opt-in
    //    (`enabled:false`), so flip Anthropic on so api.anthropic.com /v1/ is
    //    intercepted and rewritten.
    let (ca_cert_pem, ca_key_pem) = mint_ca();
    let domains = default_domains()
        .into_iter()
        .map(|mut d| {
            if d.slug == "anthropic" {
                d.enabled = true;
            }
            d
        })
        .collect();
    let engine = engine::start(
        EngineConfig {
            gateway_base_url: gateway.base_url.clone(), // http://127.0.0.1:<port>
            api_key: "sk-gw-test".into(),
            domains,
            ca_cert_pem: ca_cert_pem.clone(),
            ca_key_pem,
            preferred_port: None,
            owner_uid: None,
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

#[tokio::test]
async fn proxy_rewrites_openrouter_request_to_gateway() {
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
            domains,
            ca_cert_pem: ca_cert_pem.clone(),
            ca_key_pem,
            preferred_port: None,
            owner_uid: None,
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
    //    nests its API under /api/v1/), Gate headers injected, and the client's
    //    own bearer forwarded untouched.
    let reqs = gateway.captured.lock().unwrap().clone();
    assert_eq!(
        reqs.len(),
        1,
        "gateway should have received exactly one request"
    );
    let r = &reqs[0];
    assert_eq!(r.method, "POST");
    assert_eq!(r.path, "/api/v1/chat/completions");
    assert_eq!(r.header("x-gate-api-key"), Some("sk-gw-test"));
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
