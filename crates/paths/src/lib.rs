//! The handful of primitives the app and the standalone forwarder binary must
//! agree on exactly: where Gate Connect's per-user files live, how a loopback
//! port is persisted, and how one is bound.
//!
//! This crate exists so `gate-connect-forwarder` can stay a small binary. It
//! cannot depend on `gate-connect-core` - that would link the keychain, the
//! MITM stack and reqwest into a process whose job is to copy bytes between
//! sockets - and it must not *duplicate* these either: the
//! forwarder and the app read and write the same files, so a disagreement
//! about where they are is a failure with no error message, just two processes
//! quietly looking at different directories.
//!
//! Nothing here touches the network beyond binding a loopback listener, and
//! nothing here handles a credential.

use std::net::{SocketAddr, TcpListener};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};

/// Test seam, read from the environment rather than a process-global so it
/// survives into a spawned child. Mirrors `gate_connect_core::env`, which is
/// the definition of record; this crate reads it so a test that redirects the
/// app also redirects the forwarder it spawns.
const TEST_HOME: &str = "GATE_CONNECT_TEST_HOME";

/// Per-OS data directory for Gate Connect's own files.
///
/// - macOS: `~/Library/Application Support/Gate Connect`
/// - Windows: `%LOCALAPPDATA%\Gate Connect`
/// - Linux: `$XDG_DATA_HOME/Gate Connect`
pub fn app_support_dir() -> Result<PathBuf> {
    // Debug builds only, matching `gate_connect_core::env::test_seam`. A
    // release binary that honoured this would let anything able to set the
    // environment - `launchctl setenv`, a mechanism Gate itself uses - redirect
    // which files the app and the forwarder agree on, and with it which port
    // the app probes and then exports machine-wide.
    #[cfg(debug_assertions)]
    if let Some(home) = std::env::var_os(TEST_HOME).filter(|v| !v.is_empty()) {
        return Ok(PathBuf::from(home).join("app-support").join("Gate Connect"));
    }
    Ok(dirs::data_local_dir()
        .context("could not resolve user data directory")?
        .join("Gate Connect"))
}

/// The `proxy/` subdirectory, where every port file and marker lives.
pub fn proxy_dir() -> Result<PathBuf> {
    Ok(app_support_dir()?.join("proxy"))
}

/// The file a port is persisted in.
///
/// **No extension.** The app has written `proxy/port`, `proxy/pac-port` and
/// `proxy/relay-port` since long before this crate existed, and every install
/// in the field has them. An earlier draft of this function appended `.port`,
/// which type-checked, passed every test - the tests inject the lookup - and
/// silently broke the one thing the forwarder is for: it never found the
/// engine, so every proxied connection went direct, bypassing the MITM engine
/// and its routing rules while the app still reported routing as on. `core`'s
/// `port_persist` calls this rather than keeping its own copy, so there is one
/// definition and it cannot drift again.
pub fn port_file(name: &str) -> Result<PathBuf> {
    Ok(proxy_dir()?.join(name))
}

/// The port persisted under `name`, if any and still parseable. A missing or
/// unreadable file reads as `None` rather than failing the caller: not knowing
/// the previous port is a normal state, and it only ever costs a fresh bind.
pub fn load_port(name: &str) -> Option<u16> {
    let path = port_file(name).ok()?;
    std::fs::read_to_string(path)
        .ok()?
        .trim()
        .parse::<u16>()
        .ok()
}

/// Persist `port` under `name` for reuse on the next run. Non-secret, so 0644
/// (the mode is ignored on Windows); written via a temp file and a rename so a
/// reader never sees a half-written number.
pub fn save_port(name: &str, port: u16) -> Result<()> {
    let path = port_file(name)?;
    let dir = path.parent().context("port file has no parent")?;
    std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, port.to_string()).with_context(|| format!("writing {}", tmp.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o644));
    }
    std::fs::rename(&tmp, &path).with_context(|| format!("renaming into {}", path.display()))
}

/// Liveness path the forwarder answers. Under a reserved prefix so it cannot
/// collide with a real destination - a proxy request names an absolute URI, and
/// this is origin-form, so nothing a client legitimately proxies reaches it.
///
/// Here rather than in either binary because it is a wire contract between
/// them: the app probes it to prove the listener on the persisted port is the
/// forwarder it spawned, and not some other process that bound the port first.
pub const FORWARDER_HEALTH_PATH: &str = "/__gate/forwarder-health";

/// Header carrying the probe's random challenge.
///
/// The app never sends the secret. An earlier design did - it put the token on
/// the request and accepted any `204` - which proved nothing (a squatter can
/// answer `204` to anything) *and* handed the secret to the very process it was
/// trying to identify. The challenge is fresh per probe, so a reply cannot be
/// replayed either.
pub const FORWARDER_CHALLENGE_HEADER: &str = "x-gate-forwarder-challenge";

/// Reserved path the **relay** answers the same proof on.
///
/// A separate path from the forwarder's so a probe cannot mistake one listener
/// for the other, and under the same `/__gate/` prefix the relay already
/// reserves for its own routing segments, so it can never collide with a tool's
/// base URL.
///
/// Answered by whatever holds the public relay port: the engine's relay, or
/// the forwarder while it fronts that port.
pub const RELAY_HEALTH_PATH: &str = "/__gate/relay-health";

/// Reserved path only the **engine's** relay answers, and the forwarder never
/// does.
///
/// It is what the forwarder asks before it hands a relay connection to the
/// engine's port, and what an enable asks before refusing because another Gate
/// holds that port. The proof binds the path it was asked on
/// ([`forwarder_proof`]), so a listener squatting the engine's port cannot get
/// this answer by relaying the challenge to one of the forwarder's own health
/// paths: the forwarder computes proofs for those two paths only, for anybody.
pub const RELAY_ENGINE_HEALTH_PATH: &str = "/__gate/relay-engine-health";

/// Header the forwarder adds to its own [`RELAY_HEALTH_PATH`] answer, so a
/// caller that reached the public relay port can tell the forwarder holding it
/// from an engine's relay holding it. Value `forwarder`.
pub const RELAY_FRONT_HEADER: &str = "x-gate-relay-front";

/// Header the relay reports interception on: `1` while it rewrites to the
/// gateway, `0` while it is parked and forwarding straight through.
///
/// Readable by any process that can reach the port, which is the same-user
/// boundary `docs/security-notes-loopback.md` already accepts, and it carries
/// nothing secret: whether Gate is routing is what the app's own window says.
pub const RELAY_INTERCEPTING_HEADER: &str = "x-gate-relay-intercepting";

/// Where the relay's **public** port is persisted: the one every relay tool
/// config names (`http://127.0.0.1:<port>/...`). On macOS and Windows the
/// forwarder holds it, so the address keeps answering after the app is gone;
/// on Linux, and wherever the forwarder could not take it, the engine's relay
/// binds it directly, as it always did.
pub const RELAY_PORT_NAME: &str = "relay-port";

/// Where the engine's relay binds when the forwarder fronts
/// [`RELAY_PORT_NAME`]. The forwarder hands relay connections here while
/// something here proves it is Gate's relay, and serves them itself when
/// nothing does. Nothing writes this port into a tool config.
pub const RELAY_ENGINE_PORT_NAME: &str = "relay-engine-port";

/// Header on the forwarder's own health answer naming the relay port it holds,
/// or `none` while it holds none.
///
/// Its *absence* is information too: a forwarder built before it fronted the
/// relay never sends it, which is how the app tells a stale forwarder, left
/// running across an update, from a current one that simply lost the port.
pub const FORWARDER_RELAY_HEADER: &str = "x-gate-forwarder-relay";

/// Reserved liveness path the relay answers with a bare 204 to anybody.
pub const RELAY_LIVENESS_PATH: &str = "/__gate/health";

/// Every port file this install keeps under `proxy/`. A fresh bind for any one
/// listener skips all of them, so it cannot take a port another of our
/// listeners is about to reclaim; each caller filters out its own name.
pub const LOOPBACK_PORT_NAMES: [&str; 5] = [
    "port",
    "pac-port",
    "forwarder-port",
    RELAY_PORT_NAME,
    RELAY_ENGINE_PORT_NAME,
];

/// The ports in [`LOOPBACK_PORT_NAMES`] other than `own`, as persisted now.
pub fn remembered_ports_except(own: &str) -> Vec<u16> {
    LOOPBACK_PORT_NAMES
        .iter()
        .filter(|name| **name != own)
        .filter_map(|name| load_port(name))
        .collect()
}

/// `account.json`, in [`app_support_dir`]. Named here because the forwarder
/// reads the billing mode out of it (see [`PAYG_ELIGIBLE_SLUGS`]).
pub const ACCOUNT_FILE_NAME: &str = "account.json";

/// Catalog slugs PAYG can serve, i.e. the ones whose forwarded path is a shape
/// the gateway's reseller router understands (`/v1/messages`,
/// `/v1/chat/completions`, `/v1/responses`).
///
/// An allowlist, not a denylist, so a domain added later defaults to BYOK and a
/// new entry can never start spending an org's balance by omission.
///
/// Everything left out is left out for a reason:
/// - `claude-web`, `chatgpt-apps` - consumer chat surfaces authenticated by a
///   session cookie and covered by the user's own subscription. Gate estimates
///   their cost rather than billing it, and their paths are not inference-API
///   shapes the reseller router serves.
/// - `chatgpt` - Codex's ChatGPT-subscription Responses route. Subscription
///   traffic is by definition not pay-as-you-go; Codex reaches PAYG through the
///   `openai` entry instead (see `integrations::codex`).
/// - `opencode` - its inference lives under `/zen/v1/…`, which is not a path
///   the reseller router recognises.
///
/// Core's `effective_billing_mode` reads this list.
///
/// Here because the forwarder needs it too: a pay-as-you-go tool's request
/// carries no provider credential of its own - Gate was going to supply the
/// provider and the bill - so the forwarder must not send it on to the
/// provider directly once the app is gone. It answers with an error naming the
/// fix instead.
pub const PAYG_ELIGIBLE_SLUGS: [&str; 3] = ["anthropic", "openai", "openrouter"];

/// Slug of the hermetic e2e's mock upstream (`GATE_CONNECT_TEST_UPSTREAM`),
/// spelled once for core's relay and the forwarder.
pub const TEST_UPSTREAM_SLUG: &str = "test-upstream";

/// Marker a relay base URL carries ahead of the catalog slug to name the tool
/// configured with it. The why is on `gate_connect_core::proxy::relay`'s
/// `TOOL_PATH_PREFIX`; it is defined here because the forwarder strips it too.
pub const RELAY_TOOL_PATH_PREFIX: &str = "/__gate/t/";

/// Every catalog slug a relay base URL may name, and the upstream it forwards
/// to when Gate is not routing it.
///
/// A copy of `slug` / `upstream_url` from `gate_connect_core::proxy::catalog`,
/// which the forwarder cannot link. A test in core asserts the two are equal,
/// so adding a catalog entry without adding it here fails the test suite rather
/// than 400ing that entry's tools the first time the app is closed.
///
/// This is the whole of the forwarder's routing knowledge, and it is what keeps
/// the relay listener from being an open proxy: a request names one of these
/// slugs or it goes nowhere.
pub const RELAY_UPSTREAMS: &[(&str, &str)] = &[
    ("anthropic", "https://api.anthropic.com"),
    ("claude-web", "https://claude.ai/api"),
    ("openai", "https://api.openai.com"),
    ("chatgpt-apps", "https://chatgpt.com"),
    ("chatgpt", "https://chatgpt.com/backend-api"),
    ("openrouter", "https://openrouter.ai/api"),
    ("opencode", "https://opencode.ai"),
];

/// [`RELAY_UPSTREAMS`], plus the hermetic e2e's mock upstream in debug builds.
///
/// `GATE_CONNECT_TEST_UPSTREAM` is the seam core's relay already honours; read
/// here too so a test that aims the relay at a mock aims the forwarder at the
/// same one. Debug builds only, for the reason [`app_support_dir`] gives.
pub fn relay_upstreams() -> Vec<(String, String)> {
    #[allow(unused_mut)]
    let mut out: Vec<(String, String)> = RELAY_UPSTREAMS
        .iter()
        .map(|(slug, url)| ((*slug).to_string(), (*url).to_string()))
        .collect();
    #[cfg(debug_assertions)]
    if let Some(url) = std::env::var_os("GATE_CONNECT_TEST_UPSTREAM").filter(|v| !v.is_empty()) {
        out.push((
            TEST_UPSTREAM_SLUG.into(),
            url.to_string_lossy().into_owned(),
        ));
    }
    out
}

/// Whether an HTTP authority (`host` or `host:port`, IPv6 in brackets) names
/// this machine's loopback - the only place our plain-HTTP loopback listeners
/// (the relay, the PAC responder, the forwarder's relay listener) may be
/// addressed from.
///
/// This is the standard local-daemon DNS-rebinding defense: a browser always
/// names its target in the `Host` header, so a page that rebound
/// `attacker.example` to 127.0.0.1 still arrives carrying
/// `Host: attacker.example` and is refused, while the CLI tools these
/// listeners exist for dial `127.0.0.1` directly. The port is deliberately not
/// pinned - every listener that calls this binds loopback exclusively, so any
/// request that reached it already used our port, and pinning would only add a
/// way to break legitimate callers.
pub fn authority_is_loopback(authority: &str) -> bool {
    let authority = authority.trim();
    // Bracketed IPv6 (`[::1]:8080` / `[::1]`) carries colons inside the
    // brackets, so strip that form before splitting off a port.
    let host = if let Some(rest) = authority.strip_prefix('[') {
        rest.split(']').next().unwrap_or("")
    } else {
        authority.rsplit_once(':').map_or(authority, |(h, _)| h)
    };
    host.eq_ignore_ascii_case("localhost") || host == "127.0.0.1" || host == "::1"
}

/// Whether an `Origin` header value may talk to our loopback listeners: only
/// a loopback origin qualifies. Anything else - a remote site's origin, or
/// the opaque `null` a sandboxed/rebound context sends - marks a cross-site
/// browser request, which must never spend the owner's Gate credential even
/// though CORS already keeps the page from reading the response ("simple"
/// cross-origin POSTs are delivered without a preflight). Non-browser
/// clients send no `Origin` at all, so they never reach this check.
pub fn origin_is_loopback(origin: &str) -> bool {
    let Some(rest) = origin
        .strip_prefix("http://")
        .or_else(|| origin.strip_prefix("https://"))
    else {
        return false;
    };
    authority_is_loopback(rest.split('/').next().unwrap_or(""))
}

/// Header carrying a listener's answer to a challenge; see [`forwarder_proof`].
/// Only a process that can read the 0600 token file can produce it, which is
/// exactly the claim the app needs before exporting the port it answers on as
/// the machine's `HTTPS_PROXY`.
pub const FORWARDER_PROOF_HEADER: &str = "x-gate-forwarder-proof";

/// The proof a listener holding `token` owes for `challenge` asked on `path`:
/// hex HMAC-SHA256 keyed by the token over the path, a NUL, and the challenge.
///
/// **The path is bound in, and that is load-bearing.** The forwarder answers
/// the forwarder and relay health paths for any caller, and it has to. If the
/// proof were the same on every path, a process squatting the engine's relay
/// port could pass the forwarder's own check by relaying its challenge to one of
/// those public answers, and be handed every relay tool's plaintext request -
/// the tool's own provider key included. Bound to the path, the answer to
/// [`RELAY_ENGINE_HEALTH_PATH`] is one the forwarder never computes.
///
/// Defined here so the binaries cannot disagree about it, and so the property
/// that matters is testable in one place: knowing the challenge is not enough
/// to produce the answer.
pub fn forwarder_proof(token: &str, path: &str, challenge: &str) -> String {
    use hmac::{Mac, SimpleHmac};
    let mut mac = SimpleHmac::<sha2::Sha256>::new_from_slice(token.as_bytes())
        .expect("HMAC takes a key of any length");
    mac.update(path.as_bytes());
    mac.update(&[0]);
    mac.update(challenge.as_bytes());
    hex(&mac.finalize().into_bytes())
}

/// The proof forwarders built before [`forwarder_proof`] bound the path:
/// SHA-256 of the token then the challenge.
///
/// Accepted in exactly one place - deciding that a forwarder left running
/// across an update is ours and should be retired - and never to trust a
/// listener with traffic.
pub fn legacy_forwarder_proof(token: &str, challenge: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(token.as_bytes());
    hasher.update(challenge.as_bytes());
    hex(&hasher.finalize())
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Does the request target carry a `.` or `..` path segment?
///
/// Both relays route by `starts_with` on the raw path and then build the URL
/// they send by concatenation, which a URL parser normalizes: the two readings
/// disagree on `/anthropic/v1/../../x`, and the one that decides where the
/// request goes is not the one that decided whether to credential it. Refusing
/// keeps one string all the way through. The encoded spellings count, because
/// URL parsers treat `%2e` as a dot when they look for these segments.
///
/// Shared by core's relay and the forwarder's relay listener, because it is a
/// security boundary and two copies of it are two chances to drift.
pub fn has_dot_segment(path_and_query: &str) -> bool {
    let path = path_and_query
        .split_once('?')
        .map(|(p, _)| p)
        .unwrap_or(path_and_query);
    // `\` too: for `http` and `https` URLs the URL parser treats a backslash
    // as a path separator, so `/v1/..\..\x` collapses exactly as
    // `/v1/../../x` does.
    path.split(['/', '\\']).any(|segment| {
        [".", "%2e", "..", ".%2e", "%2e.", "%2e%2e"]
            .iter()
            .any(|form| segment.eq_ignore_ascii_case(form))
    })
}

/// Split `/<segment>/rest?query` into `("<segment>", "/rest?query")`, or `None`
/// when there is no leading segment. A path that ends at the segment becomes
/// `"/"`, and a query directly after it keeps a `/` in front so the forwarded
/// path stays absolute. Shared for the reason [`has_dot_segment`] is.
pub fn split_leading_segment(path_and_query: &str) -> Option<(&str, String)> {
    let rest = path_and_query.strip_prefix('/')?;
    let end = rest.find(['/', '?']).unwrap_or(rest.len());
    let (segment, tail) = rest.split_at(end);
    if segment.is_empty() {
        return None;
    }
    let inner = if tail.is_empty() {
        "/".to_string()
    } else if tail.starts_with('?') {
        format!("/{tail}")
    } else {
        tail.to_string()
    };
    Some((segment, inner))
}

/// The hop-by-hop request headers of RFC 9110 section 7.6.1, which never go
/// past the hop that received them. Both relays drop these; each adds the
/// headers its own forwarding re-creates (the engine's relay re-frames bodies
/// through reqwest, the forwarder answers `Expect` itself and writes `Host`).
pub const HOP_BY_HOP: [&str; 8] = [
    "connection",
    "keep-alive",
    "proxy-connection",
    "proxy-authenticate",
    "proxy-authorization",
    "te",
    "trailer",
    "upgrade",
];

/// Compare without an early exit on the first differing byte. Timing a local
/// comparison over loopback is far-fetched, but one that leaks its own prefix
/// is not worth keeping.
pub fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// Port band a *fresh* listener is picked from. Sits inside IANA's registered
/// range and below Windows' default dynamic range (49152-65535), clear of
/// assigned neighbours like 47001 (WinRM).
///
/// Defined here because the engine and the forwarder pick from the same band
/// and must not collide with each other's remembered ports.
pub const STABLE_PORT_RANGE: std::ops::Range<u16> = 47100..47200;

/// How long a preferred-port bind keeps retrying before conceding the port.
/// Sized for a previous session still letting go, not for waiting out another
/// application.
const PREFERRED_BIND_GRACE: Duration = Duration::from_millis(250);

/// Whether something is accepting on a loopback port right now.
///
/// The one question a bind failure cannot answer on its own: a port held only
/// by `TIME_WAIT` remnants refuses, a live listener accepts.
pub fn port_is_live(port: u16) -> bool {
    std::net::TcpStream::connect_timeout(
        &SocketAddr::from(([127, 0, 0, 1], port)),
        Duration::from_millis(250),
    )
    .is_ok()
}

#[cfg(unix)]
fn bind_preferred_once(port: u16) -> std::io::Result<TcpListener> {
    if let Ok(listener) = TcpListener::bind(("127.0.0.1", port)) {
        return Ok(listener);
    }
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    if port_is_live(port) {
        // Someone is live on this port - never shadow it. `SO_REUSEADDR` below
        // would happily bind `127.0.0.1:P` underneath another process's
        // `0.0.0.0:P` listener and silently steal its loopback traffic.
        return Err(std::io::Error::from(std::io::ErrorKind::AddrInUse));
    }
    let socket = socket2::Socket::new(
        socket2::Domain::IPV4,
        socket2::Type::STREAM,
        Some(socket2::Protocol::TCP),
    )?;
    socket.set_reuse_address(true)?;
    socket.bind(&addr.into())?;
    socket.listen(128)?;
    Ok(socket.into())
}

#[cfg(not(unix))]
fn bind_preferred_once(port: u16) -> std::io::Result<TcpListener> {
    TcpListener::bind(("127.0.0.1", port))
}

/// Bind the preferred port, retrying briefly before conceding it.
///
/// A single attempt answers "is the port free right now", and for a port we are
/// trying to *re*claim that is the wrong question: our own previous session may
/// still be releasing it, and conceding on the first refusal is what makes a
/// quick restart land somewhere new - precisely what a persisted port exists to
/// prevent.
pub fn bind_preferred(port: u16) -> std::io::Result<TcpListener> {
    let deadline = Instant::now() + PREFERRED_BIND_GRACE;
    loop {
        match bind_preferred_once(port) {
            Ok(listener) => return Ok(listener),
            Err(e) if Instant::now() >= deadline => return Err(e),
            Err(_) => std::thread::sleep(Duration::from_millis(10)),
        }
    }
}

/// Bind a fresh listener for a port the caller intends to persist and rebind on
/// later runs, skipping ports this install already remembers.
///
/// Why not `:0`: a port the OS hands out as ephemeral is one it hands out to
/// everything else, and nothing holds it while Gate is stopped - so the next
/// run's rebind races every local process that opened an outbound socket in the
/// meantime. On Windows it is worse than a race: Hyper-V, WSL2 and Docker
/// Desktop reserve whole blocks of the dynamic range at boot, so a persisted
/// port inside a reserved block fails to bind on *every* start and silently
/// moves each time. Either way the clients that captured the old port are
/// stranded, which for this crate's callers is the entire failure being
/// avoided.
pub fn bind_fresh(skip: &[u16]) -> std::io::Result<TcpListener> {
    for port in STABLE_PORT_RANGE {
        if skip.contains(&port) {
            continue;
        }
        if let Ok(listener) = TcpListener::bind(("127.0.0.1", port)) {
            return Ok(listener);
        }
    }
    // Whole band unavailable: an ephemeral port keeps this session alive, but
    // the next run may not get it back, so name the reason rather than letting
    // the moving-port symptom come back unexplained.
    eprintln!(
        "gate: no free port in {STABLE_PORT_RANGE:?}; binding an ephemeral port instead. \
         It may not survive a restart."
    );
    TcpListener::bind(("127.0.0.1", 0))
}

/// Does the listener on `port` prove it holds `token`?
///
/// The client half of the challenge-response Gate's loopback listeners answer:
/// a random challenge goes out on one header, and only a process that can read
/// the 0600 token file can return the matching proof for this `health_path`. A
/// bare TCP connect cannot tell our listener from anything else that happens to
/// accept, which is the difference between "the port is taken" and "the port
/// is ours".
///
/// Bounded as a whole by [`PROBE_DEADLINE`], not only per read: a listener that
/// trickles its answer a byte at a time must not hold up an enable, a quit, or
/// the forwarder's relay connections.
pub fn proves_ours(port: u16, health_path: &str, token: &str) -> bool {
    probe_with_proof(port, health_path, token).is_some()
}

/// The whole-probe budget for [`probe_with_proof`].
pub const PROBE_DEADLINE: Duration = Duration::from_millis(750);

/// [`proves_ours`], keeping the headers the listener returned.
///
/// The proof says the listener is ours; the headers are what it reports about
/// itself. Returned together and only together, so nothing can read a claim
/// from something that failed to prove it made it.
pub fn probe_with_proof(
    port: u16,
    health_path: &str,
    token: &str,
) -> Option<Vec<(String, String)>> {
    probe(port, health_path, |challenge| {
        forwarder_proof(token, health_path, challenge)
    })
}

/// [`probe_with_proof`] against the proof forwarders computed before it bound
/// the path. Only for recognising such a forwarder so it can be retired; see
/// [`legacy_forwarder_proof`].
pub fn probe_with_legacy_proof(
    port: u16,
    health_path: &str,
    token: &str,
) -> Option<Vec<(String, String)>> {
    probe(port, health_path, |challenge| {
        legacy_forwarder_proof(token, challenge)
    })
}

fn probe(
    port: u16,
    health_path: &str,
    expected: impl Fn(&str) -> String,
) -> Option<Vec<(String, String)>> {
    use std::io::{Read, Write};

    let deadline = Instant::now() + PROBE_DEADLINE;
    let remaining = || {
        deadline
            .checked_duration_since(Instant::now())
            .filter(|d| !d.is_zero())
    };

    let challenge = fresh_challenge();
    let expected = expected(&challenge);

    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let connect_budget = remaining()?.min(Duration::from_millis(250));
    let mut sock = std::net::TcpStream::connect_timeout(&addr, connect_budget).ok()?;
    let _ = sock.set_write_timeout(remaining());
    let req = format!(
        "GET {health_path} HTTP/1.1\r\nHost: 127.0.0.1\r\n{FORWARDER_CHALLENGE_HEADER}: \
         {challenge}\r\nConnection: close\r\n\r\n"
    );
    sock.write_all(req.as_bytes()).ok()?;
    let mut buf = Vec::new();
    let mut chunk = [0u8; 512];
    while buf.len() < 4096 {
        // Each read gets only what is left of the whole budget.
        sock.set_read_timeout(Some(remaining()?)).ok()?;
        match sock.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => buf.extend_from_slice(&chunk[..n]),
            Err(_) => break,
        }
        if buf.windows(4).any(|w| w == b"\r\n\r\n") {
            break;
        }
    }
    parse_proof(&buf, &expected)
}

/// A fresh random challenge for a proof probe: 16 random bytes, hex.
pub fn fresh_challenge() -> String {
    use rand::Rng;
    let mut rng = rand::thread_rng();
    (0..16)
        .map(|_| format!("{:02x}", rng.gen::<u8>()))
        .collect()
}

/// Read a health answer's headers and check its proof against `expected`.
/// `None` unless the proof matches. Public so the forwarder can check a proof
/// it read off a connection it keeps open.
pub fn parse_proof(head: &[u8], expected: &str) -> Option<Vec<(String, String)>> {
    let text = std::str::from_utf8(head).ok()?;
    let headers: Vec<(String, String)> = text
        .lines()
        .filter_map(|line| line.split_once(':'))
        .map(|(name, value)| (name.trim().to_ascii_lowercase(), value.trim().to_string()))
        .collect();
    let proved = headers.iter().any(|(name, value)| {
        name == FORWARDER_PROOF_HEADER && constant_time_eq(value.as_bytes(), expected.as_bytes())
    });
    proved.then_some(headers)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The path seam is an environment variable, which is process-global, so
    /// tests that set it cannot overlap. `core` carries the same lock for the
    /// same reason; without one these fail under the default parallel runner
    /// and pass under `--test-threads=1`, which is the worst way to find out.
    fn path_env_lock() -> std::sync::MutexGuard<'static, ()> {
        static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
        LOCK.lock().unwrap_or_else(|e| e.into_inner())
    }

    struct TestHome {
        dir: PathBuf,
        /// Held for the test's life; released on drop with the env var.
        _guard: std::sync::MutexGuard<'static, ()>,
    }

    impl TestHome {
        fn set(tag: &str) -> Self {
            let guard = path_env_lock();
            let dir = std::env::temp_dir().join(format!(
                "gate-connect-paths-{}-{}-{tag}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            std::fs::create_dir_all(&dir).unwrap();
            std::env::set_var(TEST_HOME, &dir);
            TestHome { dir, _guard: guard }
        }
    }

    impl Drop for TestHome {
        fn drop(&mut self) {
            std::env::remove_var(TEST_HOME);
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    /// The point of the challenge: the answer cannot be produced from the
    /// challenge alone, so a process that cannot read the token file cannot
    /// pass, however it replies.
    #[test]
    fn a_proof_needs_the_token_not_just_the_challenge() {
        let proof = forwarder_proof("the-token", RELAY_HEALTH_PATH, "abc123");
        assert_eq!(proof.len(), 64);
        assert_eq!(
            proof,
            forwarder_proof("the-token", RELAY_HEALTH_PATH, "abc123")
        );
        assert_ne!(
            proof,
            forwarder_proof("another-token", RELAY_HEALTH_PATH, "abc123")
        );
        // And it is bound to the challenge, so one reply cannot be replayed
        // against the next probe.
        assert_ne!(
            proof,
            forwarder_proof("the-token", RELAY_HEALTH_PATH, "abc124")
        );
    }

    /// The property the splice gate rests on: an answer the forwarder gives
    /// anybody on its own health paths is never the engine path's answer.
    #[test]
    fn a_proof_is_bound_to_the_path_it_was_asked_on() {
        let engine = forwarder_proof("t", RELAY_ENGINE_HEALTH_PATH, "c");
        assert_ne!(engine, forwarder_proof("t", RELAY_HEALTH_PATH, "c"));
        assert_ne!(engine, forwarder_proof("t", FORWARDER_HEALTH_PATH, "c"));
        assert_ne!(engine, legacy_forwarder_proof("t", "c"));
    }

    /// A listener that trickles its answer is cut off by the whole-probe
    /// deadline, not allowed a fresh read timeout per byte.
    #[test]
    fn a_trickling_listener_cannot_hold_a_probe() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            use std::io::{Read, Write};
            let (mut sock, _) = listener.accept().unwrap();
            let mut buf = [0u8; 512];
            let _ = sock.read(&mut buf);
            for _ in 0..100 {
                if sock.write_all(b"x").is_err() {
                    return;
                }
                std::thread::sleep(Duration::from_millis(100));
            }
        });
        let started = Instant::now();
        assert!(!proves_ours(port, RELAY_HEALTH_PATH, "t"));
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "{:?}",
            started.elapsed()
        );
    }

    #[test]
    fn dot_segments_are_found_in_every_spelling() {
        for bad in [
            "/anthropic/v1/../../x",
            "/anthropic/./v1",
            "/anthropic/%2E%2e/x",
            "/a/.%2e?q",
            "/anthropic/v1/..\\..\\x",
            "/anthropic/v1/.%2E\\.%2e\\x",
        ] {
            assert!(has_dot_segment(bad), "{bad}");
        }
        for ok in ["/anthropic/v1/messages", "/a/..b/c", "/a?x=../y"] {
            assert!(!has_dot_segment(ok), "{ok}");
        }
    }

    #[test]
    fn constant_time_eq_is_still_an_equality() {
        assert!(constant_time_eq(b"abc", b"abc"));
        assert!(!constant_time_eq(b"abc", b"abd"));
        assert!(!constant_time_eq(b"abc", b"ab"));
    }

    #[test]
    fn a_port_round_trips_and_a_missing_one_reads_as_none() {
        let _home = TestHome::set("round-trip");
        assert_eq!(load_port("never-written"), None);
        save_port("engine", 47_123).unwrap();
        assert_eq!(load_port("engine"), Some(47_123));
        // Distinct names must not clobber each other.
        save_port("forwarder", 47_124).unwrap();
        assert_eq!(load_port("engine"), Some(47_123));
        assert_eq!(load_port("forwarder"), Some(47_124));
    }

    #[test]
    fn unparseable_content_reads_as_none_rather_than_failing() {
        let _home = TestHome::set("garbage");
        let path = port_file("torn").unwrap();
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "not a port").unwrap();
        assert_eq!(load_port("torn"), None);
    }

    #[test]
    fn a_fresh_bind_lands_in_the_band_and_skips_what_we_already_hold() {
        let held = bind_fresh(&[]).unwrap();
        let held_port = held.local_addr().unwrap().port();
        assert!(
            STABLE_PORT_RANGE.contains(&held_port),
            "fresh binds come from the band, not the ephemeral range"
        );
        let other = bind_fresh(&[held_port]).unwrap();
        assert_ne!(other.local_addr().unwrap().port(), held_port);
    }

    #[test]
    fn a_preferred_bind_refuses_to_shadow_a_live_listener() {
        let live = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = live.local_addr().unwrap().port();
        // Shadowing is the failure that matters: `SO_REUSEADDR` makes it
        // possible, and it would silently steal another app's loopback traffic.
        assert!(bind_preferred(port).is_err());
        assert!(port_is_live(port));
        drop(live);
    }
}
