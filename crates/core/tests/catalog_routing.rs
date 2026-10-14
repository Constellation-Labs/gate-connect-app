//! Every entry in the proxy catalog is proven to route (AG-906).
//!
//! A scripted HTTPS client sends one request per host and per rewrite prefix of
//! each catalog entry, through the real engine (`engine::start`), and the mock
//! gateway must receive it with the Gate headers and the right `x-gate-client`.
//!
//! [`ROUTING_ASSERTIONS`] holds one row per catalog slug. It carries only what
//! the catalog cannot say about itself: how the sending client identifies, and
//! the slug the gateway must see for it. Hosts, prefixes and the upstream URL
//! are read from `default_domains()`, so a new prefix on an existing entry is
//! covered without touching this file.
//!
//! [`every_catalog_entry_has_a_routing_assertion`] is the build check. A
//! catalog entry with no row fails it, and the message names the entry.
//!
//! Hermetic, like `proxy_e2e.rs`: a throwaway CA, a loopback mock gateway, and
//! a client that trusts only that CA and no built-in roots. A request the
//! engine did not intercept cannot complete, so nothing leaves loopback and a
//! missed route fails loudly instead of reaching the real provider. The OS
//! trust store and the system proxy are never touched.

use std::collections::BTreeSet;
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

use gate_connect_core::proxy::engine::{self, EngineConfig, RunningEngine};
use gate_connect_core::proxy::{default_domains, ProxyDomain};

/// How one catalog entry's client is identified, and what the gateway must be
/// told about it.
struct RoutingAssertion {
    /// The catalog slug this row covers.
    slug: &'static str,
    /// Headers a real client of this entry sends that name it.
    client_headers: &'static [(&'static str, &'static str)],
    /// The `x-gate-client` value the gateway must receive.
    expected_client: &'static str,
}

/// One row per catalog entry. Adding an entry to `proxy/catalog.rs` without a
/// row here fails [`every_catalog_entry_has_a_routing_assertion`].
const ROUTING_ASSERTIONS: &[RoutingAssertion] = &[
    RoutingAssertion {
        slug: "anthropic",
        client_headers: &[("anthropic-client-platform", "desktop_app")],
        expected_client: "claude-desktop",
    },
    RoutingAssertion {
        slug: "claude-web",
        client_headers: &[("anthropic-client-platform", "desktop_app")],
        expected_client: "claude-desktop",
    },
    RoutingAssertion {
        slug: "openai",
        client_headers: &[("user-agent", "codex_cli_rs/0.50.0")],
        expected_client: "codex",
    },
    RoutingAssertion {
        slug: "chatgpt-apps",
        client_headers: &[
            ("originator", "ChatGPT Desktop"),
            ("user-agent", "ChatGPT/1.2026.280 (Mac OS X 26.0; arm64)"),
        ],
        expected_client: "chatgpt",
    },
    RoutingAssertion {
        slug: "chatgpt",
        client_headers: &[
            ("originator", "codex_cli_rs"),
            ("user-agent", "codex_cli_rs/0.50.0"),
        ],
        expected_client: "codex",
    },
    RoutingAssertion {
        slug: "openrouter",
        client_headers: &[("user-agent", "openclaw/2026.9.1")],
        expected_client: "openclaw",
    },
    RoutingAssertion {
        slug: "opencode",
        client_headers: &[("user-agent", "opencode/1.0.0")],
        expected_client: "opencode",
    },
];

/// The build check. Fails, naming each entry, when the catalog and the table
/// disagree in either direction.
#[test]
fn every_catalog_entry_has_a_routing_assertion() {
    let catalog: BTreeSet<String> = default_domains().into_iter().map(|d| d.slug).collect();
    let asserted: BTreeSet<String> = ROUTING_ASSERTIONS
        .iter()
        .map(|a| a.slug.to_string())
        .collect();
    assert_eq!(
        asserted.len(),
        ROUTING_ASSERTIONS.len(),
        "ROUTING_ASSERTIONS has a duplicate slug"
    );

    let missing: Vec<&String> = catalog.difference(&asserted).collect();
    let stale: Vec<&String> = asserted.difference(&catalog).collect();
    let mut problems = Vec::new();
    for slug in &missing {
        problems.push(format!(
            "catalog entry `{slug}` has no routing assertion: add a row for it to \
             ROUTING_ASSERTIONS in crates/core/tests/catalog_routing.rs"
        ));
    }
    for slug in &stale {
        problems.push(format!(
            "ROUTING_ASSERTIONS has a row for `{slug}`, which is not in the catalog: remove it"
        ));
    }
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}

/// Serializes the engine tests in this binary. Each one boots an engine on
/// loopback ports, and one at a time keeps a failure readable.
static SERIAL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// The env var that switches the suite into OS-trust mode.
///
/// Set only by the `catalog-routing` CI job, after it has trusted this
/// install's CA machine-wide with `gate-connect proxy trust-ca --system-trust`.
/// In that mode the engine signs with that CA, and the scripted client drops
/// its pinned root and validates through the OS trust store, the way a real
/// app does. Unset (every local run, and the workspace job), the suite mints
/// a throwaway CA and pins it, and the OS store is never read.
const OS_TRUST_ENV: &str = "GATE_CONNECT_ROUTING_OS_TRUST";

fn os_trust_mode() -> bool {
    std::env::var_os(OS_TRUST_ENV).is_some_and(|v| v == "1")
}

/// Point every per-user path at a throwaway directory before the first engine
/// starts, so the install id and preferences the engine reads never touch the
/// real home. Process-global, which is fine: this file is its own binary.
///
/// In OS-trust mode the job has already pointed both seams at the directory
/// the CLI wrote the CA into, and the test must read that same CA. Both seams
/// are required there, so the mode can never reach the real keychain or home.
fn isolate_home() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        if os_trust_mode() {
            for seam in ["GATE_CONNECT_TEST_HOME", "GATE_CONNECT_TEST_SECRETS"] {
                assert!(
                    std::env::var_os(seam).is_some_and(|v| !v.is_empty()),
                    "{OS_TRUST_ENV}=1 needs {seam} set to the directory the CI job trusted the CA from"
                );
            }
            return;
        }
        let dir = std::env::temp_dir().join(format!(
            "gate-connect-catalog-routing-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).expect("creating the test home");
        std::env::set_var("GATE_CONNECT_TEST_HOME", &dir);
    });
}

#[derive(Clone)]
struct Captured {
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

/// A loopback HTTP server standing in for the Gate gateway. Records every
/// request and answers 200 with an empty body.
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

/// A throwaway root CA, as PEM cert and key.
fn mint_ca() -> (String, String) {
    let mut params = CertificateParams::new(Vec::<String>::new()).expect("CA params");
    params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    params.key_usages = vec![
        KeyUsagePurpose::KeyCertSign,
        KeyUsagePurpose::CrlSign,
        KeyUsagePurpose::DigitalSignature,
    ];
    params
        .distinguished_name
        .push(DnType::CommonName, "Gate Connect Catalog Routing Test CA");
    let key = KeyPair::generate().expect("CA key");
    let cert = params.self_signed(&key).expect("self-signing the CA");
    (cert.pem(), key.serialize_pem())
}

/// The catalog with only `slug` switched on.
fn only_enabled(slug: &str) -> Vec<ProxyDomain> {
    default_domains()
        .into_iter()
        .map(|mut d| {
            d.enabled = d.slug == slug;
            d
        })
        .collect()
}

fn start_engine(gateway: &MockGateway, domains: Vec<ProxyDomain>) -> (RunningEngine, String) {
    let ca = if os_trust_mode() {
        gate_connect_core::proxy::testing::engine_ca_for_tests()
            .expect("loading the CA the CI job trusted machine-wide")
    } else {
        mint_ca()
    };
    start_engine_with_ca(gateway, domains, ca)
}

fn start_engine_with_ca(
    gateway: &MockGateway,
    domains: Vec<ProxyDomain>,
    (ca_cert_pem, ca_key_pem): (String, String),
) -> (RunningEngine, String) {
    let engine = engine::start(
        EngineConfig {
            gateway_base_url: gateway.base_url.clone(),
            api_key: "sk-gw-catalog".into(),
            oauth_token: String::new(),
            billing_mode: Default::default(),
            org_id: String::new(),
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
    .expect("the proxy engine should start");
    (engine, ca_cert_pem)
}

/// A concrete request path under a rewrite prefix. A prefix that ends at a
/// path boundary gets a segment appended so the request names a real
/// resource rather than a directory.
fn sample_path(prefix: &str) -> String {
    if prefix == "/organizations/" {
        // claude.ai's chat turn. The suffix is what the browser narrowing
        // keys on, so the sample is the one path every client class routes.
        return "/organizations/org-1/chat_conversations/conv-1/completion".into();
    }
    if prefix.ends_with('/') {
        format!("{prefix}probe")
    } else {
        prefix.to_string()
    }
}

/// The path component of an upstream URL ("" for a bare host).
fn upstream_path(upstream_url: &str) -> &str {
    let rest = upstream_url
        .split_once("://")
        .map_or(upstream_url, |(_, r)| r);
    rest.find('/').map_or("", |i| &rest[i..])
}

/// Fetch the PAC the engine serves. macOS and Windows only: Linux routes by
/// proxy variables and the engine serves no PAC there.
#[cfg(any(target_os = "macos", target_os = "windows"))]
async fn fetch_pac(engine: &RunningEngine) -> String {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut sock = tokio::net::TcpStream::connect(("127.0.0.1", engine.pac_port()))
        .await
        .expect("the PAC responder should accept");
    sock.write_all(b"GET /proxy.pac HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n")
        .await
        .unwrap();
    let mut out = String::new();
    sock.read_to_string(&mut out).await.unwrap();
    out.split_once("\r\n\r\n")
        .map(|(_, body)| body.to_string())
        .unwrap_or_default()
}

/// The proxy the PAC names for `host`, read the way a PAC client reads the
/// script the engine writes: one `if (h === "<host>") return "PROXY <addr>; ..."`
/// line per routed host, and anything else falls to the default.
#[cfg(any(target_os = "macos", target_os = "windows"))]
fn pac_proxy_for(pac: &str, host: &str) -> Option<String> {
    let needle = format!(
        "if (h === \"{}\") return \"PROXY ",
        host.to_ascii_lowercase()
    );
    pac.lines().find_map(|line| {
        let rest = line.trim().strip_prefix(&needle)?;
        rest.split([';', '"']).next().map(str::to_string)
    })
}

/// The proxy address the scripted client uses for `host`. On macOS and
/// Windows it is read from the engine's PAC, as the system does, and an
/// entry that is on must be named there. On Linux it is the engine port, as
/// the exported proxy variables say.
async fn proxy_for(engine: &RunningEngine, host: &str) -> String {
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    {
        let pac = fetch_pac(engine).await;
        let addr = pac_proxy_for(&pac, host)
            .unwrap_or_else(|| panic!("the PAC does not route {host}:\n{pac}"));
        assert_eq!(
            addr,
            format!("127.0.0.1:{}", engine.port()),
            "the PAC must send {host} to the engine"
        );
        format!("http://{addr}")
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        let _ = host;
        format!("http://127.0.0.1:{}", engine.port())
    }
}

/// Every catalog entry, switched on alone, routes every host and rewrite
/// prefix it claims to the gateway, with the Gate headers and its client slug.
#[tokio::test]
async fn every_catalog_entry_routes_through_the_engine() {
    let _serial = SERIAL.lock().await;
    isolate_home();

    let mut failures = Vec::new();
    for entry in default_domains() {
        let Some(row) = ROUTING_ASSERTIONS.iter().find(|a| a.slug == entry.slug) else {
            // The build check above names it; nothing to drive here.
            continue;
        };
        let gateway = start_mock_gateway().await;
        let (engine, ca_pem) = start_engine(&gateway, only_enabled(&entry.slug));
        let up_path = upstream_path(&entry.upstream_url);

        let mut sent = Vec::new();
        for host in &entry.hosts {
            let builder = reqwest::Client::builder()
                .proxy(reqwest::Proxy::all(proxy_for(&engine, host).await).unwrap());
            // OS-trust mode adds no root at all: the leaf must validate through
            // the machine store or the request fails.
            let builder = if os_trust_mode() {
                builder
            } else {
                builder.tls_certs_only([reqwest::Certificate::from_pem(ca_pem.as_bytes()).unwrap()])
            };
            let client = builder.build().unwrap();
            for prefix in &entry.rewrite_prefixes {
                let gate_path = sample_path(prefix);
                let url = format!("https://{host}{up_path}{gate_path}");
                let mut req = client
                    .post(&url)
                    .header("authorization", "Bearer client-token")
                    .header("content-type", "application/json")
                    .body("{}");
                for (k, v) in row.client_headers {
                    req = req.header(*k, *v);
                }
                match req.send().await {
                    Ok(resp) if resp.status().is_success() => sent.push((url, gate_path)),
                    Ok(resp) => failures.push(format!(
                        "[{}] {url}: engine answered {}",
                        entry.slug,
                        resp.status()
                    )),
                    Err(e) => failures.push(format!("[{}] {url}: {e:#}", entry.slug)),
                }
            }
        }
        engine.stop();

        let captured = gateway.captured.lock().unwrap().clone();
        if captured.len() != sent.len() {
            failures.push(format!(
                "[{}] gateway received {} request(s), expected {}",
                entry.slug,
                captured.len(),
                sent.len()
            ));
            continue;
        }
        for ((url, gate_path), got) in sent.iter().zip(&captured) {
            let mut check = |what: &str, ok: bool, detail: String| {
                if !ok {
                    failures.push(format!("[{}] {url}: {what} ({detail})", entry.slug));
                }
            };
            check(
                "gateway path",
                got.path == *gate_path,
                format!("got {}, want {gate_path}", got.path),
            );
            check(
                "x-gate-upstream-url",
                got.header("x-gate-upstream-url") == Some(entry.upstream_url.as_str()),
                format!("got {:?}", got.header("x-gate-upstream-url")),
            );
            check(
                "x-gate-api-key",
                got.header("x-gate-api-key") == Some("sk-gw-catalog"),
                format!("got {:?}", got.header("x-gate-api-key")),
            );
            check(
                "x-gate-client",
                got.header("x-gate-client") == Some(row.expected_client),
                format!(
                    "got {:?}, want {}",
                    got.header("x-gate-client"),
                    row.expected_client
                ),
            );
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// A catalog entry switched off is not offered to the engine. On macOS and
/// Windows the PAC is the switch the system reads: with one entry on, every
/// host that only an off entry names must fall through to the default, so a
/// PAC client never sends it to the engine. The engine also reports one
/// intercepting rule, not the catalog.
#[tokio::test]
async fn an_entry_switched_off_is_not_offered_to_the_engine() {
    let _serial = SERIAL.lock().await;
    isolate_home();

    let catalog = default_domains();
    for on in &catalog {
        let gateway = start_mock_gateway().await;
        let (engine, _) = start_engine(&gateway, only_enabled(&on.slug));
        assert_eq!(
            engine.intercepting(),
            1,
            "[{}] only the entry switched on is a live rule",
            on.slug
        );

        #[cfg(any(target_os = "macos", target_os = "windows"))]
        {
            let pac = fetch_pac(&engine).await;
            for off in catalog.iter().filter(|d| d.slug != on.slug) {
                for host in off.hosts.iter().filter(|h| !on.hosts.contains(h)) {
                    assert_eq!(
                        pac_proxy_for(&pac, host),
                        None,
                        "[{}] is off, but the PAC still routes {host} with only [{}] on:\n{pac}",
                        off.slug,
                        on.slug
                    );
                }
            }
        }
        engine.stop();
    }
}

/// The control for OS-trust mode: the same client, facing an engine that signs
/// with a CA the machine does NOT trust, must fail. Without it a green run
/// could mean the client accepts anything, not that the OS store trusts ours.
/// Skipped outside OS-trust mode, where the client pins its root instead.
#[tokio::test]
async fn os_trust_mode_rejects_a_ca_the_machine_does_not_trust() {
    if !os_trust_mode() {
        return;
    }
    let _serial = SERIAL.lock().await;
    isolate_home();

    let gateway = start_mock_gateway().await;
    let (engine, _) = start_engine_with_ca(&gateway, only_enabled("anthropic"), mint_ca());
    let client = reqwest::Client::builder()
        .proxy(reqwest::Proxy::all(proxy_for(&engine, "api.anthropic.com").await).unwrap())
        .build()
        .unwrap();
    let result = client
        .post("https://api.anthropic.com/v1/messages")
        .body("{}")
        .send()
        .await;
    engine.stop();
    assert!(
        result.is_err(),
        "a leaf from an untrusted CA was accepted, so the client is not validating through the OS store"
    );
    assert!(gateway.captured.lock().unwrap().is_empty());
}
