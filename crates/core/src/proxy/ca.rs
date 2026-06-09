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
use hudsucker::rcgen::{
    BasicConstraints, CertificateParams, DnType, IsCa, KeyPair, KeyUsagePurpose,
};

use crate::env;
use crate::keychain;

/// Subject CN of our CA. Used both as the cert subject and as the lookup
/// key for trust/untrust via `security`.
pub const CA_COMMON_NAME: &str = "Gate Connect Local CA";

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
    let mut params =
        CertificateParams::new(Vec::<String>::new()).context("building CA certificate params")?;
    params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    params
        .distinguished_name
        .push(DnType::CommonName, CA_COMMON_NAME);
    params
        .distinguished_name
        .push(DnType::OrganizationName, "Constellation Gate");
    params.key_usages = vec![
        KeyUsagePurpose::KeyCertSign,
        KeyUsagePurpose::CrlSign,
        KeyUsagePurpose::DigitalSignature,
    ];

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
        return Ok(Ca { cert_pem, key_pem });
    }

    let (cert_pem, key_pem) = generate()?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
    }
    // Order matters for crash-safety. The key (keychain) and cert (disk) are two
    // separate stores, so we can't write both atomically — but we can pick an
    // order whose interrupted state is self-healing. Write the key first (the
    // keychain set is transactional), then the cert via a temp file + atomic
    // rename. If the process dies between the two, the next launch finds a key
    // with no cert on disk and regenerates both — never a torn cert or a
    // mismatched cert/key pair that would wedge the engine with no recovery.
    keychain::set(&service, &user, &key_pem)?;
    let tmp = path.with_extension("pem.tmp");
    fs::write(&tmp, &cert_pem).with_context(|| format!("writing {}", tmp.display()))?;
    fs::rename(&tmp, &path)
        .with_context(|| format!("renaming {} -> {}", tmp.display(), path.display()))?;
    Ok(Ca { cert_pem, key_pem })
}

/// Whether our CA is **trusted** as a root — not merely present in a
/// keychain. We install trust into the user login keychain (which surfaces in
/// `dump-trust-settings`); we also check the admin domain in case it was
/// trusted there manually. A cert can sit in a keychain with no trust setting
/// at all, and the old presence-only check (`find-certificate`) reported
/// success in exactly that untrusted state, so `ensure_trusted` skipped the
/// install and every MITM handshake failed (`CertificateUnknown` /
/// `NOT_TRUSTED`). Reading trust settings is non-privileged.
pub fn is_trusted() -> Result<bool> {
    for admin_domain in [false, true] {
        let mut cmd = Command::new("/usr/bin/security");
        cmd.arg("dump-trust-settings");
        if admin_domain {
            cmd.arg("-d");
        }
        let out = cmd.output().context("running security dump-trust-settings")?;
        // It lists each trusted cert by label (ours is the CN) and exits
        // non-zero when a domain has no trust settings, so key off the output.
        if String::from_utf8_lossy(&out.stdout).contains(CA_COMMON_NAME) {
            return Ok(true);
        }
    }
    Ok(false)
}

/// The user's login keychain, where we install CA trust (no root required).
fn login_keychain() -> Result<PathBuf> {
    Ok(env::home()?.join("Library/Keychains/login.keychain-db"))
}

/// Trust the CA if it isn't already. Runs `security add-trusted-cert`
/// *directly* (not via osascript) so its native Security-Agent dialog can
/// perform the interactive trust-settings authorization that
/// `SecTrustSettingsSetTrustSettings` requires — the "with administrator
/// privileges" path fails with "authorization denied since no user
/// interaction was possible". Installs into the user login keychain (user
/// trust domain), which needs no root.
pub fn ensure_trusted() -> Result<()> {
    if is_trusted()? {
        return Ok(());
    }
    let cert = cert_path()?;
    let keychain = login_keychain()?;
    let status = Command::new("/usr/bin/security")
        .arg("add-trusted-cert")
        .args(["-r", "trustRoot", "-k"])
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

/// Remove the CA's trust. Runs `security remove-trusted-cert` directly so its
/// native authorization dialog appears.
pub fn untrust() -> Result<()> {
    if !is_trusted()? {
        return Ok(());
    }
    let cert = cert_path()?;
    let status = Command::new("/usr/bin/security")
        .arg("remove-trusted-cert")
        .arg(&cert)
        .status()
        .context("running security remove-trusted-cert")?;
    if !status.success() {
        anyhow::bail!("couldn't untrust the proxy CA (security remove-trusted-cert failed)");
    }
    Ok(())
}
