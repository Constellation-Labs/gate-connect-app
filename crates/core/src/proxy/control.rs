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
        ca_cert_pem: String,
        ca_key_pem: String,
        domains: Vec<ProxyDomain>,
        preferred_port: Option<u16>,
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
    /// [`PROTOCOL_VERSION`]. `version` defaults to 0 when absent, so a reply
    /// from a pre-versioning daemon reads as an (incompatible) version 0.
    Hello {
        ok: bool,
        #[serde(default)]
        version: u32,
    },
    /// Acknowledges a [`Request::SetIntercept`] with the port actually bound.
    Intercepting { port: u16 },
    /// Generic success (passthrough / shutdown accepted).
    Ok,
    /// Current state for [`Request::Status`].
    Status {
        running: bool,
        port: Option<u16>,
        /// Number of domains currently intercepted (0 == pass-through).
        intercepting: usize,
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

    #[test]
    fn token_is_128_bits_hex() {
        let t = random_token().unwrap();
        assert_eq!(t.len(), 32);
        assert!(t.chars().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(t, random_token().unwrap());
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
