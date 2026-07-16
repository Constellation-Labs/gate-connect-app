//! End-to-end test for the CLI reverse-proxy relay ([`proxy::relay`], hosted in
//! the engine). A plain-HTTP client - standing in for a CLI tool pointed at the
//! loopback base URL - sends an origin-form request with only the *non-secret*
//! `x-gate-upstream-url` hint and its own `Authorization`. The relay must inject
//! the live Gate credential and forward to the gateway with the path preserved,
//! while never seeing a credential in any config file.
//!
//! Fully hermetic: a throwaway CA (only the MITM half of the engine needs it -
//! the relay path is plaintext loopback), a loopback mock gateway, and an
//! in-process client. No OS trust store, no system proxy, no elevation.

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
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
    }
}

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

/// Mint a throwaway root CA. The relay path doesn't use it, but `engine::start`
/// builds its MITM half from a CA, so one must be supplied.
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

fn boot_engine(gateway_base_url: String, oauth_token: &str, org_id: &str) -> engine::RunningEngine {
    boot_engine_owned(gateway_base_url, oauth_token, org_id, None)
}

fn boot_engine_owned(
    gateway_base_url: String,
    oauth_token: &str,
    org_id: &str,
    owner_uid: Option<u32>,
) -> engine::RunningEngine {
    let (ca_cert_pem, ca_key_pem) = mint_ca();
    engine::start(
        EngineConfig {
            gateway_base_url,
            api_key: "sk-gw-test".into(),
            oauth_token: oauth_token.into(),
            org_id: org_id.into(),
            domains: default_domains(),
            ca_cert_pem,
            ca_key_pem,
            preferred_port: None,
            preferred_relay_port: None,
            owner_uid,
            #[cfg(any(target_os = "windows", target_os = "macos"))]
            upstream_proxy: None,
        },
        || {},
    )
    .expect("proxy engine should start")
}

/// A CLI tool pointed at the relay: injects the OAuth token, preserves the
/// path, passes the upstream hint and the tool's own credential through.
#[tokio::test]
async fn relay_injects_oauth_token_and_forwards_to_gateway() {
    let gateway = start_mock_gateway().await;
    let engine = boot_engine(
        gateway.base_url.clone(),
        "cognito-access-token",
        "org-uuid-1",
    );

    let client = reqwest::Client::builder().build().unwrap();
    let resp = client
        .post(format!(
            "http://127.0.0.1:{}/v1/messages",
            engine.relay_port()
        ))
        .header("authorization", "Bearer app-token")
        .header("x-gate-upstream-url", "https://api.anthropic.com")
        .json(&serde_json::json!({ "model": "claude", "messages": [] }))
        .send()
        .await
        .expect("relay request should succeed");
    assert!(
        resp.status().is_success(),
        "relay returned {}",
        resp.status()
    );

    engine.stop();

    let reqs = gateway.captured.lock().unwrap().clone();
    assert_eq!(
        reqs.len(),
        1,
        "gateway should have received exactly one request"
    );
    let r = &reqs[0];
    assert_eq!(r.method, "POST");
    assert_eq!(r.path, "/v1/messages", "the tool's path must be preserved");
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
        Some("https://api.anthropic.com")
    );
    assert_eq!(
        r.header("authorization"),
        Some("Bearer app-token"),
        "the tool's own credential must be forwarded untouched"
    );
}

/// With no OAuth token the relay falls back to the legacy `x-gate-api-key`.
#[tokio::test]
async fn relay_falls_back_to_api_key_when_no_token() {
    let gateway = start_mock_gateway().await;
    let engine = boot_engine(gateway.base_url.clone(), "", "");

    let client = reqwest::Client::builder().build().unwrap();
    let resp = client
        .post(format!(
            "http://127.0.0.1:{}/v1/messages",
            engine.relay_port()
        ))
        .header("x-gate-upstream-url", "https://api.anthropic.com")
        .json(&serde_json::json!({ "model": "claude", "messages": [] }))
        .send()
        .await
        .expect("relay request should succeed");
    assert!(resp.status().is_success());

    engine.stop();

    let reqs = gateway.captured.lock().unwrap().clone();
    assert_eq!(reqs.len(), 1);
    let r = &reqs[0];
    assert_eq!(r.header("x-gate-api-key"), Some("sk-gw-test"));
    assert_eq!(
        r.header("x-gate-authorization"),
        None,
        "no bearer when falling back to the API key"
    );
}

/// A caller that supplies its own `x-gate-api-key` keeps it: the relay forwards
/// that key untouched and injects nothing - not even the seeded OAuth token.
#[tokio::test]
async fn relay_respects_caller_supplied_gate_key() {
    let gateway = start_mock_gateway().await;
    // Seed an OAuth token + org, which would normally be injected as a bearer.
    let engine = boot_engine(
        gateway.base_url.clone(),
        "cognito-access-token",
        "org-uuid-1",
    );

    let client = reqwest::Client::builder().build().unwrap();
    let resp = client
        .post(format!(
            "http://127.0.0.1:{}/v1/messages",
            engine.relay_port()
        ))
        .header("x-gate-upstream-url", "https://api.anthropic.com")
        .header("x-gate-api-key", "sk-gw-caller")
        .json(&serde_json::json!({ "model": "claude", "messages": [] }))
        .send()
        .await
        .expect("relay request should succeed");
    assert!(resp.status().is_success());

    engine.stop();

    let reqs = gateway.captured.lock().unwrap().clone();
    assert_eq!(reqs.len(), 1);
    let r = &reqs[0];
    assert_eq!(
        r.header("x-gate-api-key"),
        Some("sk-gw-caller"),
        "the caller's own key must be forwarded untouched"
    );
    assert_eq!(
        r.header("x-gate-authorization"),
        None,
        "the seeded OAuth token must not be injected over a caller-supplied key"
    );
}

/// A refreshed token reaches the relay live, with no restart and no config
/// rewrite - the whole point of injecting per request.
#[tokio::test]
async fn relay_hot_swaps_a_refreshed_token() {
    let gateway = start_mock_gateway().await;
    let engine = boot_engine(gateway.base_url.clone(), "first-token", "org-uuid-1");

    engine.update_token("second-token");

    let client = reqwest::Client::builder().build().unwrap();
    let resp = client
        .post(format!(
            "http://127.0.0.1:{}/v1/messages",
            engine.relay_port()
        ))
        .header("x-gate-upstream-url", "https://api.anthropic.com")
        .json(&serde_json::json!({ "model": "claude", "messages": [] }))
        .send()
        .await
        .expect("relay request should succeed");
    assert!(resp.status().is_success());

    engine.stop();

    let reqs = gateway.captured.lock().unwrap().clone();
    assert_eq!(reqs.len(), 1);
    assert_eq!(
        reqs[0].header("x-gate-authorization"),
        Some("Bearer second-token")
    );
}

/// The relay refuses to forward to an upstream that isn't in the built-in
/// catalog, so a local process can't aim the gateway at an arbitrary host.
#[tokio::test]
async fn relay_rejects_unknown_upstream() {
    let gateway = start_mock_gateway().await;
    let engine = boot_engine(
        gateway.base_url.clone(),
        "cognito-access-token",
        "org-uuid-1",
    );

    let client = reqwest::Client::builder().build().unwrap();
    let resp = client
        .post(format!(
            "http://127.0.0.1:{}/v1/messages",
            engine.relay_port()
        ))
        .header("x-gate-upstream-url", "https://attacker.example")
        .json(&serde_json::json!({ "model": "claude", "messages": [] }))
        .send()
        .await
        .expect("relay request should return a response");
    assert_eq!(resp.status(), reqwest::StatusCode::FORBIDDEN);

    engine.stop();

    assert!(
        gateway.captured.lock().unwrap().is_empty(),
        "a rejected upstream must never reach the gateway"
    );
}

/// A non-owner peer can't spend the host credential: with an `owner_uid` that
/// can't match our connection, the relay drops the socket before serving, so
/// the client sees a closed connection and nothing reaches the gateway. UID
/// resolution is Linux-only, so the gate is only enforced (and tested) there.
#[cfg(target_os = "linux")]
#[tokio::test]
async fn relay_refuses_non_owner_peer() {
    let gateway = start_mock_gateway().await;
    // u32::MAX can never be our real UID, so `peer_uid_for` (our own loopback
    // connection) resolves to a different value and the peer is refused. An
    // unresolvable UID (None) also fails closed, so either way this is refused.
    let engine = boot_engine_owned(
        gateway.base_url.clone(),
        "cognito-access-token",
        "org-uuid-1",
        Some(u32::MAX),
    );

    let client = reqwest::Client::builder().build().unwrap();
    let result = client
        .post(format!(
            "http://127.0.0.1:{}/v1/messages",
            engine.relay_port()
        ))
        .header("x-gate-upstream-url", "https://api.anthropic.com")
        .json(&serde_json::json!({ "model": "claude", "messages": [] }))
        .send()
        .await;

    engine.stop();

    assert!(
        result.is_err(),
        "a non-owner peer must be refused, got {result:?}"
    );
    assert!(
        gateway.captured.lock().unwrap().is_empty(),
        "nothing may reach the gateway when the peer is refused"
    );
}
