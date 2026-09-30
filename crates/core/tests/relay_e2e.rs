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
use std::time::{Duration, Instant};

use bytes::Bytes;
use http_body_util::Full;
use hyper::body::Incoming;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Request, Response};
use hyper_util::rt::TokioIo;
use rcgen::{BasicConstraints, CertificateParams, DnType, IsCa, KeyPair, KeyUsagePurpose};
use tokio::net::TcpListener;

use gate_connect_core::account::BillingMode;
use gate_connect_core::proxy;
use gate_connect_core::proxy::default_domains;
use gate_connect_core::proxy::engine::{self, EngineConfig};

/// Serializes the tests that use the process-global `GATE_CONNECT_TEST_UPSTREAM`
/// seam. It names one upstream for the whole process, and each engine captures
/// its value at construction, so two of these running concurrently would point
/// one test's relay at the other's mock upstream.
static UPSTREAM_SEAM: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

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
    /// Credentials the gateway refuses with a 401, as (header, value) pairs:
    /// a bearer that expired while the app still thought it fresh, or a key
    /// that was revoked.
    refused: Arc<Mutex<Vec<(&'static str, String)>>>,
}

impl MockGateway {
    /// Refuse every request whose `header` carries exactly `value`.
    fn refuse(&self, header: &'static str, value: &str) {
        self.refused
            .lock()
            .unwrap()
            .push((header, value.to_string()));
    }
}

async fn start_mock_gateway() -> MockGateway {
    let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let captured: Arc<Mutex<Vec<Captured>>> = Arc::new(Mutex::new(Vec::new()));
    let refused: Arc<Mutex<Vec<(&'static str, String)>>> = Arc::new(Mutex::new(Vec::new()));

    let cap = Arc::clone(&captured);
    let rej = Arc::clone(&refused);
    tokio::spawn(async move {
        loop {
            let Ok((stream, _)) = listener.accept().await else {
                break;
            };
            let cap = Arc::clone(&cap);
            let rej = Arc::clone(&rej);
            tokio::spawn(async move {
                let service = service_fn(move |req: Request<Incoming>| {
                    let cap = Arc::clone(&cap);
                    let rej = Arc::clone(&rej);
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
                        let refused = rej.lock().unwrap().iter().any(|(header, value)| {
                            req.headers().get(*header).and_then(|v| v.to_str().ok())
                                == Some(value.as_str())
                        });
                        let resp = if refused {
                            Response::builder()
                                .status(401)
                                .body(Full::new(Bytes::from_static(
                                    br#"{"error":{"code":"invalid_gate_token"}}"#,
                                )))
                                .unwrap()
                        } else {
                            Response::new(Full::new(Bytes::new()))
                        };
                        Ok::<_, std::convert::Infallible>(resp)
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
        refused,
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
    boot_engine_full(
        gateway_base_url,
        oauth_token,
        org_id,
        owner_uid,
        BillingMode::Byok,
    )
}

fn boot_engine_full(
    gateway_base_url: String,
    oauth_token: &str,
    org_id: &str,
    owner_uid: Option<u32>,
    billing_mode: BillingMode,
) -> engine::RunningEngine {
    boot_engine_with(
        gateway_base_url,
        "sk-gw-test",
        oauth_token,
        org_id,
        owner_uid,
        billing_mode,
    )
}

fn boot_engine_with(
    gateway_base_url: String,
    api_key: &str,
    oauth_token: &str,
    org_id: &str,
    owner_uid: Option<u32>,
    billing_mode: BillingMode,
) -> engine::RunningEngine {
    let (ca_cert_pem, ca_key_pem) = mint_ca();
    engine::start(
        EngineConfig {
            gateway_base_url,
            api_key: api_key.into(),
            oauth_token: oauth_token.into(),
            org_id: org_id.into(),
            billing_mode,
            domains: default_domains(),
            ca_cert_pem,
            ca_key_pem,
            preferred_port: None,
            preferred_pac_port: None,
            preferred_relay_port: None,
            owner_uid,
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

/// The browser boundary: a DNS-rebound request arrives carrying the
/// attacker's hostname in `Host`, and a cross-site fetch carries the page's
/// `Origin`. Both must be refused before any credential is injected - CORS
/// does not stop a "simple" cross-origin POST from being *delivered*, so the
/// relay itself is the only thing standing between a web page and billed
/// inference on the owner's credential.
#[tokio::test]
async fn relay_refuses_rebound_host_and_cross_site_origin() {
    let gateway = start_mock_gateway().await;
    let engine = boot_engine(
        gateway.base_url.clone(),
        "cognito-access-token",
        "org-uuid-1",
    );
    let client = reqwest::Client::builder().build().unwrap();

    // DNS rebinding: the TCP connection reaches our loopback listener, but
    // the Host header still names the attacker's domain.
    let resp = client
        .post(format!(
            "http://127.0.0.1:{}/v1/messages",
            engine.relay_port()
        ))
        .header("host", "attacker.example")
        .json(&serde_json::json!({ "model": "claude", "messages": [] }))
        .send()
        .await
        .expect("relay request should return a response");
    assert_eq!(resp.status(), reqwest::StatusCode::FORBIDDEN);

    // Cross-site fetch: loopback Host (the browser resolved 127.0.0.1
    // directly) but a foreign Origin stamped by the page.
    let resp = client
        .post(format!(
            "http://127.0.0.1:{}/v1/messages",
            engine.relay_port()
        ))
        .header("origin", "https://attacker.example")
        .json(&serde_json::json!({ "model": "claude", "messages": [] }))
        .send()
        .await
        .expect("relay request should return a response");
    assert_eq!(resp.status(), reqwest::StatusCode::FORBIDDEN);

    // A loopback Origin (a local web UI talking to its own machine) stays
    // served - the boundary is site, not browser-ness.
    let resp = client
        .post(format!(
            "http://127.0.0.1:{}/v1/messages",
            engine.relay_port()
        ))
        .header("origin", "http://127.0.0.1:5173")
        .header("x-gate-upstream-url", "https://api.anthropic.com")
        .json(&serde_json::json!({ "model": "claude", "messages": [] }))
        .send()
        .await
        .expect("relay request should return a response");
    assert!(
        resp.status().is_success(),
        "loopback origin should be served, got {}",
        resp.status()
    );

    engine.stop();

    let reqs = gateway.captured.lock().unwrap().clone();
    assert_eq!(
        reqs.len(),
        1,
        "only the loopback-origin request may reach the gateway"
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

/// Flipping interception off (the Linux daemon with no GUI connected) turns
/// the relay into a direct forwarder: the same inference request that would
/// rewrite to the gateway goes to the real upstream instead, with every
/// Gate-internal header stripped and the tool's own credential intact, and
/// nothing reaches the gateway. Flipping it back on restores gateway rewriting
/// and credential injection. The mock upstream is admitted into the catalog
/// via the `GATE_CONNECT_TEST_UPSTREAM` seam - the built-in entries pin real
/// hosts, so a loopback mock could never pass validation otherwise.
#[tokio::test]
async fn relay_forwards_direct_when_not_intercepting() {
    let _seam = UPSTREAM_SEAM.lock().await;
    let gateway = start_mock_gateway().await;
    // A second capturing server, standing in for the tool's real upstream.
    let upstream = start_mock_gateway().await;
    std::env::set_var("GATE_CONNECT_TEST_UPSTREAM", &upstream.base_url);
    let engine = boot_engine(
        gateway.base_url.clone(),
        "cognito-access-token",
        "org-uuid-1",
    );

    engine.set_intercept(false);

    let client = reqwest::Client::builder().build().unwrap();
    let resp = client
        .post(format!(
            "http://127.0.0.1:{}/v1/messages",
            engine.relay_port()
        ))
        .header("authorization", "Bearer app-token")
        .header("x-gate-upstream-url", &upstream.base_url)
        .json(&serde_json::json!({ "model": "claude", "messages": [] }))
        .send()
        .await
        .expect("direct relay request should succeed");
    assert!(
        resp.status().is_success(),
        "direct hop returned {}",
        resp.status()
    );

    {
        let up = upstream.captured.lock().unwrap().clone();
        assert_eq!(up.len(), 1, "the request must reach the real upstream");
        let r = &up[0];
        assert_eq!(r.path, "/v1/messages", "the tool's path must be preserved");
        assert_eq!(
            r.header("authorization"),
            Some("Bearer app-token"),
            "the tool's own credential must be forwarded untouched"
        );
        assert_eq!(
            r.header("x-gate-authorization"),
            None,
            "no Gate credential may leave on a direct hop"
        );
        assert_eq!(r.header("x-gate-api-key"), None);
        assert_eq!(r.header("x-gate-org-id"), None);
        assert_eq!(
            r.header("x-gate-upstream-url"),
            None,
            "Gate-internal headers must be stripped on a direct hop"
        );
    }
    assert!(
        gateway.captured.lock().unwrap().is_empty(),
        "nothing may reach the gateway while not intercepting"
    );

    // Flip interception back on: the same request rewrites to the gateway
    // again, Gate credential injected.
    engine.set_intercept(true);
    let resp = client
        .post(format!(
            "http://127.0.0.1:{}/v1/messages",
            engine.relay_port()
        ))
        .header("authorization", "Bearer app-token")
        .header("x-gate-upstream-url", &upstream.base_url)
        .json(&serde_json::json!({ "model": "claude", "messages": [] }))
        .send()
        .await
        .expect("intercepted relay request should succeed");
    assert!(resp.status().is_success());

    engine.stop();

    let gw = gateway.captured.lock().unwrap().clone();
    assert_eq!(gw.len(), 1, "the second request must reach the gateway");
    assert_eq!(
        gw[0].header("x-gate-authorization"),
        Some("Bearer cognito-access-token")
    );
    assert_eq!(
        upstream.captured.lock().unwrap().len(),
        1,
        "the second request must not go direct"
    );
}

/// The relay's unauthenticated liveness path: answered by the relay itself, with
/// a 204 and no body, without the request ever reaching the gateway.
///
/// It proves that a relay of ours serves this port at all, and deliberately not
/// *who* is answering - anything that accepts on the port can return a 204. The
/// status probes ask `gate_connect_paths::RELAY_HEALTH_PATH` instead, where only
/// a process that can read the 0600 token can reply; `probe_relay_route` used to
/// ask here and was moved for exactly that reason.
#[tokio::test]
async fn relay_answers_its_own_health_path_without_calling_the_gateway() {
    let gateway = start_mock_gateway().await;
    let engine = boot_engine(
        gateway.base_url.clone(),
        "cognito-access-token",
        "org-uuid-1",
    );

    let client = reqwest::Client::builder().build().unwrap();
    let resp = client
        .get(format!(
            "http://127.0.0.1:{}{}",
            engine.relay_port(),
            gate_connect_core::proxy::RELAY_LIVENESS_PATH
        ))
        .send()
        .await
        .expect("health request should succeed");

    assert_eq!(
        resp.status(),
        reqwest::StatusCode::NO_CONTENT,
        "the health path must answer 204"
    );
    assert_eq!(
        resp.bytes().await.unwrap().len(),
        0,
        "the health path must carry no body"
    );

    engine.stop();

    assert!(
        gateway.captured.lock().unwrap().is_empty(),
        "a health check must never reach the gateway, let alone spend a token"
    );
}

/// A POST to the health path is a misconfigured tool, not a health check, so it
/// falls through to the catalog resolver and is refused there rather than being
/// answered 204.
#[tokio::test]
async fn relay_health_path_is_get_only() {
    let gateway = start_mock_gateway().await;
    let engine = boot_engine(
        gateway.base_url.clone(),
        "cognito-access-token",
        "org-uuid-1",
    );

    let client = reqwest::Client::builder().build().unwrap();
    let resp = client
        .post(format!(
            "http://127.0.0.1:{}{}",
            engine.relay_port(),
            gate_connect_core::proxy::RELAY_LIVENESS_PATH
        ))
        .send()
        .await
        .expect("request should complete");

    assert_ne!(
        resp.status(),
        reqwest::StatusCode::NO_CONTENT,
        "only GET is the health check"
    );

    engine.stop();
}

/// PAYG through the relay: the tool sends its own provider credential (which is
/// all a CLI tool has), and what reaches the gateway must carry neither that
/// credential nor an upstream hint. Those two absences are the entire contract -
/// with either one present the gateway routes BYOK, and with the credential
/// present but no hint it refuses the request outright.
#[tokio::test]
async fn relay_in_payg_sends_no_upstream_hint_and_no_client_credential() {
    let gateway = start_mock_gateway().await;
    let engine = boot_engine_full(gateway.base_url.clone(), "", "", None, BillingMode::Payg);

    let client = reqwest::Client::builder().build().unwrap();
    let resp = client
        .post(format!(
            "http://127.0.0.1:{}/anthropic/v1/messages",
            engine.relay_port()
        ))
        // What Claude Code / Cowork actually send, and what we must remove.
        .header("authorization", "Bearer sk-ant-oat01-app-token")
        .header("x-api-key", "sk-ant-api03-app-key")
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
    assert_eq!(reqs.len(), 1, "expected exactly one gateway request");
    let r = &reqs[0];
    // The slug is stripped, leaving the gateway-native path the reseller router
    // expects.
    assert_eq!(r.path, "/v1/messages");
    // We still say who the workspace is.
    assert_eq!(r.header("x-gate-api-key"), Some("sk-gw-test"));
    assert_eq!(
        r.header("x-gate-upstream-url"),
        None,
        "the hint's absence is what selects reseller routing"
    );
    assert_eq!(
        r.header("authorization"),
        None,
        "a provider token here is read as passthrough and forces BYOK"
    );
    assert_eq!(r.header("x-api-key"), None);
}

/// PAYG applies per domain. A consumer-chat surface is authenticated by a
/// session cookie and covered by the user's own subscription, so it keeps its
/// BYOK shape even while the account bills through Gate - stripping its
/// credential would break it and route nothing.
#[tokio::test]
async fn relay_in_payg_leaves_an_ineligible_domain_on_the_byok_shape() {
    let gateway = start_mock_gateway().await;
    let engine = boot_engine_full(gateway.base_url.clone(), "", "", None, BillingMode::Payg);

    let client = reqwest::Client::builder().build().unwrap();
    let resp = client
        .post(format!(
            "http://127.0.0.1:{}/claude-web/organizations/o/chat_conversations/c/completion",
            engine.relay_port()
        ))
        .header("authorization", "Bearer session-token")
        .json(&serde_json::json!({ "prompt": "hi" }))
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
    assert_eq!(reqs.len(), 1);
    let r = &reqs[0];
    assert_eq!(
        r.header("x-gate-upstream-url"),
        Some("https://claude.ai/api"),
        "an ineligible domain keeps routing BYOK"
    );
    assert_eq!(
        r.header("authorization"),
        Some("Bearer session-token"),
        "and keeps the credential that is the only thing authenticating it"
    );
}

/// A passthrough hop is untouched by the mode. Those are account/metadata paths
/// that go to the real upstream under the tool's own identity, so stripping the
/// credential there would simply 401 - and no Gate header may leak either way.
/// Uses the loopback test-upstream seam, since a real passthrough target is not
/// reachable from a test.
#[tokio::test]
async fn relay_in_payg_does_not_touch_a_passthrough_hop() {
    let _seam = UPSTREAM_SEAM.lock().await;
    let gateway = start_mock_gateway().await;
    let upstream = start_mock_gateway().await;
    std::env::set_var("GATE_CONNECT_TEST_UPSTREAM", &upstream.base_url);
    let engine = boot_engine_full(gateway.base_url.clone(), "", "", None, BillingMode::Payg);

    // The test-upstream entry rewrites `/v1/` only, so this path is classified
    // as passthrough and forwarded to the upstream itself.
    let client = reqwest::Client::builder().build().unwrap();
    let resp = client
        .post(format!(
            "http://127.0.0.1:{}/test-upstream/oauth/token",
            engine.relay_port()
        ))
        .header("authorization", "Bearer sk-ant-oat01-app-token")
        .json(&serde_json::json!({}))
        .send()
        .await
        .expect("passthrough request should succeed");
    assert!(
        resp.status().is_success(),
        "passthrough returned {}",
        resp.status()
    );

    engine.stop();
    std::env::remove_var("GATE_CONNECT_TEST_UPSTREAM");

    let up = upstream.captured.lock().unwrap().clone();
    assert_eq!(up.len(), 1, "the request must reach the real upstream");
    assert_eq!(
        up[0].header("authorization"),
        Some("Bearer sk-ant-oat01-app-token"),
        "PAYG must not strip the only credential a passthrough hop has"
    );
    assert_eq!(up[0].header("x-gate-api-key"), None);
    assert!(
        gateway.captured.lock().unwrap().is_empty(),
        "a non-inference path must never reach the gateway"
    );
}

/// Stands in for the desktop shell's observer, for every test in this binary
/// that drives a refusal of our bearer. The observer slot is process-global and
/// first registration wins, so one observer serves them all. It answers `true`
/// (a verdict will reach the token watch) and never releases the latch, which
/// leaves the binary in the "check in flight" state for good: every later
/// refusal waits on the watch, and each test pushes its own verdict into the
/// engine from a task, the way the shell's re-check thread does, rather than
/// from inside the observer. That also exercises the wait itself, instead of a
/// push that lands before `changed()` is polled. Releasing the latch would
/// start the 60s cooldown and make the next test's refusal pass straight
/// through, which is the ordering hazard this avoids.
fn hold_session_check_open() {
    proxy::set_gate_auth_observer(|| true);
}

/// Push `token` into the engine once the gateway has seen the first attempt,
/// from a task, as the shell's re-check thread does. Keyed on the attempt
/// rather than a delay: under a parallel test run an engine can take longer
/// to answer its first request than any fixed delay, and a verdict that lands
/// before the relay reads the watch is a different test (the request is then
/// refused locally, or sent under the new token first time). The engine is
/// held weakly so the test can unwrap and stop it once the request has been
/// answered; await the handle first.
fn push_verdict_after_first_attempt(
    engine: &Arc<engine::RunningEngine>,
    gateway: &MockGateway,
    token: &'static str,
) -> tokio::task::JoinHandle<()> {
    let weak = Arc::downgrade(engine);
    let captured = Arc::clone(&gateway.captured);
    tokio::spawn(async move {
        let deadline = Instant::now() + Duration::from_secs(5);
        while captured.lock().unwrap().is_empty() {
            assert!(
                Instant::now() < deadline,
                "the gateway never saw the first attempt"
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        if let Some(engine) = weak.upgrade() {
            engine.update_token(token);
        }
    })
}

/// A tool's inference request to the relay, with any extra headers.
async fn post_messages(port: u16, extra: &[(&str, &str)]) -> reqwest::Response {
    let client = reqwest::Client::builder().build().unwrap();
    let mut req = client
        .post(format!("http://127.0.0.1:{port}/v1/messages"))
        .header("x-gate-upstream-url", "https://api.anthropic.com");
    for (name, value) in extra {
        req = req.header(*name, *value);
    }
    req.json(&serde_json::json!({ "model": "claude", "messages": [] }))
        .send()
        .await
        .expect("the relay answers")
}

fn stop(engine: Arc<engine::RunningEngine>) {
    Arc::try_unwrap(engine)
        .ok()
        .expect("only the test holds the engine")
        .stop();
}

/// The gateway refuses the bearer the relay sent - an access token that expired
/// across a sleep, while the local clock still called it fresh. The relay hands
/// the refusal to the session observer, waits for the token the re-check
/// pushes, and retries once under it; the tool sees only the success.
#[tokio::test]
async fn relay_retries_a_refused_bearer_under_the_recovered_token() {
    hold_session_check_open();
    let gateway = start_mock_gateway().await;
    gateway.refuse("x-gate-authorization", "Bearer stale-token");
    let engine = Arc::new(boot_engine(
        gateway.base_url.clone(),
        "stale-token",
        "org-uuid-1",
    ));
    let push = push_verdict_after_first_attempt(&engine, &gateway, "fresh-token");

    let resp = post_messages(engine.relay_port(), &[]).await;
    assert!(
        resp.status().is_success(),
        "the tool must not see the refusal: got {}",
        resp.status()
    );

    push.await.unwrap();
    stop(engine);

    let reqs = gateway.captured.lock().unwrap().clone();
    assert_eq!(reqs.len(), 2, "one refused attempt, one retry");
    assert_eq!(
        reqs[0].header("x-gate-authorization"),
        Some("Bearer stale-token")
    );
    assert_eq!(
        reqs[1].header("x-gate-authorization"),
        Some("Bearer fresh-token")
    );
    assert_eq!(
        reqs[1].header("x-gate-org-id"),
        Some("org-uuid-1"),
        "the retry carries the org like the first attempt"
    );
    assert_eq!(
        reqs[1].header("x-gate-upstream-url"),
        Some("https://api.anthropic.com")
    );
}

/// The re-check finds the session dead and pushes the empty token. That ends
/// the wait at once, the 401 goes to the tool unchanged, and nothing is
/// retried.
#[tokio::test]
async fn relay_passes_the_401_through_when_the_session_is_dead() {
    hold_session_check_open();
    let gateway = start_mock_gateway().await;
    gateway.refuse("x-gate-authorization", "Bearer stale-token");
    let engine = Arc::new(boot_engine(
        gateway.base_url.clone(),
        "stale-token",
        "org-uuid-1",
    ));
    let push = push_verdict_after_first_attempt(&engine, &gateway, "");

    let started = Instant::now();
    let resp = post_messages(engine.relay_port(), &[]).await;
    assert_eq!(resp.status(), reqwest::StatusCode::UNAUTHORIZED);
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "the empty token ends the wait; the deadline must not be what ended it"
    );

    push.await.unwrap();
    stop(engine);

    let reqs = gateway.captured.lock().unwrap().clone();
    assert_eq!(reqs.len(), 1, "nothing is retried on a dead session");
}

/// The retry is once only: a gateway that refuses the recovered token too
/// leaves the tool with that 401 after exactly two attempts.
#[tokio::test]
async fn relay_retries_only_once() {
    hold_session_check_open();
    let gateway = start_mock_gateway().await;
    gateway.refuse("x-gate-authorization", "Bearer stale-token");
    gateway.refuse("x-gate-authorization", "Bearer fresh-token");
    let engine = Arc::new(boot_engine(
        gateway.base_url.clone(),
        "stale-token",
        "org-uuid-1",
    ));
    let push = push_verdict_after_first_attempt(&engine, &gateway, "fresh-token");

    let started = Instant::now();
    let resp = post_messages(engine.relay_port(), &[]).await;
    assert_eq!(resp.status(), reqwest::StatusCode::UNAUTHORIZED);
    // A second retry would find the watch still on the token it just sent and
    // wait out the deadline before giving up with the same two attempts
    // captured, so the count alone cannot tell once from a loop.
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "the refused retry is passed on at once, not after another wait"
    );

    push.await.unwrap();
    stop(engine);

    let reqs = gateway.captured.lock().unwrap().clone();
    assert_eq!(reqs.len(), 2, "one attempt and one retry, never a third");
    assert_eq!(
        reqs[1].header("x-gate-authorization"),
        Some("Bearer fresh-token")
    );
}

/// The retry rebuilds the whole rewrite, not just the bearer: under PAYG the
/// served shape - no upstream hint, none of the tool's own credential - has to
/// hold on the second attempt too, or the retry is billed to the org and
/// forwarded to the tool's provider at once.
#[tokio::test]
async fn relay_retries_a_refused_bearer_in_payg_keeping_the_served_shape() {
    hold_session_check_open();
    let gateway = start_mock_gateway().await;
    gateway.refuse("x-gate-authorization", "Bearer stale-token");
    let engine = Arc::new(boot_engine_full(
        gateway.base_url.clone(),
        "stale-token",
        "org-uuid-1",
        None,
        BillingMode::Payg,
    ));
    let push = push_verdict_after_first_attempt(&engine, &gateway, "fresh-token");

    let client = reqwest::Client::builder().build().unwrap();
    let resp = client
        .post(format!(
            "http://127.0.0.1:{}/anthropic/v1/messages",
            engine.relay_port()
        ))
        .header("authorization", "Bearer sk-ant-oat01-app-token")
        .header("x-api-key", "sk-ant-api03-app-key")
        .json(&serde_json::json!({ "model": "claude", "messages": [] }))
        .send()
        .await
        .expect("the relay answers");
    assert!(
        resp.status().is_success(),
        "the tool must not see the refusal: got {}",
        resp.status()
    );

    push.await.unwrap();
    stop(engine);

    let reqs = gateway.captured.lock().unwrap().clone();
    assert_eq!(reqs.len(), 2, "one refused attempt, one retry");
    assert_eq!(
        reqs[1].header("x-gate-authorization"),
        Some("Bearer fresh-token")
    );
    for (n, r) in reqs.iter().enumerate() {
        assert_eq!(r.path, "/v1/messages", "attempt {n}");
        assert_eq!(r.header("x-gate-upstream-url"), None, "attempt {n}");
        assert_eq!(r.header("authorization"), None, "attempt {n}");
        assert_eq!(r.header("x-api-key"), None, "attempt {n}");
    }
}

/// An app-support dir holding one stored choice, Codex on a Gate model, for
/// the whole binary. The override is process-global and the other tests here
/// boot engines concurrently, so it is set once and never reset rather than
/// swapped per test. Only the Gate-model test names Codex, so no other request
/// in this binary picks the choice up.
fn gate_model_home() {
    static HOME: std::sync::OnceLock<()> = std::sync::OnceLock::new();
    HOME.get_or_init(|| {
        let dir = std::env::temp_dir().join(format!("gc-relay-e2e-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("create temp app-support dir");
        gate_connect_core::env::set_app_support_dir_for_tests(Some(dir));
        gate_connect_core::preferences::reset_cache_for_tests();
        gate_connect_core::preferences::set_tool_model(
            "codex",
            gate_connect_core::preferences::ModelSource::Gate,
            vec!["openai/gpt-4o".into()],
            true,
        )
        .expect("store the choice");
    });
}

/// A Gate-model request goes to a different path from the one it arrived on,
/// so the retry's target has to come from the same rewrite as its headers. A
/// retry that rebuilt only the headers would resend `/codex/responses`, which
/// the gateway can only forward, without the upstream hint that says where.
#[tokio::test]
async fn relay_retries_a_refused_bearer_on_a_gate_model_keeping_the_served_path() {
    hold_session_check_open();
    gate_model_home();
    let gateway = start_mock_gateway().await;
    gateway.refuse("x-gate-authorization", "Bearer stale-token");
    let engine = Arc::new(boot_engine(
        gateway.base_url.clone(),
        "stale-token",
        "org-uuid-1",
    ));
    let push = push_verdict_after_first_attempt(&engine, &gateway, "fresh-token");

    let client = reqwest::Client::builder().build().unwrap();
    let resp = client
        .post(format!(
            "http://127.0.0.1:{}/__gate/t/codex/chatgpt/codex/responses",
            engine.relay_port()
        ))
        .header("authorization", "Bearer chatgpt-subscription-token")
        .json(&serde_json::json!({ "model": "gpt-5", "input": [] }))
        .send()
        .await
        .expect("the relay answers");
    assert!(
        resp.status().is_success(),
        "the tool must not see the refusal: got {}",
        resp.status()
    );

    push.await.unwrap();
    stop(engine);

    let reqs = gateway.captured.lock().unwrap().clone();
    assert_eq!(reqs.len(), 2, "one refused attempt, one retry");
    assert_eq!(
        reqs[1].header("x-gate-authorization"),
        Some("Bearer fresh-token")
    );
    for (n, r) in reqs.iter().enumerate() {
        assert_eq!(r.path, "/v1/responses", "attempt {n}");
        assert_eq!(
            r.header("x-gate-model"),
            Some("openai/gpt-4o"),
            "attempt {n}"
        );
        assert_eq!(r.header("x-gate-upstream-url"), None, "attempt {n}");
        assert_eq!(r.header("authorization"), None, "attempt {n}");
    }
}

/// A refused legacy key is a different problem with a different fix, and not
/// ours to recover: the 401 goes straight to the tool, with no wait and no
/// retry.
#[tokio::test]
async fn relay_does_not_retry_a_refused_legacy_key() {
    hold_session_check_open();
    let gateway = start_mock_gateway().await;
    gateway.refuse("x-gate-api-key", "sk-gw-test");
    let engine = boot_engine(gateway.base_url.clone(), "", "");

    let started = Instant::now();
    let resp = post_messages(engine.relay_port(), &[]).await;
    assert_eq!(resp.status(), reqwest::StatusCode::UNAUTHORIZED);
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "a refused key is passed on at once; there is no verdict to wait for"
    );

    engine.stop();

    let reqs = gateway.captured.lock().unwrap().clone();
    assert_eq!(reqs.len(), 1, "a refused key is not retried");
    assert_eq!(reqs[0].header("x-gate-api-key"), Some("sk-gw-test"));
}

/// A caller that brings its own Gate key is served under it even while a
/// session is live, and a refusal of that key is the caller's, not ours: it is
/// passed on at once, with no re-check and no retry. This is the case the
/// "only retry our own bearer" rule exists for; the legacy-key test above
/// cannot reach it, because with no token the refusal is settled before the
/// rule is consulted.
#[tokio::test]
async fn relay_does_not_retry_a_refused_caller_key_while_signed_in() {
    hold_session_check_open();
    let gateway = start_mock_gateway().await;
    gateway.refuse("x-gate-api-key", "sk-gw-caller");
    let engine = boot_engine(gateway.base_url.clone(), "live-token", "org-uuid-1");

    let started = Instant::now();
    let resp = post_messages(engine.relay_port(), &[("x-gate-api-key", "sk-gw-caller")]).await;
    assert_eq!(resp.status(), reqwest::StatusCode::UNAUTHORIZED);
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "a refusal of the caller's own key has no verdict of ours to wait for"
    );

    engine.stop();

    let reqs = gateway.captured.lock().unwrap().clone();
    assert_eq!(reqs.len(), 1, "a caller's refused key is not retried");
    assert_eq!(reqs[0].header("x-gate-api-key"), Some("sk-gw-caller"));
    assert_eq!(
        reqs[0].header("x-gate-authorization"),
        None,
        "nothing of ours goes on a caller-keyed request"
    );
}

/// An OAuth account whose session is dead has no key to fall back to. The relay
/// refuses the request itself with the same typed body the engine sends, and
/// nothing reaches the gateway - which would otherwise have answered with a
/// complaint about a missing API key, a credential this account never had.
#[tokio::test]
async fn relay_refuses_locally_when_signed_out() {
    let gateway = start_mock_gateway().await;
    let engine = boot_engine_with(
        gateway.base_url.clone(),
        "",
        "",
        "",
        None,
        BillingMode::Byok,
    );

    let resp = post_messages(engine.relay_port(), &[]).await;
    assert_eq!(resp.status(), reqwest::StatusCode::UNAUTHORIZED);
    assert_eq!(
        resp.headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok()),
        Some("application/json"),
        "the one relay error a tool is meant to parse is typed"
    );
    assert!(
        resp.headers()
            .get("www-authenticate")
            .is_some_and(|v| v.as_bytes().starts_with(b"Bearer")),
        "a 401 names its scheme"
    );
    assert_eq!(
        resp.headers()
            .get("x-content-type-options")
            .and_then(|v| v.to_str().ok()),
        Some("nosniff")
    );
    let body = resp.text().await.unwrap();
    assert!(
        body.contains("gate_signed_out"),
        "typed for a client log, like the engine's: {body}"
    );
    assert!(
        body.contains("sign in"),
        "the refusal names the fix: {body}"
    );

    engine.stop();

    assert!(
        gateway.captured.lock().unwrap().is_empty(),
        "nothing goes out bare"
    );
}

/// The signed-out refusal is about the app's own credential. A caller that
/// brings its own Gate key is served under it, session or no session, as the
/// shared injection rule says.
#[tokio::test]
async fn relay_serves_a_caller_supplied_key_while_signed_out() {
    let gateway = start_mock_gateway().await;
    let engine = boot_engine_with(
        gateway.base_url.clone(),
        "",
        "",
        "",
        None,
        BillingMode::Byok,
    );

    let resp = post_messages(engine.relay_port(), &[("x-gate-api-key", "sk-gw-caller")]).await;
    assert!(resp.status().is_success(), "got {}", resp.status());

    engine.stop();

    let reqs = gateway.captured.lock().unwrap().clone();
    assert_eq!(reqs.len(), 1);
    assert_eq!(reqs[0].header("x-gate-api-key"), Some("sk-gw-caller"));
    assert_eq!(reqs[0].header("x-gate-authorization"), None);
}
