use super::*;

fn table(origin_port: u16) -> Vec<Upstream> {
    vec![
        Upstream::parse("anthropic", &format!("http://127.0.0.1:{origin_port}")).unwrap(),
        Upstream::parse("claude-web", &format!("http://127.0.0.1:{origin_port}/api")).unwrap(),
    ]
}

fn hdrs(list: &[(&str, &str)]) -> Vec<(String, Vec<u8>)> {
    list.iter()
        .map(|(n, v)| ((*n).to_string(), v.as_bytes().to_vec()))
        .collect()
}

fn plan11(target: &str, headers: &[(String, Vec<u8>)], t: &[Upstream]) -> Result<Plan, Refusal> {
    plan("POST", target, 1, headers, t)
}

fn head_text(plan: &Plan) -> String {
    String::from_utf8(plan.head.clone()).unwrap()
}

#[test]
fn the_catalog_urls_parse() {
    let u = Upstream::parse("claude-web", "https://claude.ai/api").unwrap();
    assert_eq!(
        (u.tls, u.host.as_str(), u.port, u.base_path.as_str()),
        (true, "claude.ai", 443, "/api")
    );
    assert_eq!(u.host_header(), "claude.ai");
    let u = Upstream::parse("x", "http://127.0.0.1:8080").unwrap();
    assert_eq!((u.tls, u.port, u.base_path.as_str()), (false, 8080, ""));
    assert_eq!(u.host_header(), "127.0.0.1:8080");
    // Every entry has to parse, or its tools lose the direct path. Compared
    // against the same list `upstreams` reads, so the e2e's extra mock entry
    // counts on both sides.
    assert_eq!(
        upstreams().len(),
        gate_connect_paths::relay_upstreams().len()
    );
}

/// The shape every config written today sends: the tool marker, then the
/// slug, then the client's own path.
#[test]
fn routes_a_marked_request_by_its_slug() {
    let t = table(9);
    let plan = plan11(
        "/__gate/t/opencode/claude-web/v1/messages?beta=true",
        &hdrs(&[("Host", "127.0.0.1:47111"), ("Content-Length", "2")]),
        &t,
    )
    .unwrap();
    assert_eq!(plan.upstream, 1);
    let head = head_text(&plan);
    assert!(
        head.starts_with("POST /api/v1/messages?beta=true HTTP/1.1\r\nHost: 127.0.0.1:9\r\n"),
        "{head}"
    );
    assert_eq!(plan.body, Framing::Length(2));
    assert!(head.contains("Content-Length: 2\r\n"), "{head}");
    assert!(head.ends_with("Connection: close\r\n\r\n"), "{head}");
    assert_eq!(head.matches("Host:").count(), 1, "{head}");
}

#[test]
fn routes_an_unmarked_request_and_a_legacy_header() {
    let t = table(9);
    let plan1 = plan("GET", "/anthropic", 1, &[], &t).unwrap();
    assert!(head_text(&plan1).starts_with("GET / HTTP/1.1\r\n"));

    let legacy = plan11(
        "/v1/messages",
        &hdrs(&[("x-gate-upstream-url", "http://127.0.0.1:9/api")]),
        &t,
    )
    .unwrap();
    assert_eq!(legacy.upstream, 1);
    assert!(head_text(&legacy).starts_with("POST /api/v1/messages HTTP/1.1\r\n"));
}

/// The property that keeps the listener from being an open proxy.
#[test]
fn refuses_anything_outside_the_catalog() {
    let t = table(9);
    assert_eq!(plan11("/evil.example/x", &[], &t).unwrap_err().status, 400);
    assert_eq!(
        plan11(
            "/x",
            &hdrs(&[("x-gate-upstream-url", "https://evil.example")]),
            &t
        )
        .unwrap_err()
        .status,
        403
    );
    assert_eq!(
        plan11("http://evil.example/x", &[], &t).unwrap_err().status,
        400
    );
    // The engine-only proof path is never something this listener answers.
    assert_eq!(
        plan("GET", RELAY_ENGINE_HEALTH_PATH, 1, &[], &t)
            .unwrap_err()
            .status,
        400
    );
}

#[test]
fn refuses_a_dot_segment() {
    let t = table(9);
    for target in [
        "/anthropic/v1/../../x",
        "/anthropic/%2e%2e/x",
        // A provider behind a URL parser would read a backslash as `/`.
        "/anthropic/v1/..\\..\\x",
    ] {
        assert_eq!(plan11(target, &[], &t).unwrap_err().status, 400, "{target}");
    }
}

/// The browser boundary, from the same definition the engine's relay uses,
/// including a header that is present but not readable.
#[test]
fn refuses_a_browser() {
    let t = table(9);
    let rebound = hdrs(&[("Host", "attacker.example")]);
    assert_eq!(
        plan11("/anthropic/x", &rebound, &t).unwrap_err().status,
        403
    );
    let cross_site = hdrs(&[
        ("Host", "127.0.0.1"),
        ("Origin", "https://attacker.example"),
    ]);
    assert_eq!(
        plan11("/anthropic/x", &cross_site, &t).unwrap_err().status,
        403
    );
    let unreadable = vec![("Origin".to_string(), vec![0xff, 0xfe])];
    assert_eq!(
        plan11("/anthropic/x", &unreadable, &t).unwrap_err().status,
        403
    );
    let unreadable = vec![("Host".to_string(), vec![0xff])];
    assert_eq!(
        plan11("/anthropic/x", &unreadable, &t).unwrap_err().status,
        403
    );
}

/// Nothing Gate-internal reaches a provider, and nothing that addressed
/// this hop does either. The tool's own credential does: it is the only
/// credential on this path.
#[test]
fn strips_gate_and_hop_by_hop_headers_and_keeps_the_tools_credential() {
    let t = table(9);
    let plan = plan11(
        "/anthropic/v1/messages",
        &hdrs(&[
            ("Host", "localhost"),
            ("Authorization", "Bearer sk-own"),
            ("x-api-key", "own-key"),
            ("X-Gate-Authorization", "Bearer gate"),
            ("x-gate-api-key", "gate-key"),
            ("x-gate-something-new", "1"),
            ("Connection", "keep-alive, X-Custom-Hop"),
            ("X-Custom-Hop", "1"),
            ("Keep-Alive", "timeout=5"),
            ("Proxy-Authorization", "Basic Z2F0ZQ=="),
            ("Upgrade", "websocket"),
            ("Content-Length", "0"),
        ]),
        &t,
    )
    .unwrap();
    let head = head_text(&plan).to_ascii_lowercase();
    assert!(head.contains("authorization: bearer sk-own\r\n"), "{head}");
    assert!(head.contains("x-api-key: own-key\r\n"), "{head}");
    for gone in [
        "x-gate-",
        "x-custom-hop",
        "keep-alive",
        "proxy-authorization",
        "upgrade",
        "host: localhost",
    ] {
        assert!(!head.contains(gone), "{gone} survived: {head}");
    }
    assert_eq!(head.matches("connection:").count(), 1, "{head}");
}

/// `Transfer-Encoding` and `Content-Length` together is the classic
/// smuggling shape; the length goes so the next hop cannot read it.
#[test]
fn chunked_wins_over_a_length() {
    let t = table(9);
    let plan = plan11(
        "/anthropic/v1/messages",
        &hdrs(&[("Transfer-Encoding", "chunked"), ("Content-Length", "5")]),
        &t,
    )
    .unwrap();
    assert_eq!(plan.body, Framing::Chunked);
    assert!(!head_text(&plan)
        .to_ascii_lowercase()
        .contains("content-length"));
}

/// Framing another hop could read differently is refused, not passed on.
#[test]
fn ambiguous_framing_is_refused() {
    let t = table(9);
    for bad in [
        hdrs(&[("Content-Length", "5"), ("Content-Length", "5")]),
        hdrs(&[
            ("Transfer-Encoding", "chunked"),
            ("Transfer-Encoding", "identity"),
        ]),
        hdrs(&[("Content-Length", "+5")]),
        hdrs(&[("Content-Length", "5, 5")]),
        hdrs(&[("Content-Length", "")]),
        hdrs(&[("Transfer-Encoding", "gzip")]),
    ] {
        assert_eq!(
            plan11("/anthropic/x", &bad, &t).unwrap_err().status,
            400,
            "{bad:?}"
        );
    }
}

#[test]
fn expect_continue_is_owed_only_to_http_1_1() {
    let t = table(9);
    let h = hdrs(&[("Expect", "100-continue"), ("Content-Length", "1")]);
    assert!(
        plan("POST", "/anthropic/x", 1, &h, &t)
            .unwrap()
            .expect_continue
    );
    assert!(
        !plan("POST", "/anthropic/x", 0, &h, &t)
            .unwrap()
            .expect_continue
    );
    assert!(!head_text(&plan11("/anthropic/x", &h, &t).unwrap())
        .to_ascii_lowercase()
        .contains("expect"));
}

#[test]
fn a_response_is_closed_and_framed() {
    let (head, status, framing) = rewrite_response(
        b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\n\
          Connection: keep-alive\r\nKeep-Alive: timeout=5\r\n\r\n",
        false,
    )
    .unwrap();
    let head = String::from_utf8(head).unwrap();
    assert_eq!((status, framing), (200, Framing::Chunked));
    assert!(head.contains("Transfer-Encoding: chunked\r\n"), "{head}");
    assert!(!head.contains("keep-alive"), "{head}");
    assert!(head.ends_with("Connection: close\r\n\r\n"), "{head}");

    let (head, _, framing) =
        rewrite_response(b"HTTP/1.1 200 OK\r\nContent-Length: 3\r\n\r\n", true).unwrap();
    assert_eq!(framing, Framing::None, "a HEAD response has no body");
    assert!(String::from_utf8(head)
        .unwrap()
        .contains("Content-Length: 3"));
    let (_, _, framing) = rewrite_response(b"HTTP/1.1 200 OK\r\n\r\n", false).unwrap();
    assert_eq!(framing, Framing::UntilClose);
    // Conflicting lengths fall back to reading until close, and no length is
    // passed on for the client to trust instead.
    let (head, _, framing) = rewrite_response(
        b"HTTP/1.1 200 OK\r\nContent-Length: 3\r\nContent-Length: 4\r\n\r\n",
        false,
    )
    .unwrap();
    assert_eq!(framing, Framing::UntilClose);
    assert!(!String::from_utf8(head).unwrap().contains("Content-Length"));
}

#[tokio::test]
async fn chunked_bodies_are_relayed_canonically_and_stop_at_the_end() {
    let body = b"4\r\nWiki\r\n5;ext=1\r\npedia\r\n0\r\nX-Trailer: 1\r\n\r\nNEXT REQUEST";
    let mut reader = BufReader::new(&body[..]);
    let mut out = Vec::new();
    copy_chunked(&mut reader, &mut out).await.unwrap();
    assert_eq!(out, b"4\r\nWiki\r\n5\r\npedia\r\n0\r\n\r\n".to_vec());
}

#[tokio::test]
async fn loose_chunked_framing_is_refused() {
    for bad in [
        &b"+4\r\nWiki\r\n0\r\n\r\n"[..],
        b"4\nWiki\r\n0\r\n\r\n",
        b"4\r\nWiki\n0\r\n\r\n",
        b"zz\r\n",
        b"11111111111111111\r\n",
    ] {
        let mut reader = BufReader::new(bad);
        let mut out = Vec::new();
        assert!(
            copy_chunked(&mut reader, &mut out).await.is_err(),
            "{:?}",
            String::from_utf8_lossy(bad)
        );
    }
}

// ---- over real sockets -------------------------------------------------

const TOKEN: &str = "relay-test-token";

/// A plain-HTTP origin that records one request and answers `reply`.
fn origin(reply: &'static [u8]) -> (u16, tokio::sync::oneshot::Receiver<Vec<u8>>) {
    let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = listener.local_addr().unwrap().port();
    let (tx, rx) = tokio::sync::oneshot::channel();
    std::thread::spawn(move || {
        use std::io::{Read, Write};
        let (mut sock, _) = listener.accept().unwrap();
        let _ = sock.set_read_timeout(Some(Duration::from_millis(300)));
        let mut seen = Vec::new();
        let mut chunk = [0u8; 4096];
        loop {
            match sock.read(&mut chunk) {
                Ok(0) | Err(_) => break,
                Ok(n) => seen.extend_from_slice(&chunk[..n]),
            }
        }
        let _ = sock.write_all(reply);
        let _ = tx.send(seen);
    });
    (port, rx)
}

fn services(backend: Option<u16>, table: Vec<Upstream>, payg: bool) -> Services {
    Services {
        backend: Arc::new(move || backend),
        token: Arc::from(TOKEN),
        table: Arc::new(table),
        payg: Arc::new(move || payg),
    }
}

async fn start_relay(backend: Option<u16>, table: Vec<Upstream>) -> u16 {
    start_relay_with(services(backend, table, false)).await
}

async fn start_relay_with(services: Services) -> u16 {
    let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(serve(listener, port, services, std::future::pending()));
    port
}

async fn roundtrip(port: u16, request: &[u8]) -> String {
    let mut client = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    client.write_all(request).await.unwrap();
    let mut out = Vec::new();
    tokio::time::timeout(Duration::from_secs(5), client.read_to_end(&mut out))
        .await
        .expect("the relay should answer and close")
        .unwrap();
    String::from_utf8_lossy(&out).to_string()
}

fn dead_port() -> u16 {
    std::net::TcpListener::bind(("127.0.0.1", 0))
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

/// The challenge a proof request carries, from its raw head.
fn challenge_of(req: &str) -> String {
    req.lines()
        .find_map(|l| {
            let (name, value) = l.split_once(':')?;
            name.trim()
                .eq_ignore_ascii_case(FORWARDER_CHALLENGE_HEADER)
                .then(|| value.trim().to_string())
        })
        .unwrap_or_default()
}

/// A stand-in for the engine's relay: answers the engine proof with
/// keep-alive, then answers the next request on the same connection.
async fn fake_engine() -> (u16, tokio::sync::oneshot::Receiver<Vec<u8>>) {
    let engine = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let port = engine.local_addr().unwrap().port();
    let (tx, rx) = tokio::sync::oneshot::channel::<Vec<u8>>();
    tokio::spawn(async move {
        let (mut sock, _) = engine.accept().await.unwrap();
        let mut buf = Vec::new();
        let end = read_head(&mut sock, &mut buf, true).await.unwrap().unwrap();
        let req = String::from_utf8_lossy(&buf[..end]).to_string();
        assert!(
            req.starts_with(&format!("GET {RELAY_ENGINE_HEALTH_PATH} ")),
            "{req}"
        );
        let proof = gate_connect_paths::forwarder_proof(
            TOKEN,
            RELAY_ENGINE_HEALTH_PATH,
            &challenge_of(&req),
        );
        sock.write_all(
            format!("HTTP/1.1 204 No Content\r\n{FORWARDER_PROOF_HEADER}: {proof}\r\n\r\n")
                .as_bytes(),
        )
        .await
        .unwrap();
        buf.drain(..end);
        let end = read_head(&mut sock, &mut buf, true).await.unwrap().unwrap();
        let _ = sock
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 6\r\nConnection: close\r\n\r\nengine")
            .await;
        let _ = tx.send(buf[..end].to_vec());
    });
    (port, rx)
}

/// The whole point: with no engine, a tool's request reaches its provider.
#[tokio::test]
async fn goes_to_the_provider_when_the_engine_is_gone() {
    let (origin_port, seen) =
        origin(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: keep-alive\r\n\r\nok");
    let port = start_relay(Some(dead_port()), table(origin_port)).await;

    let reply = roundtrip(
        port,
        b"POST /__gate/t/codex/anthropic/v1/messages HTTP/1.1\r\nHost: 127.0.0.1\r\n\
          Authorization: Bearer sk-own\r\nContent-Length: 4\r\n\r\nbody",
    )
    .await;
    assert!(reply.starts_with("HTTP/1.1 200 OK\r\n"), "{reply}");
    assert!(reply.contains("Connection: close\r\n"), "{reply}");
    assert!(reply.ends_with("\r\n\r\nok"), "{reply}");

    let seen = String::from_utf8(seen.await.unwrap()).unwrap();
    assert!(seen.starts_with("POST /v1/messages HTTP/1.1\r\n"), "{seen}");
    assert!(seen.contains("Authorization: Bearer sk-own\r\n"), "{seen}");
    assert!(seen.ends_with("\r\n\r\nbody"), "{seen}");
}

/// A pay-as-you-go tool sends no provider credential, so with the app
/// closed it gets an error naming the fix, not a request its provider will
/// refuse. Slugs Gate never serves pay-as-you-go still go direct.
#[tokio::test]
async fn a_payg_request_gets_an_error_not_a_credentialless_request() {
    let (origin_port, seen) = origin(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok");
    let port = start_relay_with(services(Some(dead_port()), table(origin_port), true)).await;

    let reply = roundtrip(
        port,
        b"POST /__gate/t/codex/anthropic/v1/messages HTTP/1.1\r\nHost: 127.0.0.1\r\n\
          Content-Length: 2\r\n\r\n{}",
    )
    .await;
    assert!(reply.starts_with("HTTP/1.1 503"), "{reply}");
    assert!(
        reply.contains("\"type\":\"gate_connect_not_running\""),
        "{reply}"
    );
    assert!(reply.contains("Open Gate Connect"), "{reply}");

    // `claude-web` is not pay-as-you-go eligible: it still reaches the origin.
    let reply = roundtrip(
        port,
        b"GET /claude-web/x HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n",
    )
    .await;
    assert!(reply.starts_with("HTTP/1.1 200"), "{reply}");
    assert!(String::from_utf8(seen.await.unwrap())
        .unwrap()
        .starts_with("GET /api/x "));
}

/// A relay of ours on the engine port gets the connection, byte for byte,
/// on the same connection it proved itself on.
#[tokio::test]
async fn hands_the_connection_to_an_engine_that_proves_itself() {
    let (engine_port, engine_saw) = fake_engine().await;
    let port = start_relay(Some(engine_port), table(dead_port())).await;

    let request = b"POST /__gate/t/codex/anthropic/v1/messages HTTP/1.1\r\nHost: 127.0.0.1\r\n\
                    x-gate-client: kept-for-the-engine\r\nContent-Length: 0\r\n\r\n";
    let reply = roundtrip(port, request).await;
    assert!(reply.ends_with("engine"), "{reply}");
    assert_eq!(
        engine_saw.await.unwrap(),
        request.to_vec(),
        "spliced untouched"
    );
}

/// A listener on the engine port that cannot prove itself gets no request
/// - it would carry the tool's own provider key in plaintext - and the tool
/// is not sent around it either: that listener may be a busy engine.
#[tokio::test]
async fn a_listener_that_cannot_prove_itself_gets_a_refusal() {
    let squatter = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let squatter_port = squatter.local_addr().unwrap().port();
    let (tx, rx) = std::sync::mpsc::channel::<Vec<u8>>();
    std::thread::spawn(move || {
        use std::io::{Read, Write};
        for sock in squatter.incoming() {
            let Ok(mut sock) = sock else { continue };
            let _ = sock.set_read_timeout(Some(Duration::from_millis(200)));
            let mut buf = vec![0u8; 4096];
            let n = sock.read(&mut buf).unwrap_or(0);
            let _ = tx.send(buf[..n].to_vec());
            let _ = sock.write_all(b"HTTP/1.1 204 No Content\r\n\r\n");
        }
    });
    let (origin_port, _seen) = origin(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n");
    let port = start_relay(Some(squatter_port), table(origin_port)).await;

    let reply = roundtrip(
        port,
        b"GET /anthropic/v1/models HTTP/1.1\r\nHost: 127.0.0.1\r\n\
          Authorization: Bearer sk-own\r\n\r\n",
    )
    .await;
    assert!(reply.starts_with("HTTP/1.1 502"), "{reply}");
    while let Ok(got) = rx.try_recv() {
        let got = String::from_utf8_lossy(&got).to_string();
        assert!(
            !got.contains("sk-own"),
            "the squatter must only see the challenge: {got}"
        );
    }
}

/// The reflection attack: a squatter on the engine port relays the
/// forwarder's challenge to the forwarder's own health answers and replays
/// what comes back. Neither answer is the engine path's proof, so it is
/// refused and never handed the request.
#[tokio::test]
async fn a_challenge_reflected_to_the_forwarders_own_health_is_not_trusted() {
    let port = start_relay(None, table(dead_port())).await;
    let squatter = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let squatter_port = squatter.local_addr().unwrap().port();
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<String>();
    tokio::spawn(async move {
        while let Ok((mut sock, _)) = squatter.accept().await {
            let mut buf = Vec::new();
            let Ok(Some(end)) = read_head(&mut sock, &mut buf, true).await else {
                continue;
            };
            let challenge = challenge_of(&String::from_utf8_lossy(&buf[..end]));
            // Ask the forwarder's relay listener for its proof of this
            // challenge, and replay its answer as our own.
            let reflected = roundtrip(
                port,
                format!(
                    "GET {RELAY_HEALTH_PATH} HTTP/1.1\r\nHost: 127.0.0.1\r\n\
                     {FORWARDER_CHALLENGE_HEADER}: {challenge}\r\n\r\n"
                )
                .as_bytes(),
            )
            .await;
            let proof = reflected
                .lines()
                .find_map(|l| l.strip_prefix(&format!("{FORWARDER_PROOF_HEADER}: ")))
                .unwrap_or_default()
                .to_string();
            let _ = sock
                .write_all(
                    format!("HTTP/1.1 204 No Content\r\n{FORWARDER_PROOF_HEADER}: {proof}\r\n\r\n")
                        .as_bytes(),
                )
                .await;
            let mut rest = vec![0u8; 4096];
            let n = tokio::time::timeout(Duration::from_millis(300), sock.read(&mut rest))
                .await
                .ok()
                .and_then(|r| r.ok())
                .unwrap_or(0);
            let _ = tx.send(String::from_utf8_lossy(&rest[..n]).to_string());
        }
    });
    let relay = start_relay(Some(squatter_port), table(dead_port())).await;

    let reply = roundtrip(
        relay,
        b"GET /anthropic/v1/models HTTP/1.1\r\nHost: 127.0.0.1\r\n\
          Authorization: Bearer sk-own\r\n\r\n",
    )
    .await;
    assert!(reply.starts_with("HTTP/1.1 502"), "{reply}");
    let after_proof = rx.recv().await.unwrap();
    assert!(
        !after_proof.contains("sk-own"),
        "the squatter must not receive the request: {after_proof}"
    );
}

/// With no engine, the listener still proves it is ours, says it is not
/// routing - the same answer a parked engine gives - and says it is the
/// forwarder answering.
#[tokio::test]
async fn answers_the_relay_proof_as_the_forwarder_not_intercepting() {
    let port = start_relay(None, table(dead_port())).await;
    let headers = tokio::task::spawn_blocking(move || {
        gate_connect_paths::probe_with_proof(port, RELAY_HEALTH_PATH, TOKEN)
    })
    .await
    .unwrap()
    .expect("the listener must prove the token");
    assert!(
        headers
            .iter()
            .any(|(n, v)| n == RELAY_INTERCEPTING_HEADER && v == "0"),
        "{headers:?}"
    );
    assert!(
        headers
            .iter()
            .any(|(n, v)| n == RELAY_FRONT_HEADER && v == "forwarder"),
        "{headers:?}"
    );
    let reply = roundtrip(
        port,
        format!("GET {RELAY_LIVENESS_PATH} HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n").as_bytes(),
    )
    .await;
    assert!(reply.starts_with("HTTP/1.1 204"), "{reply}");
}

#[tokio::test]
async fn a_refusal_is_an_answer_not_silence() {
    let port = start_relay(None, table(dead_port())).await;
    let reply = roundtrip(port, b"GET /nowhere/x HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n").await;
    assert!(reply.starts_with("HTTP/1.1 400"), "{reply}");
    assert!(reply.contains("X-Content-Type-Options: nosniff"), "{reply}");

    let reply = roundtrip(
        port,
        b"GET /anthropic/x HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n",
    )
    .await;
    assert!(
        reply.starts_with("HTTP/1.1 502"),
        "an unreachable provider: {reply}"
    );
}

/// A refusal sent while the client is still uploading reaches it: the
/// unread body is drained rather than answered with a reset.
#[tokio::test]
async fn a_refusal_mid_upload_arrives_as_an_answer() {
    let port = start_relay(None, table(dead_port())).await;
    let mut client = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    client
        .write_all(b"POST /nowhere/x HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Length: 200000\r\n\r\n")
        .await
        .unwrap();
    let body = vec![b'a'; 200_000];
    let _ = client.write_all(&body).await;
    let mut out = Vec::new();
    tokio::time::timeout(Duration::from_secs(5), client.read_to_end(&mut out))
        .await
        .expect("answered")
        .expect("no reset");
    assert!(String::from_utf8_lossy(&out).starts_with("HTTP/1.1 400"));
}

/// `Expect: 100-continue` is answered here, and the body still goes on.
#[tokio::test]
async fn expect_continue_gets_its_100_and_the_body_follows() {
    let (origin_port, seen) = origin(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n");
    let port = start_relay(Some(dead_port()), table(origin_port)).await;
    let mut client = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    client
        .write_all(
            b"POST /anthropic/v1/messages HTTP/1.1\r\nHost: 127.0.0.1\r\n\
              Expect: 100-continue\r\nContent-Length: 4\r\n\r\n",
        )
        .await
        .unwrap();
    let mut first = [0u8; 25];
    client.read_exact(&mut first).await.unwrap();
    assert_eq!(&first, b"HTTP/1.1 100 Continue\r\n\r\n");
    client.write_all(b"body").await.unwrap();
    let mut out = Vec::new();
    client.read_to_end(&mut out).await.unwrap();
    assert!(String::from_utf8_lossy(&out).starts_with("HTTP/1.1 200"));
    let seen = String::from_utf8(seen.await.unwrap()).unwrap();
    assert!(!seen.to_ascii_lowercase().contains("expect"), "{seen}");
    assert!(seen.ends_with("body"), "{seen}");
}

/// An interim 103 is passed on and the final answer still follows.
#[tokio::test]
async fn an_interim_response_is_passed_on() {
    let (origin_port, _seen) = origin(
        b"HTTP/1.1 103 Early Hints\r\nLink: </x>\r\n\r\nHTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok",
    );
    let port = start_relay(Some(dead_port()), table(origin_port)).await;
    let reply = roundtrip(
        port,
        b"GET /anthropic/x HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n",
    )
    .await;
    assert!(reply.starts_with("HTTP/1.1 103 Early Hints\r\n"), "{reply}");
    assert!(reply.contains("HTTP/1.1 200 OK\r\n"), "{reply}");
    assert!(reply.ends_with("ok"), "{reply}");
}

/// A chunked request body reaches the provider, re-framed canonically.
#[tokio::test]
async fn a_chunked_request_body_reaches_the_provider() {
    let (origin_port, seen) = origin(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n");
    let port = start_relay(Some(dead_port()), table(origin_port)).await;
    let reply = roundtrip(
        port,
        b"POST /anthropic/x HTTP/1.1\r\nHost: 127.0.0.1\r\nTransfer-Encoding: chunked\r\n\r\n\
          4;x=y\r\nWiki\r\n0\r\n\r\n",
    )
    .await;
    assert!(reply.starts_with("HTTP/1.1 200"), "{reply}");
    let seen = String::from_utf8(seen.await.unwrap()).unwrap();
    assert!(seen.ends_with("\r\n\r\n4\r\nWiki\r\n0\r\n\r\n"), "{seen}");
}

/// A body shorter than its declared length is not passed off as complete.
#[tokio::test]
async fn a_short_body_is_not_passed_off_as_complete() {
    // A provider that waits for the whole body before answering, as a real
    // one does.
    let silent = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let origin_port = silent.local_addr().unwrap().port();
    std::thread::spawn(move || {
        use std::io::Read;
        let (mut sock, _) = silent.accept().unwrap();
        let mut sink = [0u8; 4096];
        while sock.read(&mut sink).unwrap_or(0) > 0 {}
    });
    let port = start_relay(Some(dead_port()), table(origin_port)).await;
    let mut client = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    client
        .write_all(
            b"POST /anthropic/x HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Length: 10\r\n\r\nabc",
        )
        .await
        .unwrap();
    client.shutdown().await.unwrap();
    let mut out = Vec::new();
    let _ = tokio::time::timeout(Duration::from_secs(10), client.read_to_end(&mut out)).await;
    let out = String::from_utf8_lossy(&out);
    assert!(out.starts_with("HTTP/1.1 502"), "{out}");
    assert!(out.contains("body ended after 3 of 10 bytes"), "{out}");
}

/// The listener gives up a port the port file no longer names, so a session
/// that wrote the engine's port there does not leave it answering on a port
/// nothing points at.
#[tokio::test]
async fn serve_stops_when_released() {
    let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let (tx, rx) = tokio::sync::oneshot::channel::<()>();
    let served = tokio::spawn(serve(
        listener,
        port,
        services(None, table(dead_port()), false),
        async {
            let _ = rx.await;
        },
    ));
    tx.send(()).unwrap();
    tokio::time::timeout(Duration::from_secs(2), served)
        .await
        .expect("serve returns once released")
        .unwrap();
    assert!(TcpStream::connect(("127.0.0.1", port)).await.is_err());
}

/// Only the first attempt at the relay port may take a fresh one, and no
/// attempt takes a fresh one while a port is persisted: a retry that did would
/// persist a port no config names and move every tool off the one the app is
/// serving.
#[test]
fn only_the_first_attempt_may_take_a_fresh_port() {
    // The one test here that reads files, through the debug-build path seam.
    let dir = std::env::temp_dir().join(format!("gate-fwd-bind-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::env::set_var("GATE_CONNECT_TEST_HOME", &dir);

    assert!(bind(false, 1).is_none(), "a retry never picks a fresh port");
    let fresh = bind(true, 1).expect("a first run takes a fresh port");
    drop(fresh);

    let held = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = held.local_addr().unwrap().port();
    gate_connect_paths::save_port(RELAY_PORT_NAME, port).unwrap();
    assert!(
        bind(true, 1).is_none(),
        "a persisted port held elsewhere is waited for"
    );
    assert!(bind(false, 1).is_none());
    drop(held);
    let got = bind(false, 1).expect("the persisted port once it is free");
    assert_eq!(got.local_addr().unwrap().port(), port);

    std::env::remove_var("GATE_CONNECT_TEST_HOME");
    let _ = std::fs::remove_dir_all(&dir);
}

/// `Connection` may not take away the headers the body is framed by: the body
/// would go on with nothing saying where it ends, and the next hop would read
/// its bytes as a second request.
#[test]
fn connection_cannot_nominate_the_framing_headers() {
    let t = table(9);
    let plan = plan11(
        "/anthropic/v1/messages",
        &hdrs(&[("Connection", "content-length"), ("Content-Length", "5")]),
        &t,
    )
    .unwrap();
    assert_eq!(plan.body, Framing::Length(5));
    assert!(head_text(&plan).contains("Content-Length: 5\r\n"));

    let plan = plan11(
        "/anthropic/v1/messages",
        &hdrs(&[
            ("Connection", "Transfer-Encoding"),
            ("Transfer-Encoding", "chunked"),
        ]),
        &t,
    )
    .unwrap();
    assert_eq!(plan.body, Framing::Chunked);
    assert!(head_text(&plan).contains("Transfer-Encoding: chunked\r\n"));

    // The response side: a chunked body re-emitted as chunks has to keep the
    // header that says so.
    let (head, _, framing) = rewrite_response(
        b"HTTP/1.1 200 OK\r\nConnection: transfer-encoding\r\nTransfer-Encoding: chunked\r\n\r\n",
        false,
    )
    .unwrap();
    assert_eq!(framing, Framing::Chunked);
    assert!(String::from_utf8(head)
        .unwrap()
        .contains("Transfer-Encoding: chunked\r\n"));
}

/// `chunked` exactly once and last; and none at all on HTTP/1.0, which has no
/// transfer codings.
#[test]
fn chunked_twice_or_on_http_1_0_is_refused() {
    let t = table(9);
    for te in [
        "chunked, chunked",
        "chunked, gzip, chunked",
        ", chunked",
        "chunked,",
    ] {
        assert_eq!(
            plan11("/anthropic/x", &hdrs(&[("Transfer-Encoding", te)]), &t)
                .unwrap_err()
                .status,
            400,
            "{te}"
        );
    }
    let ok = plan11(
        "/anthropic/x",
        &hdrs(&[("Transfer-Encoding", "gzip, Chunked")]),
        &t,
    )
    .unwrap();
    assert_eq!(ok.body, Framing::Chunked);

    let te = hdrs(&[("Transfer-Encoding", "chunked")]);
    assert_eq!(
        plan("POST", "/anthropic/x", 0, &te, &t).unwrap_err().status,
        400
    );
}

/// A 103 loses the provider's connection management, like a final head, and
/// gains no `Connection: close` of its own. An HTTP/1.0 client gets no 1xx at
/// all: it would read one as the final answer.
#[tokio::test]
async fn an_interim_response_is_rewritten_and_only_for_http_1_1() {
    const REPLY: &[u8] = b"HTTP/1.1 103 Early Hints\r\nLink: </x>\r\nConnection: keep-alive\r\n\
        Keep-Alive: timeout=5\r\n\r\nHTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok";
    let (origin_port, _seen) = origin(REPLY);
    let port = start_relay(Some(dead_port()), table(origin_port)).await;
    let reply = roundtrip(
        port,
        b"GET /anthropic/x HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n",
    )
    .await;
    let (interim, rest) = reply.split_once("\r\n\r\n").unwrap();
    assert!(
        interim.starts_with("HTTP/1.1 103 Early Hints\r\n"),
        "{reply}"
    );
    assert!(interim.contains("Link: </x>"), "{reply}");
    let lower = interim.to_ascii_lowercase();
    assert!(
        !lower.contains("keep-alive") && !lower.contains("connection:"),
        "{reply}"
    );
    assert!(rest.starts_with("HTTP/1.1 200 OK\r\n"), "{reply}");

    let (origin_port, _seen) = origin(REPLY);
    let port = start_relay(Some(dead_port()), table(origin_port)).await;
    let reply = roundtrip(
        port,
        b"GET /anthropic/x HTTP/1.0\r\nHost: 127.0.0.1\r\n\r\n",
    )
    .await;
    assert!(reply.starts_with("HTTP/1.1 200 OK\r\n"), "{reply}");
    assert!(!reply.contains("103"), "{reply}");
}

/// A provider that accepts the connection and never answers the ClientHello
/// costs its budget, not the connection's slot forever.
#[tokio::test]
async fn a_stalled_tls_handshake_is_bounded() {
    let silent = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let addr = silent.local_addr().unwrap();
    let _hold = tokio::spawn(async move {
        let (sock, _) = silent.accept().await.unwrap();
        std::future::pending::<()>().await;
        drop(sock);
    });
    let tcp = TcpStream::connect(addr).await.unwrap();
    let started = std::time::Instant::now();
    let Err(err) = tls_connect("example.com", tcp, Duration::from_millis(200)).await else {
        panic!("a handshake nobody answers fails");
    };
    assert!(started.elapsed() < Duration::from_secs(5));
    assert!(format!("{err:#}").contains("did not complete"), "{err:#}");
}

/// A spliced connection on which nothing moves is let go; one that keeps
/// moving is not.
#[tokio::test]
async fn a_spliced_connection_is_closed_only_when_idle() {
    async fn pair() -> (TcpStream, TcpStream) {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let near = TcpStream::connect(listener.local_addr().unwrap())
            .await
            .unwrap();
        let (far, _) = listener.accept().await.unwrap();
        (near, far)
    }
    let idle = Duration::from_millis(300);

    let (mut client, _tool) = pair().await;
    let (mut engine, _gate) = pair().await;
    let started = std::time::Instant::now();
    let quiet = splice(&mut client, &mut engine, idle).await;
    assert!(quiet.is_err(), "an idle splice ends");
    assert!(started.elapsed() < Duration::from_secs(5));

    // The busy half gets its own, longer idle window. With the 300ms one, a
    // byte every 100ms left a 3x margin, and a loaded CI runner stretching one
    // sleep past 300ms made the splice close as idle - correctly - and the test
    // fail (seen on macOS). A byte every 50ms against 1s is a 20x margin, and
    // 45 of them still run past two idle periods in total.
    let busy_idle = Duration::from_secs(1);
    let (mut client, mut tool) = pair().await;
    let (mut engine, mut gate) = pair().await;
    let talk = tokio::spawn(async move {
        for _ in 0..45 {
            tokio::time::sleep(Duration::from_millis(50)).await;
            tool.write_all(b"x").await.unwrap();
            let mut one = [0u8; 1];
            gate.read_exact(&mut one).await.unwrap();
        }
        // Past two idle periods in total, and never idle for one.
        drop(tool);
        drop(gate);
    });
    splice(&mut client, &mut engine, busy_idle)
        .await
        .expect("a splice that keeps moving runs to its close");
    talk.await.unwrap();
}

/// Past the cap a connection is told so, rather than reset.
#[tokio::test]
async fn a_connection_over_the_cap_gets_an_answer() {
    let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(serve_capped(
        listener,
        port,
        services(None, table(dead_port()), false),
        std::future::pending(),
        1,
    ));
    // Holds the one slot: a head that never finishes.
    let mut first = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    first
        .write_all(b"GET /anthropic/x HTTP/1.1\r\n")
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;

    let reply = roundtrip(
        port,
        b"GET /anthropic/x HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n",
    )
    .await;
    assert!(reply.starts_with("HTTP/1.1 503 "), "{reply}");
    let (head, body) = reply.split_once("\r\n\r\n").unwrap();
    assert!(
        head.contains(&format!("Content-Length: {}", body.len())),
        "{reply}"
    );
}

/// A proof followed by bytes nobody asked for is not a proof: those bytes
/// would be read as the answer to the client's request.
#[tokio::test]
async fn a_proof_with_trailing_bytes_is_not_trusted() {
    let engine = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let engine_port = engine.local_addr().unwrap().port();
    tokio::spawn(async move {
        while let Ok((mut sock, _)) = engine.accept().await {
            let mut buf = Vec::new();
            let Ok(Some(end)) = read_head(&mut sock, &mut buf, true).await else {
                continue;
            };
            let req = String::from_utf8_lossy(&buf[..end]).to_string();
            let proof = gate_connect_paths::forwarder_proof(
                TOKEN,
                RELAY_ENGINE_HEALTH_PATH,
                &challenge_of(&req),
            );
            let _ = sock
                .write_all(
                    format!(
                        "HTTP/1.1 204 No Content\r\n{FORWARDER_PROOF_HEADER}: {proof}\r\n\r\n\
                         HTTP/1.1 200 OK\r\nContent-Length: 6\r\n\r\nforged"
                    )
                    .as_bytes(),
                )
                .await;
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    });
    let port = start_relay(Some(engine_port), table(dead_port())).await;
    let reply = roundtrip(
        port,
        b"GET /anthropic/x HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n",
    )
    .await;
    assert!(reply.starts_with("HTTP/1.1 502"), "{reply}");
    assert!(!reply.contains("forged"), "{reply}");
}
