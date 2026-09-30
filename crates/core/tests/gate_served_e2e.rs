//! The relay's Gate models route, end to end: a tool whose config points at
//! `<relay>/__gate/t/<tool>/gate/v1` is served by Gate on the organization's
//! credits, and only for the models the user enabled for that tool.
//!
//! This is what replaced the proxy stamping a model header and the gateway
//! rewriting the body. So the assertions are about what reaches the gateway:
//! the tool's own request, untouched in its model, with nothing on it that
//! would make the gateway forward it to the tool's own provider - and nothing
//! at all when the model is not one the user enabled.
//!
//! Hermetic like `relay_e2e.rs` (throwaway CA, loopback mock gateway), plus a
//! temp app-support dir for the stored choice the route checks against. Its
//! own binary, since that dir is process-wide.

use std::sync::{Arc, Mutex};

use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::body::Incoming;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Request, Response};
use hyper_util::rt::TokioIo;
use rcgen::{BasicConstraints, CertificateParams, DnType, IsCa, KeyPair, KeyUsagePurpose};
use tokio::net::TcpListener;

use gate_connect_core::account::BillingMode;
use gate_connect_core::preferences::{self, ModelSource};
use gate_connect_core::proxy::default_domains;
use gate_connect_core::proxy::engine::{self, EngineConfig};

static SERIAL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

const LUNA: &str = "openai/gpt-5.6-luna";

#[derive(Clone)]
struct Captured {
    method: String,
    path: String,
    headers: Vec<(String, String)>,
    body: String,
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
                        let method = req.method().to_string();
                        let path = req.uri().path().to_string();
                        let headers = req
                            .headers()
                            .iter()
                            .map(|(k, v)| {
                                (k.as_str().to_string(), v.to_str().unwrap_or("").to_string())
                            })
                            .collect();
                        let body = req.into_body().collect().await.unwrap().to_bytes();
                        cap.lock().unwrap().push(Captured {
                            method,
                            path,
                            headers,
                            body: String::from_utf8_lossy(&body).into_owned(),
                        });
                        Ok::<_, std::convert::Infallible>(Response::new(Full::new(Bytes::from(
                            "{}",
                        ))))
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

fn mint_ca() -> (String, String) {
    let mut params = CertificateParams::new(Vec::<String>::new()).unwrap();
    params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    params.key_usages = vec![
        KeyUsagePurpose::KeyCertSign,
        KeyUsagePurpose::CrlSign,
        KeyUsagePurpose::DigitalSignature,
    ];
    params
        .distinguished_name
        .push(DnType::CommonName, "Gate Connect Test CA");
    let key = KeyPair::generate().unwrap();
    let cert = params.self_signed(&key).unwrap();
    (cert.pem(), key.serialize_pem())
}

/// A BYOK account: the mode in which nothing but the route asks Gate to serve.
fn boot_engine(gateway_base_url: String) -> engine::RunningEngine {
    let (ca_cert_pem, ca_key_pem) = mint_ca();
    engine::start(
        EngineConfig {
            gateway_base_url,
            api_key: "sk-gw-test".into(),
            oauth_token: String::new(),
            org_id: String::new(),
            billing_mode: BillingMode::Byok,
            domains: default_domains(),
            ca_cert_pem,
            ca_key_pem,
            preferred_port: None,
            preferred_pac_port: None,
            preferred_relay_port: None,
            owner_uid: None,
            upstream_proxy: None,
        },
        || {},
    )
    .expect("proxy engine should start")
}

struct TempHome(std::path::PathBuf);

impl TempHome {
    fn set() -> Self {
        let n = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("gate-served-e2e-{}-{n}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::env::set_var("GATE_CONNECT_TEST_HOME", &dir);
        preferences::reset_cache_for_tests();
        TempHome(dir)
    }
}

impl Drop for TempHome {
    fn drop(&mut self) {
        std::env::remove_var("GATE_CONNECT_TEST_HOME");
        preferences::reset_cache_for_tests();
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn codex_on(ids: &[&str]) {
    preferences::set_tool_model(
        "codex",
        ModelSource::Gate,
        ids.iter().map(|s| s.to_string()).collect(),
        true,
        vec![],
    )
    .unwrap();
}

fn url(engine: &engine::RunningEngine, path: &str) -> String {
    format!("http://127.0.0.1:{}{path}", engine.relay_port())
}

#[tokio::test]
async fn an_enabled_model_is_served_by_gate_even_under_byok() {
    let _s = SERIAL.lock().await;
    let _home = TempHome::set();
    codex_on(&[LUNA]);
    let gateway = start_mock_gateway().await;
    let engine = boot_engine(gateway.base_url.clone());

    let body = serde_json::json!({ "model": LUNA, "input": "hi" });
    let resp = reqwest::Client::new()
        .post(url(&engine, "/__gate/t/codex/gate/v1/responses"))
        // What Codex would carry with its ChatGPT login, and two headers a local
        // process could try to steer with. None of them may reach the gateway.
        .header("authorization", "Bearer chatgpt-oauth-token")
        .header("x-gate-upstream-url", "https://chatgpt.com/backend-api")
        .header("x-gate-model", "anthropic/claude-opus-5")
        .json(&body)
        .send()
        .await
        .unwrap();
    assert!(
        resp.status().is_success(),
        "relay returned {}",
        resp.status()
    );
    engine.stop();

    let reqs = gateway.captured.lock().unwrap().clone();
    assert_eq!(reqs.len(), 1);
    let r = &reqs[0];
    assert_eq!(r.method, "POST");
    assert_eq!(
        r.path, "/v1/responses",
        "the served wire format, on its own path"
    );
    assert_eq!(
        r.header("x-gate-upstream-url"),
        None,
        "no hint: Gate serves it"
    );
    assert_eq!(
        r.header("authorization"),
        None,
        "the tool's login is not Gate's to send"
    );
    assert_eq!(
        r.header("x-gate-model"),
        None,
        "the retired header is stripped"
    );
    assert_eq!(r.header("x-gate-api-key"), Some("sk-gw-test"));
    assert_eq!(
        r.header("x-gate-client"),
        Some("codex"),
        "named by the path marker"
    );
    let sent: serde_json::Value = serde_json::from_str(&r.body).unwrap();
    assert_eq!(
        sent["model"], LUNA,
        "the model the tool asked for, untouched"
    );
}

#[tokio::test]
async fn a_model_outside_the_set_is_refused_before_the_gateway() {
    let _s = SERIAL.lock().await;
    let _home = TempHome::set();
    codex_on(&[LUNA]);
    let gateway = start_mock_gateway().await;
    let engine = boot_engine(gateway.base_url.clone());

    let resp = reqwest::Client::new()
        .post(url(&engine, "/__gate/t/codex/gate/v1/responses"))
        .json(&serde_json::json!({ "model": "gpt-5.6-sol", "input": "hi" }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 400);
    let err: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(err["error"]["code"], "gate_model_not_enabled");
    let message = err["error"]["message"].as_str().unwrap();
    assert!(
        message.contains("gpt-5.6-sol") && message.contains(LUNA),
        "{message}"
    );

    // A tool that is not on Gate models at all, on the same route.
    let resp = reqwest::Client::new()
        .post(url(&engine, "/__gate/t/hermes/gate/v1/chat/completions"))
        .json(&serde_json::json!({ "model": LUNA, "messages": [] }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 400);
    let err: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(err["error"]["code"], "gate_models_off");

    engine.stop();
    assert!(
        gateway.captured.lock().unwrap().is_empty(),
        "nothing is billed for a refusal"
    );
}

#[tokio::test]
async fn the_route_serves_the_model_list_and_refuses_what_gate_cannot_answer() {
    let _s = SERIAL.lock().await;
    let _home = TempHome::set();
    codex_on(&[LUNA]);
    let gateway = start_mock_gateway().await;
    let engine = boot_engine(gateway.base_url.clone());
    let client = reqwest::Client::new();

    let models = client
        .get(url(&engine, "/__gate/t/codex/gate/v1/models"))
        .send()
        .await
        .unwrap();
    assert!(models.status().is_success());

    let not_served = client
        .post(url(&engine, "/__gate/t/codex/gate/v1/responses/compact"))
        .json(&serde_json::json!({ "model": LUNA }))
        .send()
        .await
        .unwrap();
    assert_eq!(not_served.status(), 404);

    let no_tool = client
        .post(url(&engine, "/gate/v1/responses"))
        .json(&serde_json::json!({ "model": LUNA }))
        .send()
        .await
        .unwrap();
    assert_eq!(
        no_tool.status(),
        400,
        "the set is looked up by tool, so one is required"
    );

    engine.stop();
    let reqs = gateway.captured.lock().unwrap().clone();
    assert_eq!(reqs.len(), 1, "only the model list went through");
    assert_eq!(reqs[0].method, "GET");
    assert_eq!(reqs[0].path, "/v1/models");
    assert_eq!(reqs[0].header("x-gate-upstream-url"), None);
}

/// Claude Code on Gate models: the served route drops the tool's own
/// `x-api-key` as well as its `Authorization`, and the provider pins a local
/// caller could use to steer which org account pays (review on #382).
#[tokio::test]
async fn the_route_drops_the_tools_own_credentials_and_provider_pins() {
    let _s = SERIAL.lock().await;
    let _home = TempHome::set();
    preferences::set_tool_model(
        "claude-code",
        ModelSource::Gate,
        vec!["anthropic/claude-opus-5".into()],
        true,
        vec![],
    )
    .unwrap();
    let gateway = start_mock_gateway().await;
    let engine = boot_engine(gateway.base_url.clone());

    let resp = reqwest::Client::new()
        .post(url(
            &engine,
            "/__gate/t/claude-code/gate/v1/messages?beta=true",
        ))
        .header("x-api-key", "sk-ant-api03-user-key")
        .header("authorization", "Bearer sk-ant-oat01-user-token")
        .header("x-gate-provider", "some-other-account")
        .json(&serde_json::json!({
            "model": "anthropic/claude-opus-5",
            "provider": { "order": ["some-other-account"] },
            "messages": [],
        }))
        .send()
        .await
        .unwrap();
    assert!(resp.status().is_success(), "{}", resp.status());
    engine.stop();

    let reqs = gateway.captured.lock().unwrap().clone();
    assert_eq!(reqs.len(), 1);
    let r = &reqs[0];
    assert_eq!(r.path, "/v1/messages");
    assert_eq!(r.header("x-api-key"), None);
    assert_eq!(r.header("authorization"), None);
    assert_eq!(r.header("x-gate-provider"), None);
    let sent: serde_json::Value = serde_json::from_str(&r.body).unwrap();
    assert!(sent.get("provider").is_none(), "{sent}");
    assert_eq!(sent["model"], "anthropic/claude-opus-5");
}

/// The route serves only a tool that supports Gate models, and only once this
/// install has accepted paid use; a set stored without either is refused
/// before the gateway (review on #382).
#[tokio::test]
async fn the_route_needs_a_supported_tool_and_the_paid_use_acknowledgement() {
    let _s = SERIAL.lock().await;
    let _home = TempHome::set();
    // Stored with no acknowledgement, as a hand edit or an older build could.
    preferences::set_tool_model("codex", ModelSource::Gate, vec![LUNA.into()], false, vec![])
        .unwrap();
    // And a set for a tool with no Gate models support at all.
    preferences::set_tool_model(
        "opencode",
        ModelSource::Gate,
        vec![LUNA.into()],
        true,
        vec![],
    )
    .unwrap();
    assert!(
        preferences::load().gate_model_paid_ack_unix.is_some(),
        "acked by the second"
    );
    let gateway = start_mock_gateway().await;
    let engine = boot_engine(gateway.base_url.clone());
    let client = reqwest::Client::new();

    let unsupported = client
        .post(url(&engine, "/__gate/t/opencode/gate/v1/chat/completions"))
        .json(&serde_json::json!({ "model": LUNA, "messages": [] }))
        .send()
        .await
        .unwrap();
    assert_eq!(unsupported.status(), 400);
    let err: serde_json::Value = unsupported.json().await.unwrap();
    assert_eq!(err["error"]["code"], "gate_models_off");

    // Clear the acknowledgement: now Codex's stored set is not servable either.
    let path = gate_connect_core::env::app_support_dir()
        .unwrap()
        .join("preferences.json");
    let mut v: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    v["gate_model_paid_ack_unix"] = serde_json::Value::Null;
    std::fs::write(&path, serde_json::to_vec_pretty(&v).unwrap()).unwrap();
    let unacked = client
        .get(url(&engine, "/__gate/t/codex/gate/v1/models"))
        .send()
        .await
        .unwrap();
    assert_eq!(unacked.status(), 400, "not even the model list");

    engine.stop();
    assert!(gateway.captured.lock().unwrap().is_empty());
}

/// Routing off: the route has no provider to fall back to, so it says so with
/// a 503 the SDKs will not retry.
#[tokio::test]
async fn the_route_refuses_without_retry_while_routing_is_off() {
    let _s = SERIAL.lock().await;
    let _home = TempHome::set();
    codex_on(&[LUNA]);
    let gateway = start_mock_gateway().await;
    let engine = boot_engine(gateway.base_url.clone());
    engine.set_intercept(false);

    let resp = reqwest::Client::new()
        .post(url(&engine, "/__gate/t/codex/gate/v1/responses"))
        .json(&serde_json::json!({ "model": LUNA, "input": "hi" }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 503);
    assert_eq!(
        resp.headers()
            .get("x-should-retry")
            .and_then(|v| v.to_str().ok()),
        Some("false")
    );
    let err: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(err["error"]["code"], "gate_not_routing");
    engine.stop();
    assert!(gateway.captured.lock().unwrap().is_empty());
}

/// The relay's ordinary catalog route strips a caller-set `x-gate-model` too:
/// an older gateway would still rewrite the model from it (review on #382).
#[tokio::test]
async fn the_catalog_route_strips_a_caller_set_model_header() {
    let _s = SERIAL.lock().await;
    let _home = TempHome::set();
    let gateway = start_mock_gateway().await;
    let engine = boot_engine(gateway.base_url.clone());

    let resp = reqwest::Client::new()
        .post(url(&engine, "/__gate/t/codex/openai/v1/responses"))
        .header("x-gate-model", "anthropic/claude-opus-5")
        .header("authorization", "Bearer sk-own")
        .json(&serde_json::json!({ "model": "gpt-5.6-sol", "input": "hi" }))
        .send()
        .await
        .unwrap();
    assert!(resp.status().is_success());
    engine.stop();

    let reqs = gateway.captured.lock().unwrap().clone();
    assert_eq!(reqs.len(), 1);
    assert_eq!(reqs[0].header("x-gate-model"), None);
    assert_eq!(
        reqs[0].header("x-gate-upstream-url"),
        Some("https://api.openai.com"),
        "an ordinary BYOK forward otherwise"
    );
}
