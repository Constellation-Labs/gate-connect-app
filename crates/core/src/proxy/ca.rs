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
//!
//! [`ensure_trusted_system`] is the exception, and the one path that does not
//! prompt: an opt-in, CLI-only install into the **admin** trust domain, for
//! machines with nobody to answer a dialog (build agents, headless Macs). It is
//! never the default and never reachable from the GUI - the dialog is deliberate
//! product behaviour, not an obstacle.

use std::fs;
use std::path::PathBuf;
use std::process::Command;

use anyhow::{Context, Result};
use hudsucker::rcgen::KeyPair;

use crate::env;
use crate::keychain;
use crate::primitives::{run_as_admin, run_as_root_noninteractive, sh_quote};
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
    // Written after the cert so an interrupted sequence leaves the sidecar
    // absent or stale — never describing a cert that isn't there yet.
    cert_authority::write_host_fingerprint(&path)?;
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
///
/// The seam comes first: a login keychain is a property of the OS session
/// rather than of `$HOME`, so a harness that redirects home must still be able
/// to name the session's real one. See [`env::test_login_keychain`].
fn login_keychain() -> Result<PathBuf> {
    if let Some(path) = env::test_login_keychain() {
        return Ok(path);
    }
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
    remove_login_keychain_ca();

    if system_keychain_has_ca()? {
        run_as_admin(&system_trust_remove_script())
            .context("removing a stale proxy CA from the System keychain")?;
    }
    Ok(())
}

/// Best-effort delete of a Gate CA (and its user-domain trust settings, via
/// `-t`) from the login keychain. Non-privileged and never prompts, so both the
/// interactive trust path and the headless one can call it; a missing cert just
/// exits non-zero.
fn remove_login_keychain_ca() {
    let Ok(keychain) = login_keychain() else {
        return;
    };
    let _ = Command::new("/usr/bin/security")
        .args(["delete-certificate", "-c", CA_COMMON_NAME, "-t"])
        .arg(keychain)
        .status();
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
    let out = Command::new("/usr/bin/security")
        .arg("add-trusted-cert")
        // `-p ssl` scopes the trust setting to TLS server evaluation instead
        // of every policy (S/MIME, code signing, ...). The proxy only ever
        // needs this root for the MITM TLS leg, so the anchor should not be
        // trusted for anything else. `is_trusted` verifies against the same
        // `-p ssl` policy.
        .args(["-r", "trustRoot", "-p", "ssl", "-k"])
        .arg(&keychain)
        .arg(&cert)
        .output()
        .context("running security add-trusted-cert")?;
    if !out.status.success() {
        // Say what `security` said. This used to report the cancelled/denied
        // dialog as fact for *any* non-zero exit, which is a cause it cannot
        // know: on CI the real failure was "SecCertificateAddToKeychain: The
        // specified keychain could not be found" - printed by the child, then
        // contradicted by this line - and a user on a managed Mac or with a
        // damaged login keychain would be told they declined a prompt that was
        // never shown. Cancelling is still the likeliest cause when the tool
        // says nothing, so it stays as the hint for that case only.
        let detail = String::from_utf8_lossy(&out.stderr);
        let detail = detail.trim();
        if detail.is_empty() {
            anyhow::bail!(
                "couldn't trust the proxy CA \u{2014} the certificate trust dialog was cancelled or denied"
            );
        }
        anyhow::bail!(
            "couldn't trust the proxy CA in {}: {detail}",
            keychain.display()
        );
    }
    Ok(())
}

/// Remove the CA's trust and its key material. Runs `security
/// remove-trusted-cert` directly so its native authorization dialog appears,
/// then tears down the private key and public cert so an explicit "remove"
/// leaves nothing behind.
///
/// One thing it cannot remove: a machine-wide root installed by
/// [`ensure_trusted_system`]. That lives in the root-owned System keychain, and
/// this path is the one the GUI runs, where escalating for a state only a CLI
/// flag can create would be a surprise. So it finishes everything it can and
/// then *says* what is left rather than reporting a clean removal over a root
/// that is still trusted for every user on the box.
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
    // Read before the teardown: `find-certificate` is keyed on the CN, not on
    // the cert file, but checking first keeps the report true even if a future
    // change makes it content-keyed.
    let machine_wide = system_keychain_has_ca().unwrap_or(false);
    remove_ca_material()?;
    if machine_wide {
        anyhow::bail!(
            "the per-user trust and the CA's key material are gone, but a machine-wide copy of the CA is still in {SYSTEM_KEYCHAIN} (installed with --system-trust). Remove it with `gate-connect proxy untrust-ca --system-trust` as root."
        );
    }
    Ok(())
}

/// Install the CA machine-wide, without any dialog. The headless counterpart of
/// [`ensure_trusted`], reached only from `proxy trust-ca --system-trust`.
///
/// `-d` writes the **admin** trust domain (and `-k` the root-owned System
/// keychain) instead of the user domain, which is the whole trick: the
/// Security-Agent dialog exists to authorize a user-domain trust-settings
/// write, and root in the admin domain needs no such authorization.
///
/// Measured on GitHub's macOS runner rather than assumed - this exact command
/// is what `ci/e2e/run.sh` hand-rolled before this flag existed, and it returns
/// in about a second with no prompt, after which `security verify-cert -p ssl`
/// is satisfied. That last part is why the flag is worth anything: [`is_trusted`]
/// is a *policy* evaluation, so it honours admin-domain trust, and a later
/// `enable` short-circuits instead of reaching the dialog.
///
/// What it widens: the CA becomes a trusted TLS root for **every user on this
/// machine**, where [`ensure_trusted`] deliberately stays per-user. The `-r
/// trustRoot -p ssl` scoping is carried over from the interactive path, so it
/// stays an anchor for TLS server evaluation only, not for S/MIME or code
/// signing. The `is_trusted` check before returning is the guard against a
/// silent mismatch between what we install and what the policy evaluation
/// accepts: it fails here, loudly, rather than resurfacing as a dialog on the
/// next enable.
pub fn ensure_trusted_system() -> Result<()> {
    if is_trusted()? {
        return Ok(());
    }
    // Same reasoning as `ensure_trusted`: a stale same-CN root from a prior
    // install would be stacked alongside ours. Both deletes are best-effort -
    // a missing cert exits non-zero, and if escalation is unavailable the
    // install below is what reports it, with the actionable message.
    remove_login_keychain_ca();
    if system_keychain_has_ca().unwrap_or(false) {
        let _ = run_as_root_noninteractive(&system_trust_remove_script());
    }
    let cert = cert_path()?;
    run_as_root_noninteractive(&system_trust_install_script(&cert))
        .context("installing the proxy CA machine-wide")?;
    // Trust that does not satisfy our own check would leave the product
    // re-prompting after a "successful" headless install, which is the failure
    // this flag exists to remove. Verify rather than assume.
    if !is_trusted()? {
        anyhow::bail!(
            "installed the proxy CA into {SYSTEM_KEYCHAIN}, but `security verify-cert -p ssl` still rejects it"
        );
    }
    Ok(())
}

/// Remove the machine-wide CA installed by [`ensure_trusted_system`], then the
/// key material. The headless counterpart of [`untrust`].
///
/// It deletes the *certificate* from the System keychain rather than the
/// admin-domain trust *setting*: `remove-trusted-cert -d` needs the same
/// interactive Security-Agent authorization that `add-trusted-cert` needs in the
/// user domain, so there is no promptless way to unset it. Deleting the cert is
/// enough - `verify-cert` then finds nothing to anchor to, so [`is_trusted`]
/// goes false, and an admin trust entry naming a cert that is in no keychain is
/// inert (the same asymmetry `remove_stale_cas` documents).
pub fn untrust_system() -> Result<()> {
    if system_keychain_has_ca()? {
        run_as_root_noninteractive(&system_trust_remove_script())
            .context("removing the machine-wide proxy CA")?;
    }
    // A desktop Mac can hold both installs at once; the per-user half needs the
    // dialog, so this path can only name it. Read before the teardown, which
    // deletes the cert file `verify-cert` reads.
    let per_user_left = is_trusted().unwrap_or(false);
    remove_ca_material()?;
    if per_user_left {
        anyhow::bail!(
            "the machine-wide CA and its key material are gone, but this login still carries a per-user trust setting for it. Removing that needs the certificate dialog: run `gate-connect proxy untrust-ca`."
        );
    }
    Ok(())
}

/// The privileged install, as a shell command for [`run_as_root_noninteractive`].
/// Pure so the flags can be tested without touching a trust store.
fn system_trust_install_script(cert: &std::path::Path) -> String {
    format!(
        "/usr/bin/security add-trusted-cert -d -r trustRoot -p ssl -k {kc} {cert}",
        kc = sh_quote(SYSTEM_KEYCHAIN),
        cert = sh_quote(&cert.display().to_string()),
    )
}

/// The privileged removal, as a shell command. No `-t`: dropping admin-domain
/// trust settings needs interactive authorization, and it would make the whole
/// command fail (see [`untrust_system`]).
fn system_trust_remove_script() -> String {
    format!(
        "/usr/bin/security delete-certificate -c {cn} {kc}",
        cn = sh_quote(CA_COMMON_NAME),
        kc = sh_quote(SYSTEM_KEYCHAIN),
    )
}

/// Full teardown for an explicit removal: drop the private key from the
/// keychain and the public cert from disk, so "remove" clears the MITM
/// material rather than only the trust setting. Best-effort on the key (a
/// missing entry is fine) and on an absent cert file.
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

#[cfg(test)]
mod tests {
    use super::*;

    /// The flags are the feature here: `-d` is what makes the install
    /// promptless, and dropping `-p ssl` would quietly widen a machine-wide
    /// anchor to every trust policy. Asserted on the command string because the
    /// install itself needs root and a real trust store.
    #[test]
    fn the_machine_wide_install_targets_the_admin_domain_and_stays_ssl_only() {
        let script = system_trust_install_script(std::path::Path::new("/tmp/ca cert.pem"));
        assert!(script.contains(" -d "), "{script}");
        assert!(script.contains("-r trustRoot"), "{script}");
        assert!(script.contains("-p ssl"), "{script}");
        assert!(script.contains(SYSTEM_KEYCHAIN), "{script}");
        // A path with a space must survive the trip through `sh -c`.
        assert!(script.contains("'/tmp/ca cert.pem'"), "{script}");
    }

    /// `-t` would ask for the admin-domain trust setting to be unset, which
    /// needs interactive authorization and fails the whole command - the
    /// asymmetry `untrust_system` is built around.
    #[test]
    fn the_machine_wide_removal_deletes_the_cert_without_touching_trust_settings() {
        let script = system_trust_remove_script();
        assert!(script.contains("delete-certificate"), "{script}");
        assert!(script.contains(CA_COMMON_NAME), "{script}");
        assert!(script.contains(SYSTEM_KEYCHAIN), "{script}");
        assert!(!script.contains(" -t"), "{script}");
    }
}
