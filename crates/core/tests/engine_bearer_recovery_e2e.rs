//! The engine's pre-send wait, end to end: a request that would leave under
//! an expired bearer waits for the session re-check, and leaves under the
//! renewed one instead.
//!
//! Hermetic, on `proxy_e2e`'s harness: a throwaway CA, a loopback mock
//! gateway that records the headers it is sent, and an in-process `reqwest`
//! routed through the engine. The re-check is stood in for by the observer
//! seam: the engine asks it, the test pushes the renewed token into the
//! engine and ends the check, exactly as the desktop shell's
//! `recheck_gate_session` does with a `Recovered` verdict.
//!
//! The expired bearer carries no local stamp (nothing in this process minted
//! it), so it is judged on its own `exp`, which is set well past the fallback
//! margin.
use std::sync::{Arc, Mutex};

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use http_body_util::Empty;
use hyper::body::{Bytes, Incoming};
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Request, Response};
use hyper_util::rt::TokioIo;
use rcgen::{BasicConstraints, CertificateParams, DnType, IsCa, KeyPair, KeyUsagePurpose};
use tokio::net::TcpListener;

use gate_connect_core::proxy::default_domains;
use gate_connect_core::proxy::engine::{self, EngineConfig};

fn jwt_expiring_at(exp: i64) -> String {
    let payload = URL_SAFE_NO_PAD.encode(format!(r#"{{"sub":"u","exp":{exp}}}"#));
    format!("h.{payload}.s")
}

fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64
}

struct MockGateway {
    base_url: String,
    bearers: Arc<Mutex<Vec<String>>>,
}

async fn start_mock_gateway() -> MockGateway {
    let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let bearers: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let seen = Arc::clone(&bearers);
    tokio::spawn(async move {
        loop {
            let Ok((stream, _)) = listener.accept().await else {
                break;
            };
            let seen = Arc::clone(&seen);
            tokio::spawn(async move {
                let service = service_fn(move |req: Request<Incoming>| {
                    let seen = Arc::clone(&seen);
                    async move {
                        let bearer = req
                            .headers()
                            .get("x-gate-authorization")
                            .and_then(|v| v.to_str().ok())
                            .unwrap_or("")
                            .to_string();
                        seen.lock().unwrap().push(bearer);
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
        bearers,
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

#[tokio::test]
async fn an_expired_bearer_waits_for_the_renewed_one_before_the_request_leaves() {
    let gateway = start_mock_gateway().await;
    let stale = jwt_expiring_at(now_unix() - 3 * 3600);
    let fresh = jwt_expiring_at(now_unix() + 3600);

    // The observer seam: the engine asks for a re-check, and the test plays
    // the shell's part once it hears the ask.
    let (asked_tx, mut asked_rx) = tokio::sync::mpsc::unbounded_channel::<()>();
    gate_connect_core::proxy::set_gate_auth_observer(move || {
        let _ = asked_tx.send(());
        true
    });

    let (ca_cert_pem, ca_key_pem) = mint_ca();
    let engine = Arc::new(
        engine::start(
            EngineConfig {
                gateway_base_url: gateway.base_url.clone(),
                api_key: String::new(),
                oauth_token: stale.clone(),
                billing_mode: Default::default(),
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
        .expect("proxy engine should start"),
    );

    let client = reqwest::Client::builder()
        .proxy(reqwest::Proxy::all(format!("http://127.0.0.1:{}", engine.port())).unwrap())
        .add_root_certificate(reqwest::Certificate::from_pem(ca_cert_pem.as_bytes()).unwrap())
        .build()
        .unwrap();
    let request = tokio::spawn(async move {
        client
            .post("https://api.anthropic.com/v1/messages")
            .header("authorization", "Bearer app-token")
            .json(&serde_json::json!({ "model": "claude", "messages": [] }))
            .send()
            .await
    });

    // The request is held until the check answers: the engine asked, and the
    // gateway has seen nothing.
    tokio::time::timeout(std::time::Duration::from_secs(5), asked_rx.recv())
        .await
        .expect("the engine should ask for a re-check before sending an expired bearer")
        .expect("observer channel");
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    assert!(
        gateway.bearers.lock().unwrap().is_empty(),
        "nothing should leave under the expired bearer while the check runs"
    );

    // The shell's `Recovered` verdict: push the renewed token, end the check.
    engine.update_token(&fresh);
    gate_connect_core::proxy::gate_auth_check_finished();

    let resp = tokio::time::timeout(std::time::Duration::from_secs(10), request)
        .await
        .expect("the held request should leave once the token is renewed")
        .unwrap()
        .expect("request should reach the gateway through the proxy");
    assert!(
        resp.status().is_success(),
        "gateway returned {}",
        resp.status()
    );

    let bearers = gateway.bearers.lock().unwrap().clone();
    assert_eq!(
        bearers,
        vec![format!("Bearer {fresh}")],
        "exactly one request, under the renewed bearer"
    );
}
