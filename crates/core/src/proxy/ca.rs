//! Local root CA for the proxy. We generate a CA once, keep its private key
//! in the OS keychain and its public cert on disk, and (on enable) install
//! the cert as a trusted root in the user's login keychain so the MITM engine
//! can mint per-host leaf certs the OS trust store will accept.
//!
//! Trust is installed by running `security add-trusted-cert` directly (NOT
//! via `run_as_admin`/osascript): modifying trust settings needs the
//! interactive Security-Agent authorization dialog, which the AppleScript
//! "with administrator privileges" context cannot provide (it fails with
//! "authorization was denied since no user interaction was possible"). Using
//! the user login keychain also avoids needing root. User-domain trust is
//! honored by the system trust evaluation for this user's apps.
//!
//! Trust is tightly scoped: the CA only ever signs leaf certs for the handful
//! of inference hosts the user explicitly enables (every other host is
//! blind-tunnelled, never MITM'd), and the private key never leaves the
//! keychain. macOS only.

use std::fs;
use std::path::PathBuf;
use std::process::Command;

use anyhow::{Context, Result};
use hudsucker::rcgen::KeyPair;

use crate::env;
use crate::keychain;
use crate::primitives::{run_as_admin, sh_quote};
use crate::proxy::cert_authority;

/// Subject CN of our CA. Used both as the cert subject and as the lookup
/// key for trust/untrust via `security`.
pub const CA_COMMON_NAME: &str = cert_authority::CA_COMMON_NAME;

/// The root-owned System keychain. Older builds installed CA trust here
/// (admin domain) rather than the user login keychain; a stale root left
/// behind by such an install is the one that breaks proxied HTTPS after a
/// reinstall, so we reconcile it away on the next enable.
const SYSTEM_KEYCHAIN: &str = "/Library/Keychains/System.keychain";

/// A loaded CA. The cert is public; the key is sensitive and only handed to
/// the engine (same process) to build the signing authority.
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

fn generate() -> Result<(String, String)> {
    let params = cert_authority::ca_certificate_params()?;
    let key_pair = KeyPair::generate().context("generating CA key pair")?;
    let cert = params
        .self_signed(&key_pair)
        .context("self-signing CA certificate")?;
    Ok((cert.pem(), key_pair.serialize_pem()))
}

/// Load the CA, generating + persisting one on first use. The pair is kept
/// in sync: if either half is missing we regenerate both.
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
        // Reuse the persisted pair only if its Name Constraints still permit
        // every catalog host. If the catalog grew since this CA was minted,
        // fall through to reissue so the new hosts become routable (the delete
        // below clears the stale trusted root; ensure_trusted re-installs).
        if cert_authority::ca_covers_catalog(&cert_pem) {
            return Ok(Ca { cert_pem, key_pem });
        }
    }

    // Regenerating must not leave the *old* root trusted: trust is keyed
    // by CN, so `ensure_trusted` would see the stale root and no-op while
    // every MITM handshake fails against the new CA. Best-effort delete of
    // any previous cert (and its trust settings, via -t) before persisting
    // the new pair.
    let _ = Command::new("/usr/bin/security")
        .args(["delete-certificate", "-c", CA_COMMON_NAME, "-t"])
        .arg(login_keychain()?)
        .status();

    let (cert_pem, key_pem) = generate()?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
    }
    // Order matters for crash-safety. The key (keychain) and cert (disk) are two
    // separate stores, so we can't write both atomically - but we can pick an
    // order whose interrupted state is self-healing. Write the key first (the
    // keychain set is transactional), then the cert via a temp file + atomic
    // rename. If the process dies between the two, the next launch finds a key
    // with no cert on disk and regenerates both - never a torn cert or a
    // mismatched cert/key pair that would wedge the engine with no recovery.
    keychain::set(&service, &user, &key_pem)?;
    let tmp = path.with_extension("pem.tmp");
    fs::write(&tmp, &cert_pem).with_context(|| format!("writing {}", tmp.display()))?;
    fs::rename(&tmp, &path)
        .with_context(|| format!("renaming {} -> {}", tmp.display(), path.display()))?;
    Ok(Ca { cert_pem, key_pem })
}

/// Whether the **current on-disk CA** is trusted as a root - keyed on the
/// actual certificate, not just its name. We verify the cert against the
/// system trust store with `security verify-cert`, which evaluates the real
/// cert bytes across the user and admin trust domains and exits non-zero
/// (`CSSMERR_TP_NOT_TRUSTED`) when it doesn't anchor to a trusted root.
///
/// The previous check matched the CN in `dump-trust-settings`, which a stale
/// root from a prior install satisfies - it shares our CN but has a different
/// key/fingerprint. That made `ensure_trusted` no-op while the engine signed
/// leaves with a *different*, untrusted CA, so every MITM handshake failed
/// (`NOT_TRUSTED`) with no recovery short of manual keychain surgery - the
/// reinstall-breaks-the-proxy bug. Verifying the cert itself catches the
/// fingerprint mismatch and lets `ensure_trusted` re-install. Read-only /
/// non-privileged.
pub fn is_trusted() -> Result<bool> {
    let cert = cert_path()?;
    if !cert.exists() {
        return Ok(false);
    }
    let out = Command::new("/usr/bin/security")
        .arg("verify-cert")
        // Evaluate against the SSL policy so this matches the scope the trust
        // setting is installed with (`ensure_trusted` uses `-p ssl`). A basic
        // (unscoped) evaluation would not see a policy-scoped trust setting and
        // would report the CA as untrusted, re-prompting on every launch.
        .args(["-p", "ssl"])
        .arg("-c")
        .arg(&cert)
        .output()
        .context("running security verify-cert")?;
    Ok(out.status.success())
}

/// The user's login keychain, where we install CA trust (no root required).
fn login_keychain() -> Result<PathBuf> {
    Ok(env::home()?.join("Library/Keychains/login.keychain-db"))
}

/// Whether a cert with our CN is sitting in the root-owned System keychain.
/// Reading is non-privileged, so we use this to gate the (privileged) System
/// keychain cleanup and avoid prompting for admin when there's nothing stale
/// to remove (e.g. every normal first-time enable).
fn system_keychain_has_ca() -> Result<bool> {
    let out = Command::new("/usr/bin/security")
        .args(["find-certificate", "-c", CA_COMMON_NAME])
        .arg(SYSTEM_KEYCHAIN)
        .output()
        .context("running security find-certificate")?;
    Ok(out.status.success())
}

/// Remove any Gate CA a prior install left behind before we install the
/// current one. We always drop it from the user login keychain (no admin),
/// and - only if one is actually present - from the root-owned System
/// keychain via a single admin prompt. The login-keychain delete uses `-t`
/// to drop the cert's user-domain trust settings with it and is best-effort
/// (a missing cert just exits non-zero). The System-keychain delete must NOT
/// use `-t`: removing admin-domain trust settings needs the interactive
/// Security-Agent authorization that the osascript "with administrator
/// privileges" context cannot provide (same limitation as `add-trusted-cert`,
/// see module docs), so `-t` makes the whole cleanup fail even after the user
/// enters a correct password. Deleting just the cert is enough - an orphaned
/// admin trust entry for a cert that's no longer in any keychain is inert.
/// The System-keychain removal surfaces a real failure because it's gated
/// behind the admin prompt the user just accepted.
fn remove_stale_cas() -> Result<()> {
    let _ = Command::new("/usr/bin/security")
        .args(["delete-certificate", "-c", CA_COMMON_NAME, "-t"])
        .arg(login_keychain()?)
        .status();

    if system_keychain_has_ca()? {
        let script = format!(
            "/usr/bin/security delete-certificate -c {cn} {kc}",
            cn = sh_quote(CA_COMMON_NAME),
            kc = sh_quote(SYSTEM_KEYCHAIN),
        );
        run_as_admin(&script).context("removing a stale proxy CA from the System keychain")?;
    }
    Ok(())
}

/// Trust the CA if it isn't already. Runs `security add-trusted-cert`
/// *directly* (not via osascript) so its native Security-Agent dialog can
/// perform the interactive trust-settings authorization that
/// `SecTrustSettingsSetTrustSettings` requires - the "with administrator
/// privileges" path fails with "authorization denied since no user
/// interaction was possible". Installs into the user login keychain (user
/// trust domain), which needs no root.
pub fn ensure_trusted() -> Result<()> {
    if is_trusted()? {
        return Ok(());
    }
    // Not trusted, but a prior install may have left a stale root with our CN
    // (different key) still in a keychain. Adding ours alongside it leaves two
    // same-CN roots; worse, the stale one was what shadowed the real fix.
    // Drop any leftover Gate CA before installing the current one.
    remove_stale_cas()?;
    let cert = cert_path()?;
    let keychain = login_keychain()?;
    let status = Command::new("/usr/bin/security")
        .arg("add-trusted-cert")
        // `-p ssl` scopes the trust setting to TLS server evaluation instead
        // of every policy (S/MIME, code signing, ...). The proxy only ever
        // needs this root for the MITM TLS leg, so the anchor should not be
        // trusted for anything else. `is_trusted` verifies against the same
        // `-p ssl` policy.
        .args(["-r", "trustRoot", "-p", "ssl", "-k"])
        .arg(&keychain)
        .arg(&cert)
        .status()
        .context("running security add-trusted-cert")?;
    if !status.success() {
        anyhow::bail!(
            "couldn't trust the proxy CA \u{2014} the certificate trust dialog was cancelled or denied"
        );
    }
    Ok(())
}

/// Remove the CA's trust and its key material. Runs `security
/// remove-trusted-cert` directly so its native authorization dialog appears,
/// then tears down the private key and public cert so an explicit "remove"
/// leaves nothing behind.
pub fn untrust() -> Result<()> {
    if is_trusted()? {
        let cert = cert_path()?;
        let status = Command::new("/usr/bin/security")
            .arg("remove-trusted-cert")
            .arg(&cert)
            .status()
            .context("running security remove-trusted-cert")?;
        if !status.success() {
            anyhow::bail!("couldn't untrust the proxy CA (security remove-trusted-cert failed)");
        }
    }
    remove_ca_material()
}

/// Full teardown for an explicit removal: drop the private key from the
/// keychain and the public cert from disk, so "remove" clears the MITM
/// material rather than only the trust setting. Best-effort on the key (a
/// missing entry is fine) and on an absent cert file.
fn remove_ca_material() -> Result<()> {
    let _ = keychain::delete(&key_service(), &env::current_user()?);
    let cert = cert_path()?;
    match fs::remove_file(&cert) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e).with_context(|| format!("removing {}", cert.display())),
    }
}
