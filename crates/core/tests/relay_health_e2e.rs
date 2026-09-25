//! The relay's health endpoint, against a real engine rather than a stub.
//!
//! Everything else that asserts on interception drives a test double, so a
//! relay that reported a constant would pass all of it. This boots the engine,
//! reads the report through the production path (`proxy::relay_report`), and
//! flips the engine's own intercept flag underneath it.
//!
//! Its own test binary because it sets `GATE_CONNECT_TEST_HOME`, which is
//! process-global: cargo runs test binaries one at a time, so a single test per
//! binary cannot race a sibling that reads the same paths.

use rcgen::{BasicConstraints, CertificateParams, DnType, IsCa, KeyPair, KeyUsagePurpose};

use gate_connect_core::proxy::default_domains;
use gate_connect_core::proxy::engine::{self, EngineConfig};

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

/// A relay proves it is ours and reports what it is doing, and the report
/// follows the engine rather than being a constant.
///
/// The second half is the point: a relay that always claimed to be
/// intercepting would satisfy every stub-driven test, and would put a green
/// pill over a parked engine forwarding traffic straight past Gate.
#[test]
fn relay_health_proves_itself_and_tracks_the_intercept_flag() {
    let home = std::env::temp_dir().join(format!(
        "gate-connect-relay-health-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&home).unwrap();
    // SAFETY: one test in this binary, and cargo runs binaries sequentially.
    std::env::set_var("GATE_CONNECT_TEST_HOME", &home);

    let (ca_cert_pem, ca_key_pem) = mint_ca();
    let engine = engine::start(
        EngineConfig {
            gateway_base_url: "https://gateway.example.com".into(),
            billing_mode: Default::default(),
            api_key: "sk-gw-test".into(),
            oauth_token: String::new(),
            org_id: String::new(),
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
    .expect("proxy engine should start");

    // The production reader finds the relay through the persisted port, the way
    // it does on a real install; the engine binds an ephemeral one under test.
    let proxy_dir = gate_connect_core::env::app_support_dir()
        .unwrap()
        .join("proxy");
    std::fs::create_dir_all(&proxy_dir).unwrap();
    std::fs::write(
        proxy_dir.join("relay-port"),
        engine.relay_port().to_string(),
    )
    .unwrap();

    let routing = gate_connect_core::proxy::relay_report()
        .expect("a running relay must prove itself to the production reader");
    assert!(
        routing.intercepting,
        "a routing relay must report that it is intercepting"
    );

    engine.set_intercept(false);

    let parked = gate_connect_core::proxy::relay_report()
        .expect("a parked relay still proves itself - it is still ours");
    assert!(
        !parked.intercepting,
        "a parked relay must report that it is not intercepting, or its tools \
         read Connected over traffic going direct"
    );

    engine_path_proof_keeps_the_connection(engine.relay_port());

    engine.stop();
    let _ = std::fs::remove_dir_all(&home);
}

/// What the forwarder relies on before it hands the engine a relay request:
/// the engine's relay answers the engine-only proof, and keeps the connection
/// open for the request that follows on it. If it closed after the proof, the
/// forwarder would splice every tool request onto a dead socket.
fn engine_path_proof_keeps_the_connection(port: u16) {
    use std::io::Write;

    let token = std::fs::read_to_string(
        gate_connect_core::env::app_support_dir()
            .unwrap()
            .join("proxy")
            .join("forwarder.token"),
    )
    .expect("the reports above minted the token");
    let token = token.trim();
    let challenge = "0123456789abcdef";
    let path = gate_connect_paths::RELAY_ENGINE_HEALTH_PATH;

    let mut sock = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
    sock.set_read_timeout(Some(std::time::Duration::from_secs(5)))
        .unwrap();
    sock.write_all(
        format!(
            "GET {path} HTTP/1.1\r\nHost: 127.0.0.1\r\n{}: {challenge}\r\n\r\n",
            gate_connect_paths::FORWARDER_CHALLENGE_HEADER
        )
        .as_bytes(),
    )
    .unwrap();
    let head = read_head(&mut sock);
    assert!(head.starts_with("HTTP/1.1 204"), "{head}");
    let expected = gate_connect_paths::forwarder_proof(token, path, challenge);
    assert!(
        head.to_ascii_lowercase().contains(&expected),
        "the engine must answer the engine-only proof: {head}"
    );

    sock.write_all(
        format!(
            "GET {} HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n",
            gate_connect_core::proxy::RELAY_LIVENESS_PATH
        )
        .as_bytes(),
    )
    .unwrap();
    let second = read_head(&mut sock);
    assert!(
        second.starts_with("HTTP/1.1 204"),
        "a second request on the proved connection must be served: {second}"
    );
}

fn read_head(sock: &mut std::net::TcpStream) -> String {
    use std::io::Read;
    let mut head = Vec::new();
    let mut byte = [0u8; 1];
    while !head.ends_with(b"\r\n\r\n") {
        match sock.read(&mut byte) {
            Ok(1) => head.push(byte[0]),
            _ => break,
        }
    }
    String::from_utf8_lossy(&head).to_string()
}
