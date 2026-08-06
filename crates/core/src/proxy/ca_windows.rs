//! Local root CA for the proxy (Windows). We generate a CA once, keep its
//! private key in the Windows Credential Manager (via [`crate::keychain`]) and
//! its public cert on disk, and (on enable) install the cert into the current
//! user's "Trusted Root Certification Authorities" store so the MITM engine can
//! mint per-host leaf certs the OS trust store will accept.
//!
//! Trust is installed with `certutil -user -addstore Root`, which targets the
//! per-user root store (`HKCU`) and needs no admin - but Windows still shows
//! its native "you are about to install a certificate from a certification
//! authority claiming to represent…" confirmation dialog. That dialog is the
//! reassuring gatekeeper prompt the user sees once on enable; declining it
//! makes `certutil` exit non-zero.
//!
//! Trust is tightly scoped: the CA only ever signs leaf certs for the handful
//! of inference hosts the user explicitly enables (every other host is
//! blind-tunnelled, never MITM'd), and the private key never leaves the
//! credential store. Windows counterpart of the macOS [`super::ca`] module.

use std::fs;
use std::os::windows::process::CommandExt;
use std::path::PathBuf;
use std::process::Command;

use anyhow::{Context, Result};
use hudsucker::rcgen::KeyPair;
use sha1::{Digest, Sha1};

use crate::env;
use crate::keychain;
use crate::proxy::cert_authority;

/// Subject CN of our CA. Used both as the cert subject and as the match token
/// for trust/untrust via `certutil`.
pub const CA_COMMON_NAME: &str = cert_authority::CA_COMMON_NAME;

/// `CREATE_NO_WINDOW`: spawn the child without allocating a console, so the
/// `certutil` calls below don't flash a black terminal window each time the
/// proxy is toggled (status/enable/disable all shell out to it).
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// `certutil` command with the console window suppressed. Used for every
/// `certutil` invocation in this module so none of them flash a terminal.
fn certutil() -> Command {
    let mut cmd = Command::new("certutil");
    cmd.creation_flags(CREATE_NO_WINDOW);
    cmd
}

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

    // Regenerating must not leave the *old* root trusted: trust is keyed
    // by CN, so `ensure_trusted` would see the stale root and no-op while
    // every MITM handshake fails against the new CA. Best-effort delete of
    // any previous cert from the per-user root store before persisting.
    let _ = certutil()
        .args(["-user", "-delstore", "Root", CA_COMMON_NAME])
        .status();

    let (cert_pem, key_pem) = generate()?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
    }
    // Write the key first (the keychain set is transactional), then the cert via
    // a temp file + atomic rename. The two stores can't be written atomically
    // together, but this order's interrupted state is self-healing: a crash
    // between them leaves a key with no cert on disk, so the next launch
    // regenerates both - never a torn cert or a mismatched cert/key pair.
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

/// SHA-1 thumbprint of the **current on-disk** CA cert, normalized to
/// uppercase hex with no separators - the form `certutil` accepts as a
/// `CertId`. Returns `None` if the cert file is missing.
///
/// A cert's thumbprint is just the SHA-1 of its DER encoding, so we compute it
/// in-process: decode the PEM to DER, then digest. We deliberately do *not*
/// scrape `certutil <file>`'s `Cert Hash(sha1):` line - that output is
/// localized and frequently emitted as UTF-16/OEM rather than UTF-8, so
/// `from_utf8_lossy` could mangle the thumbprint and make `is_trusted` report
/// an installed cert as untrusted forever (the banner never clears, and every
/// trust click re-prompts).
fn cert_thumbprint() -> Result<Option<String>> {
    let cert = cert_path()?;
    let cert_pem = match fs::read_to_string(&cert) {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e).with_context(|| format!("reading {}", cert.display())),
    };
    let der = pem::parse(cert_pem.as_bytes())
        .with_context(|| format!("parsing CA cert PEM at {}", cert.display()))?;
    let digest = Sha1::digest(der.contents());
    let thumb = digest
        .iter()
        .map(|b| format!("{b:02X}"))
        .collect::<String>();
    Ok(Some(thumb))
}

/// Whether the **current on-disk CA** is trusted - keyed on the cert's
/// thumbprint, not its name. We look the cert's SHA-1 thumbprint up in the
/// per-user root store (`certutil -user -store Root <thumbprint>`), which
/// exits 0 only when a cert with that exact thumbprint is present.
///
/// The previous check matched the CN, which a stale root from a prior install
/// satisfies - it shares our CN but has a different key/fingerprint. That made
/// `ensure_trusted` no-op while the engine signed leaves with a *different*,
/// untrusted CA, so every MITM handshake failed with no recovery. Matching the
/// thumbprint catches the mismatch and lets `ensure_trusted` re-install.
/// Read-only / non-privileged.
pub fn is_trusted() -> Result<bool> {
    let thumb = match cert_thumbprint()? {
        Some(t) => t,
        None => return Ok(false),
    };
    let out = certutil()
        .args(["-user", "-store", "Root", &thumb])
        .output()
        .context("running certutil -store Root")?;
    Ok(out.status.success())
}

/// Trust the CA if it isn't already. `certutil -user -addstore Root` installs
/// the cert into the per-user root store (no admin) and triggers Windows'
/// native trust-confirmation dialog. Cancelling/declining the dialog makes
/// certutil exit non-zero.
pub fn ensure_trusted() -> Result<()> {
    if is_trusted()? {
        return Ok(());
    }
    // A prior install may have left a same-CN root (different key) in the
    // per-user store; drop it so we don't stack a duplicate before adding the
    // current cert. Best-effort - a missing cert just makes this a no-op.
    let _ = certutil()
        .args(["-user", "-delstore", "Root", CA_COMMON_NAME])
        .status();
    let cert = cert_path()?;
    let status = certutil()
        .args(["-user", "-addstore", "Root"])
        .arg(&cert)
        .status()
        .context("running certutil -addstore Root")?;
    if !status.success() {
        anyhow::bail!(
            "couldn't trust the proxy CA \u{2014} the certificate trust dialog was cancelled or denied"
        );
    }
    Ok(())
}

/// Remove the CA's trust and its key material. `certutil -user -delstore Root
/// <CN>` deletes our cert from the per-user root store by common name; then the
/// private key and public cert are torn down so an explicit "remove" leaves
/// nothing behind.
pub fn untrust() -> Result<()> {
    if is_trusted()? {
        let status = certutil()
            .args(["-user", "-delstore", "Root", CA_COMMON_NAME])
            .status()
            .context("running certutil -delstore Root")?;
        if !status.success() {
            anyhow::bail!("couldn't untrust the proxy CA (certutil -delstore Root failed)");
        }
    }
    remove_ca_material()
}

/// Full teardown for an explicit removal: drop the private key from the
/// certificate store and the public cert from disk, so "remove" clears the
/// MITM material rather than only the trust setting. Best-effort on the key (a
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
