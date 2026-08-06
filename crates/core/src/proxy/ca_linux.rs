//! Local root CA for the proxy (Linux). We generate a CA once, keep its private
//! key in the OS secret store (via [`crate::keychain`] → Secret Service) and its
//! public cert on disk, and (on enable) install the cert into the **system**
//! trust store so the MITM engine can mint per-host leaf certs that the OS - and
//! command-line tools that read the system bundle (curl, git, openssl) - accept.
//!
//! Unlike macOS/Windows, Linux has no per-user root store: trust is system-wide
//! and the install needs root. We support the two common layouts:
//!
//! - Debian/Ubuntu/Arch: drop the PEM in `/usr/local/share/ca-certificates/`
//!   and run `update-ca-certificates`.
//! - Fedora/RHEL/openSUSE: drop it in `/etc/pki/ca-trust/source/anchors/` and
//!   run `update-ca-trust extract`.
//!
//! The privileged step is performed via [`crate::primitives::run_as_admin`]
//! (sudo in a terminal, polkit/`pkexec` in a GUI session). Tools that ship their
//! own CA bundle instead of using the system store still need pointing at our
//! CA: Node-based CLIs (e.g. Claude Code) are covered by `NODE_EXTRA_CA_CERTS`,
//! which [`super::system_proxy`] writes alongside the proxy variables.
//!
//! Trust is tightly scoped: the CA only ever signs leaf certs for the handful of
//! inference hosts the user explicitly enables (every other host is
//! blind-tunnelled, never MITM'd), and the private key never leaves the secret
//! store. Linux counterpart of the macOS [`super::ca`] module.

use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result};
use hudsucker::rcgen::KeyPair;

use crate::env;
use crate::keychain;
use crate::primitives::{run_as_admin, sh_quote};
use crate::proxy::cert_authority;

/// Subject CN of our CA. Used both as the cert subject and as the basename of
/// the installed anchor file.
pub const CA_COMMON_NAME: &str = cert_authority::CA_COMMON_NAME;

/// A loaded CA. The cert is public; the key is sensitive and only handed to the
/// engine (same process) to build the signing authority.
pub struct Ca {
    cert_pem: String,
    key_pem: String,
}

impl Ca {
    pub fn cert_pem(&self) -> &str {
        &self.cert_pem
    }

    pub(crate) fn key_pem(&self) -> &str {
        &self.key_pem
    }
}

fn cert_path() -> Result<PathBuf> {
    Ok(env::app_support_dir()?.join("proxy").join("ca-cert.pem"))
}

fn key_service() -> String {
    keychain::tool_service("proxy", "ca-key")
}

/// A system trust store layout: where to drop our anchor PEM and how to rebuild
/// the consolidated bundle afterwards. `refresh_cmd` is the rebuild used after a
/// removal (Debian's `update-ca-certificates` only *adds* unless told to start
/// fresh; `update-ca-trust extract` handles both).
struct TrustStore {
    anchor: PathBuf,
    install_cmd: &'static str,
    refresh_cmd: &'static str,
}

/// Resolve the distro's trust store by probing for its update tool. Debian-
/// family (and Arch) ship `update-ca-certificates`; RHEL-family and openSUSE
/// ship `update-ca-trust`.
fn trust_store() -> Result<TrustStore> {
    let anchor_file = format!("{CA_COMMON_NAME}.crt");
    if PathBuf::from("/usr/sbin/update-ca-certificates").exists()
        || PathBuf::from("/usr/bin/update-ca-certificates").exists()
    {
        return Ok(TrustStore {
            anchor: PathBuf::from("/usr/local/share/ca-certificates").join(&anchor_file),
            install_cmd: "update-ca-certificates",
            refresh_cmd: "update-ca-certificates --fresh",
        });
    }
    if PathBuf::from("/usr/bin/update-ca-trust").exists() {
        return Ok(TrustStore {
            anchor: PathBuf::from("/etc/pki/ca-trust/source/anchors").join(&anchor_file),
            install_cmd: "update-ca-trust extract",
            refresh_cmd: "update-ca-trust extract",
        });
    }
    anyhow::bail!(
        "unsupported Linux distribution: neither update-ca-certificates nor update-ca-trust found"
    )
}

fn generate() -> Result<(String, String)> {
    let params = cert_authority::ca_certificate_params()?;
    let key_pair = KeyPair::generate().context("generating CA key pair")?;
    let cert = params
        .self_signed(&key_pair)
        .context("self-signing CA certificate")?;
    Ok((cert.pem(), key_pair.serialize_pem()))
}

/// Load the CA, generating + persisting one on first use. The pair is kept in
/// sync: if either half is missing we regenerate both.
pub fn load_or_create() -> Result<Ca> {
    let user = env::current_user()?;
    let service = key_service();
    let path = cert_path()?;

    let existing_key = keychain::get(&service, &user)?;
    let existing_cert = match fs::read_to_string(&path) {
        Ok(c) => Some(c),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
    };

    if let (Some(key_pem), Some(cert_pem)) = (existing_key, existing_cert) {
        // Presence is not enough: the stored root's X.509 name constraints were
        // fixed at generation time from the domain catalog, so one minted before
        // a host was added cannot issue for it and interception of that host dies
        // at the handshake with nothing naming the cause. A stale fingerprint (or
        // none, on an install predating this check) falls through to regenerate.
        //
        // Safe to regenerate here because callers invoke `ensure_trusted()`
        // immediately after, and `is_trusted()` is content-keyed on all three
        // platforms — thumbprint, `verify-cert` against the current file, and a
        // content comparison — so it reports false for the new root and the trust
        // step installs it rather than short-circuiting on the old one.
        if cert_authority::host_fingerprint_is_current(&path) {
            return Ok(Ca { cert_pem, key_pem });
        }
    }

    let (cert_pem, key_pem) = generate()?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
    }
    fs::write(&path, &cert_pem).with_context(|| format!("writing {}", path.display()))?;
    keychain::set(&service, &user, &key_pem)?;
    // Written after the cert so an interrupted sequence leaves the sidecar
    // absent or stale — never describing a cert that isn't there yet.
    cert_authority::write_host_fingerprint(&path)?;
    Ok(Ca { cert_pem, key_pem })
}

/// Whether our *current* CA is installed in the system trust store. The
/// anchor file (a byte-copy of our cert installed by `ensure_trusted`)
/// must exist **and match the cert on disk** - a presence-only check would
/// let a regenerated pair no-op `ensure_trusted` while the stale root
/// stays in the bundle and every MITM handshake fails. Re-installing
/// overwrites the same anchor filename, so a mismatch self-heals on the
/// next `ensure_trusted`. The anchor dir is world-readable, so this is
/// non-privileged.
pub fn is_trusted() -> Result<bool> {
    let store = trust_store()?;
    let anchor = match fs::read_to_string(&store.anchor) {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(e) => {
            return Err(e).with_context(|| format!("reading {}", store.anchor.display()));
        }
    };
    let cert = match fs::read_to_string(cert_path()?) {
        Ok(c) => c,
        // No local cert means whatever is anchored isn't our current CA.
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(e) => return Err(e).context("reading the local CA cert"),
    };
    Ok(anchor == cert)
}

/// Trust the CA if it isn't already. Copies the public cert into the distro's
/// anchor directory and rebuilds the system bundle, in a single privileged
/// invocation (so the user authenticates once). The private key is never
/// touched here - only the public cert leaves the secret store.
pub fn ensure_trusted() -> Result<()> {
    if is_trusted()? {
        return Ok(());
    }
    let store = trust_store()?;
    let cert = cert_path()?;
    let parent = store.anchor.parent().context("anchor path has no parent")?;
    let script = format!(
        "/bin/mkdir -p {parent} && /usr/bin/install -m 0644 {src} {dst} && {update}",
        parent = sh_quote(&parent.display().to_string()),
        src = sh_quote(&cert.display().to_string()),
        dst = sh_quote(&store.anchor.display().to_string()),
        update = store.install_cmd,
    );
    run_as_admin(&script).context("installing the proxy CA into the system trust store")?;
    Ok(())
}

/// Remove the CA's trust: delete our anchor file and rebuild the bundle so the
/// cert drops out of it. Privileged. Keyed on the anchor *existing*, not on
/// `is_trusted` - a stale anchor left by a regenerated pair must still be
/// removable.
pub fn untrust() -> Result<()> {
    let store = trust_store()?;
    if store.anchor.exists() {
        let script = format!(
            "/bin/rm -f {dst} && {refresh}",
            dst = sh_quote(&store.anchor.display().to_string()),
            refresh = store.refresh_cmd,
        );
        run_as_admin(&script).context("removing the proxy CA from the system trust store")?;
    }
    remove_ca_material()
}

/// Full teardown for an explicit removal: drop the private key from the secret
/// store and the public cert from disk, so "remove" clears the MITM material
/// rather than only the system-store anchor. Best-effort on the key (a missing
/// entry is fine) and on an absent cert file.
fn remove_ca_material() -> Result<()> {
    let _ = keychain::delete(&key_service(), &env::current_user()?);
    let cert = cert_path()?;
    // The catalog fingerprint sidecar is CA material too — leaving it behind
    // would be residue after an untrust that claims to remove everything.
    let _ = fs::remove_file(cert_authority::host_fingerprint_path(&cert));
    match fs::remove_file(&cert) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e).with_context(|| format!("removing {}", cert.display())),
    }
}
