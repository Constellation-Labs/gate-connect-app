//! Live replay of a real tool request through the Gate models route, against a
//! real gateway. Ignored by default: it needs a gateway, a key and a captured
//! request, and it spends credits.
//!
//! For each model it sends the captured request twice: straight to the
//! gateway as the tool sent it (direct), and through the relay's Gate models
//! route (through Gate Connect). It reports both, and fails if Gate Connect
//! changed the outcome: a different status, or an answer on one side and none
//! on the other. A model that fails both ways is recorded, not failed on:
//! Gate Connect forwards faithfully and does not patch a model and app
//! incompatibility (review on #400). Used for docs/model-app-compatibility.md.
//!
//! ```text
//! GATE_LIVE_GATEWAY=https://gateway-staging.constellationgate.ai \
//! GATE_LIVE_KEY_FILE=~/.gate-connect-dev/secrets/<gateway-api-key file> \
//! GATE_LIVE_BODY=/path/to/captured-claude-code-request.json \
//! GATE_LIVE_MODELS=meta-llama/muse-spark-1-2,openai/gpt-6-luna \
//! cargo test -p gate-connect-core --test live_tool_schema_replay -- --ignored --nocapture
//! ```
//!
//! The captured request is not checked in: it carries a whole system prompt
//! from the machine it was captured on. Capture one by pointing the tool at a
//! local stub that saves the body, as described in the doc above.

use gate_connect_core::account::BillingMode;
use gate_connect_core::preferences::{self, ModelSource};
use gate_connect_core::proxy::default_domains;
use gate_connect_core::proxy::engine::{self, EngineConfig};

fn env(name: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| panic!("{name} is required, see the module doc"))
}

fn mint_ca() -> (String, String) {
    let mut params = rcgen::CertificateParams::new(Vec::<String>::new()).unwrap();
    params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
    params.key_usages = vec![
        rcgen::KeyUsagePurpose::KeyCertSign,
        rcgen::KeyUsagePurpose::CrlSign,
        rcgen::KeyUsagePurpose::DigitalSignature,
    ];
    params
        .distinguished_name
        .push(rcgen::DnType::CommonName, "gate replay CA");
    let key = rcgen::KeyPair::generate().unwrap();
    let cert = params.self_signed(&key).unwrap();
    (cert.pem(), key.serialize_pem())
}

/// Status, and whether the answer had any content.
async fn send(
    client: &reqwest::Client,
    url: &str,
    key: Option<&str>,
    body: &serde_json::Value,
) -> (u16, bool, String) {
    let mut req = client
        .post(url)
        .header("anthropic-version", "2023-06-01")
        .json(body);
    if let Some(key) = key {
        req = req.header("x-gate-api-key", key);
    }
    let resp = req.send().await.expect("request");
    let status = resp.status().as_u16();
    let text = resp.text().await.unwrap_or_default();
    let v: serde_json::Value = serde_json::from_str(&text).unwrap_or_default();
    let content = v["content"].as_array().is_some_and(|c| !c.is_empty());
    let error = v["error"]["message"]
        .as_str()
        .map(|m| m.chars().take(120).collect())
        .unwrap_or_default();
    (status, content, error)
}

#[tokio::test]
#[ignore = "live: needs a gateway, a key and a captured request"]
async fn replay_a_captured_request_before_and_after_the_gate_models_route() {
    let gateway = env("GATE_LIVE_GATEWAY");
    let key_file = env("GATE_LIVE_KEY_FILE");
    let key_file = key_file.replacen('~', &std::env::var("HOME").unwrap(), 1);
    let key = std::fs::read_to_string(&key_file)
        .expect("key file")
        .trim()
        .to_string();
    let captured: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(env("GATE_LIVE_BODY")).unwrap()).unwrap();
    let models: Vec<String> = env("GATE_LIVE_MODELS")
        .split(',')
        .map(str::to_string)
        .collect();

    let home = std::env::temp_dir().join(format!("gate-live-replay-{}", std::process::id()));
    std::fs::create_dir_all(&home).unwrap();
    std::env::set_var("GATE_CONNECT_TEST_HOME", &home);
    preferences::reset_cache_for_tests();
    preferences::set_tool_model(
        "claude-code",
        ModelSource::Gate,
        models.clone(),
        true,
        vec![],
    )
    .unwrap();

    let (ca_cert_pem, ca_key_pem) = mint_ca();
    let engine = engine::start(
        EngineConfig {
            gateway_base_url: gateway.clone(),
            api_key: key.clone(),
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
    .expect("engine");
    let relay = format!(
        "http://127.0.0.1:{}/__gate/t/claude-code/gate/v1/messages?beta=true",
        engine.relay_port()
    );
    let direct = format!("{}/v1/messages", gateway.trim_end_matches('/'));
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(180))
        .no_proxy()
        .build()
        .unwrap();

    let mut changed = Vec::new();
    for model in &models {
        let mut body = captured.clone();
        body["model"] = model.as_str().into();
        body["stream"] = false.into();
        body["max_tokens"] = 2000.into();
        body["messages"] = serde_json::json!([
            {"role": "user", "content": "Use the Bash tool to run: echo hi"}
        ]);
        let direct_result = send(&client, &direct, Some(&key), &body).await;
        let through = send(&client, &relay, None, &body).await;
        println!("{model}\n  direct:               {direct_result:?}\n  through Gate Connect: {through:?}");
        if (direct_result.0, direct_result.1) != (through.0, through.1) {
            changed.push(model.clone());
        }
    }
    engine.stop();
    let _ = std::fs::remove_dir_all(&home);
    assert!(
        changed.is_empty(),
        "Gate Connect changed the outcome for: {changed:?}"
    );
}
