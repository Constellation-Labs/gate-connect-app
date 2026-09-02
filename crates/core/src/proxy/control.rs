//! Control protocol between the GUI app and the long-lived proxy helper daemon
//! ([`super::helper`]), plus the client used to drive it ([`super::helper_client`]).
//!
//! The daemon owns the loopback listener so the proxy outlives the GUI process:
//! when the GUI quits or crashes, the daemon drops to pass-through (blind-tunnel
//! everything) rather than letting a frozen session's proxy pointer dangle at a
//! dead port. The GUI talks to it over a **Unix-domain socket** in
//! `$XDG_RUNTIME_DIR` - never a TCP port, which on loopback is reachable by
//! every local user. Access control is layered:
//!
//! 1. The socket dir is `0700` and the socket `0600` (owner-only).
//! 2. Every accepted connection is checked with `SO_PEERCRED`; a peer UID other
//!    than ours is rejected.
//! 3. The first message must carry a per-run token the daemon wrote to a `0600`
//!    file only the owner can read (defense-in-depth behind the UID check).
//!
//! The guiding principle: the trusted CA must only ever be driven by a caller
//! authenticated as the owner. The daemon also validates every requested
//! intercept domain against the built-in catalog, so even an authenticated
//! caller can't point the MITM at an arbitrary host/upstream.
//!
//! Wire format: newline-delimited JSON, one [`Request`]/[`Response`] per line.

use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::account::BillingMode;
use crate::proxy::{default_domains, ProxyDomain};

/// Subdirectory under `$XDG_RUNTIME_DIR` holding the daemon's socket + token.
/// `$XDG_RUNTIME_DIR` is itself per-user `0700` and cleared on logout, which is
/// exactly the lifetime we want for the control channel (transient, not
/// surviving a reboot).
const RUNTIME_SUBDIR: &str = "gate-connect";
const SOCKET_NAME: &str = "proxyd.sock";
const TOKEN_NAME: &str = "proxyd.token";

/// Resolve `$XDG_RUNTIME_DIR/gate-connect`, creating it `0700` if needed.
/// Falls back to a `0700` dir under the system temp dir keyed on the uid when
/// `$XDG_RUNTIME_DIR` is unset (rare; e.g. a bare `su` session).
pub fn runtime_dir() -> Result<PathBuf> {
    let base = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            // SAFETY: getuid is always safe and never fails.
            let uid = unsafe { libc::getuid() };
            std::env::temp_dir().join(format!("gate-connect-{uid}"))
        });
    let dir = base.join(RUNTIME_SUBDIR);
    std::fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
    set_mode(&dir, 0o700).with_context(|| format!("locking down {}", dir.display()))?;
    Ok(dir)
}

pub fn socket_path() -> Result<PathBuf> {
    Ok(runtime_dir()?.join(SOCKET_NAME))
}

pub fn token_path() -> Result<PathBuf> {
    Ok(runtime_dir()?.join(TOKEN_NAME))
}

/// Lockfile the daemon holds for its whole life so only one daemon runs per
/// user session (see [`super::flock`]).
pub fn singleton_lock_path() -> Result<PathBuf> {
    Ok(runtime_dir()?.join("proxyd.lock"))
}

/// File the daemon writes its PID to at startup, so a client that can't shut it
/// down cleanly (e.g. a leftover speaking a protocol the client no longer
/// understands) can force-kill it and take over.
pub fn pid_path() -> Result<PathBuf> {
    Ok(runtime_dir()?.join("proxyd.pid"))
}

fn set_mode(path: &std::path::Path, mode: u32) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))
        .with_context(|| format!("setting mode {mode:o} on {}", path.display()))
}

/// 32 hex chars (128 bits) from `/dev/urandom`. Used as the control-socket
/// auth token. Distinct from `primitives::install_id` (telemetry uuid).
pub fn random_token() -> Result<String> {
    use std::io::Read;
    let mut bytes = [0u8; 16];
    std::fs::File::open("/dev/urandom")
        .context("opening /dev/urandom")?
        .read_exact(&mut bytes)
        .context("reading /dev/urandom")?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

/// Control-protocol version. Bump on any wire-incompatible change to
/// [`Request`]/[`Response`] (e.g. a new required field). Exchanged in the
/// `Hello` handshake so a client detects a daemon left over from an older build
/// and replaces it, rather than talking a mismatched protocol and failing later
/// on a reply whose shape it no longer recognizes.
pub const PROTOCOL_VERSION: u32 = 1;

/// Digest of the core crate's sources, computed by `build.rs` and exchanged in
/// the `Hello` handshake alongside [`PROTOCOL_VERSION`]. The version only
/// changes on wire-incompatible edits, so a daemon whose *behavior* differs
/// (new catalog entry, relay fix, engine change) used to be reused silently
/// and e.g. reject a domain the client's catalog has. Any core source change
/// alters this fingerprint, so the client replaces such a daemon instead.
pub const BUILD_FINGERPRINT: &str = env!("GATE_CORE_FINGERPRINT");

/// GUI → daemon. The CA + gateway travel only inside [`Request::SetIntercept`];
/// the daemon needs them to mint leaf certs and rewrite to the gateway.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Request {
    /// First message on every connection: authenticate with the per-run token
    /// and advertise the client's [`PROTOCOL_VERSION`]. `version` defaults to 0
    /// when absent, so a newer daemon can still parse an older client's Hello.
    Hello {
        token: String,
        #[serde(default)]
        version: u32,
    },
    /// Start (or live-update) interception. Carries everything the engine needs
    /// for one session. `preferred_port` reuses the stable port across runs.
    SetIntercept {
        gateway_base_url: String,
        api_key: String,
        /// Cognito access token to inject instead of the API key; empty means
        /// fall back to the API key.
        oauth_token: String,
        /// Selected org UUID, injected as `X-Gate-Org-Id` alongside the OAuth
        /// token; empty means none selected.
        org_id: String,
        /// Who pays the upstream provider. `serde(default)` keeps an older
        /// client's message parseable, and defaults to `Byok` - the shape that
        /// forwards the tool's own credential, so a client that cannot say
        /// which mode it wants never spends an org's balance by accident.
        #[serde(default)]
        billing_mode: BillingMode,
        ca_cert_pem: String,
        ca_key_pem: String,
        domains: Vec<ProxyDomain>,
        /// Keep intercepting after this client's control connection closes.
        ///
        /// The daemon otherwise reverts to pass-through on any disconnect,
        /// which is right for the GUI: it holds the connection for as long as
        /// routing is meant to be on, so a lost connection means the owner is
        /// gone and a machine should not be left behind a proxy nobody holds.
        ///
        /// It is wrong for the CLI, which is short-lived by construction.
        /// `gate-connect proxy enable` connected, armed the engine, printed
        /// success and exited - and the exit cleared every rule, so the engine
        /// tunnelled everything while status still read "on". This flag is how
        /// a caller says its own lifetime is not the routing lifetime.
        ///
        /// `serde(default)` keeps an older client's message parseable; absent
        /// means the old always-revert behaviour.
        #[serde(default)]
        detached: bool,
        preferred_port: Option<u16>,
        /// Preferred loopback port for the CLI reverse-proxy relay, so it
        /// rebinds the same address across restarts and baked CLI configs stay
        /// valid.
        preferred_relay_port: Option<u16>,
    },
    /// Drop to pass-through: keep listening, blind-tunnel everything. Used on
    /// the GUI's explicit "off" - the engine keeps the port bound so frozen
    /// sessions aren't stranded.
    SetPassthrough,
    /// Report current state.
    Status,
    /// Ask the daemon to exit (used by the CLI / tests, not normal GUI quit).
    Shutdown,
}

/// daemon → GUI.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Response {
    /// Auth result for [`Request::Hello`], carrying the daemon's
    /// [`PROTOCOL_VERSION`] and [`BUILD_FINGERPRINT`]. Both default when
    /// absent (version 0, empty fingerprint), so a reply from an older daemon
    /// reads as incompatible.
    Hello {
        ok: bool,
        #[serde(default)]
        version: u32,
        #[serde(default)]
        fingerprint: String,
    },
    /// Acknowledges a [`Request::SetIntercept`] with the MITM proxy port and
    /// the CLI reverse-proxy relay port actually bound.
    Intercepting { port: u16, relay_port: u16 },
    /// Generic success (passthrough / shutdown accepted).
    Ok,
    /// Current state for [`Request::Status`].
    Status {
        running: bool,
        port: Option<u16>,
        /// Number of domains currently intercepted (0 == pass-through).
        intercepting: usize,
        /// Times the engine has seen the gateway refuse a request carrying
        /// *our* OAuth bearer, since this daemon started. Monotone, so the GUI
        /// acts on the edge (the count moved) rather than on a level, and a
        /// missed poll cannot lose the signal.
        ///
        /// This is how 401-driven session recovery reaches Linux at all. On the
        /// other platforms the engine is in-process and notifies the shell
        /// directly (`proxy::set_gate_auth_observer`); here it lives in this
        /// daemon, so the observation has to survive a trip over the control
        /// socket. The daemon deliberately draws no conclusion from it - see
        /// `startup::reverify_session` for why only a probe with our own token
        /// may call a session dead.
        ///
        /// `serde(default)` keeps an older daemon's reply parseable, reading as
        /// "never refused", which is the pre-feature behaviour.
        #[serde(default)]
        gate_auth_refusals: u64,
    },
    /// Something went wrong; `message` is human-readable.
    Error { message: String },
}

/// Reject any requested intercept domain that doesn't exactly match a built-in
/// catalog entry (same slug, hosts, and upstream). This is the "constrain
/// intercept config" guard: even a caller past the UID + token checks can only
/// route the providers we ship - never an arbitrary host onto an arbitrary
/// upstream under the trusted CA.
pub fn validate_domains(domains: &[ProxyDomain]) -> Result<()> {
    let catalog = default_domains();
    for d in domains {
        let known = catalog.iter().find(|c| c.slug == d.slug).with_context(|| {
            format!(
                "intercept domain {:?} is not in the built-in catalog",
                d.slug
            )
        })?;
        if d.hosts != known.hosts {
            anyhow::bail!("intercept domain {:?} has hosts not in the catalog", d.slug);
        }
        if d.upstream_url != known.upstream_url {
            anyhow::bail!(
                "intercept domain {:?} upstream {:?} does not match the catalog",
                d.slug,
                d.upstream_url
            );
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_round_trips_as_jsonl() {
        let req = Request::SetPassthrough;
        let line = serde_json::to_string(&req).unwrap();
        assert!(!line.contains('\n'));
        let back: Request = serde_json::from_str(&line).unwrap();
        assert!(matches!(back, Request::SetPassthrough));
    }

    /// The billing mode crosses a process boundary into a separately-installed
    /// daemon binary, so both halves of that hop need pinning: a mode we send
    /// must arrive, and a message from a client too old to know the field must
    /// still parse - as `Byok`, the shape that forwards the tool's own
    /// credential rather than spending an org's balance.
    #[test]
    fn set_intercept_carries_the_billing_mode() {
        let req = Request::SetIntercept {
            gateway_base_url: "https://gw.example.com".into(),
            api_key: "sk-gw-test".into(),
            oauth_token: String::new(),
            org_id: String::new(),
            billing_mode: BillingMode::Payg,
            ca_cert_pem: String::new(),
            ca_key_pem: String::new(),
            domains: Vec::new(),
            detached: false,
            preferred_port: None,
            preferred_relay_port: None,
        };
        let line = serde_json::to_string(&req).unwrap();
        let back: Request = serde_json::from_str(&line).unwrap();
        match back {
            Request::SetIntercept { billing_mode, .. } => {
                assert_eq!(billing_mode, BillingMode::Payg)
            }
            other => panic!("expected SetIntercept, got {other:?}"),
        }
    }

    #[test]
    fn set_intercept_without_a_billing_mode_parses_as_byok() {
        let line = r#"{"SetIntercept":{"gateway_base_url":"https://gw.example.com",
            "api_key":"sk-gw-test","oauth_token":"","org_id":"","ca_cert_pem":"",
            "ca_key_pem":"","domains":[],"preferred_port":null,
            "preferred_relay_port":null}}"#;
        let back: Request = serde_json::from_str(line).expect("an older client must still parse");
        match back {
            Request::SetIntercept { billing_mode, .. } => {
                assert_eq!(billing_mode, BillingMode::Byok)
            }
            other => panic!("expected SetIntercept, got {other:?}"),
        }
    }

    #[test]
    fn status_without_a_refusal_count_parses_as_never_refused() {
        let line = r#"{"Status":{"running":true,"port":45981,"intercepting":7}}"#;
        let back: Response = serde_json::from_str(line).expect("an older daemon must still parse");
        match back {
            Response::Status {
                gate_auth_refusals, ..
            } => assert_eq!(gate_auth_refusals, 0),
            other => panic!("expected Status, got {other:?}"),
        }
    }

    #[test]
    fn token_is_128_bits_hex() {
        let t = random_token().unwrap();
        assert_eq!(t.len(), 32);
        assert!(t.chars().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(t, random_token().unwrap());
    }

    #[test]
    fn build_fingerprint_is_hex_u64() {
        // Source-sensitivity is inherent to build.rs (it hashes `src/`); here
        // we only pin the shape the handshake compares.
        assert_eq!(BUILD_FINGERPRINT.len(), 16);
        assert!(BUILD_FINGERPRINT.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn validate_accepts_catalog_domains() {
        assert!(validate_domains(&default_domains()).is_ok());
    }

    #[test]
    fn validate_rejects_unknown_slug() {
        let mut d = default_domains();
        d[0].slug = "evil".into();
        assert!(validate_domains(&d).is_err());
    }

    #[test]
    fn validate_rejects_tampered_upstream() {
        let mut d = default_domains();
        d[0].upstream_url = "https://attacker.example".into();
        assert!(validate_domains(&d).is_err());
    }

    #[test]
    fn validate_rejects_extra_host() {
        let mut d = default_domains();
        d[0].hosts.push("evil.example".into());
        assert!(validate_domains(&d).is_err());
    }
}
