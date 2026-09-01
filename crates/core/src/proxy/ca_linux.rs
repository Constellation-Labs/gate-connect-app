//! Local root CA for the proxy (Linux). We generate a CA once, keep its private
//! key in the OS secret store (via [`crate::keychain`] → Secret Service) and its
//! public cert on disk, and (on enable) install the cert into the **system**
//! trust store so the MITM engine can mint per-host leaf certs that the OS - and
//! command-line tools that read the system bundle (curl, git, openssl) - accept.
//!
//! Unlike macOS/Windows, Linux has no per-user root store *for the OS*: trust is
//! system-wide and the install needs root. We support the two common layouts:
//!
//! - Debian/Ubuntu/Arch: drop the PEM in `/usr/local/share/ca-certificates/`
//!   and run `update-ca-certificates`.
//! - Fedora/RHEL/openSUSE: drop it in `/etc/pki/ca-trust/source/anchors/` and
//!   run `update-ca-trust extract`.
//!
//! The system store is not the whole job, though. Chromium-based browsers on
//! Linux never read it - they use their own built-in roots plus a per-user NSS
//! database at `~/.pki/nssdb` - so a system-only install leaves Chrome and
//! Chromium failing every intercepted host with `ERR_CERT_AUTHORITY_INVALID`
//! while Firefox works, because Firefox picks the system anchors up through
//! p11-kit. So [`ensure_trusted`] writes that database too, unprivileged and
//! best-effort, via `certutil`.
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
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Context, Result};
use hudsucker::rcgen::KeyPair;

use crate::env;
use crate::keychain;
use crate::primitives::{run_as_admin, run_as_root_noninteractive, sh_quote};
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
    if !is_trusted()? {
        let store = trust_store()?;
        run_as_admin(&anchor_install_script(&store, &cert_path()?))
            .context("installing the proxy CA into the system trust store")?;
    }
    // Deliberately outside the `is_trusted` short-circuit above. That check
    // reads the system anchor and nothing else, so every machine whose anchor
    // is already current - which includes every install predating this call -
    // would otherwise never reach the NSS store, and Chromium would keep
    // rejecting intercepted hosts with no way to recover short of removing and
    // re-adding trust.
    ensure_trusted_nss();
    Ok(())
}

/// Trust the CA machine-wide **without any prompt**, for hosts where nobody can
/// answer one (build agents, containers, headless servers). Reached only from
/// `proxy trust-ca --system-trust`.
///
/// Linux has no per-user root store, so this installs the same anchor in the
/// same place as [`ensure_trusted`] - the only difference is the escalation.
/// [`run_as_admin`] picks `sudo` when stdout is a tty and `pkexec` otherwise, so
/// a script with redirected output on a host with no polkit agent fails at the
/// authentication agent rather than at anything to do with certificates (the
/// workaround being to hand it a pty). `run_as_root_noninteractive` needs
/// neither: it runs the command directly when already root and via `sudo -n`
/// otherwise, and turns "would have prompted" into an error that says so.
pub fn ensure_trusted_system() -> Result<()> {
    if is_trusted()? {
        return Ok(());
    }
    let store = trust_store()?;
    run_as_root_noninteractive(&anchor_install_script(&store, &cert_path()?))
        .context("installing the proxy CA into the system trust store")?;
    Ok(())
}

/// Remove the CA's trust: delete our anchor file and rebuild the bundle so the
/// cert drops out of it. Privileged. Keyed on the anchor *existing*, not on
/// `is_trusted` - a stale anchor left by a regenerated pair must still be
/// removable.
pub fn untrust() -> Result<()> {
    let store = trust_store()?;
    if store.anchor.exists() {
        run_as_admin(&anchor_remove_script(&store))
            .context("removing the proxy CA from the system trust store")?;
    }
    untrust_nss();
    remove_ca_material()
}

/// Remove the CA's trust without a prompt. The headless counterpart of
/// [`untrust`]: same anchor, same rebuild, non-interactive escalation.
pub fn untrust_system() -> Result<()> {
    let store = trust_store()?;
    if store.anchor.exists() {
        run_as_root_noninteractive(&anchor_remove_script(&store))
            .context("removing the proxy CA from the system trust store")?;
    }
    remove_ca_material()
}

/// Install the anchor and rebuild the bundle, as one shell command so a single
/// escalation covers both. Pure, so the interactive and headless callers cannot
/// drift and the shape is testable without root or a trust store.
fn anchor_install_script(store: &TrustStore, cert: &std::path::Path) -> String {
    // The anchor is a filename joined onto its directory, so `parent` is always
    // Some; falling back to the anchor itself keeps this total rather than
    // introducing an error case no input can reach.
    let parent = store.anchor.parent().unwrap_or(&store.anchor);
    format!(
        "/bin/mkdir -p {parent} && /usr/bin/install -m 0644 {src} {dst} && {update}",
        parent = sh_quote(&parent.display().to_string()),
        src = sh_quote(&cert.display().to_string()),
        dst = sh_quote(&store.anchor.display().to_string()),
        update = store.install_cmd,
    )
}

/// Delete the anchor and rebuild the bundle, as one shell command.
fn anchor_remove_script(store: &TrustStore) -> String {
    format!(
        "/bin/rm -f {dst} && {refresh}",
        dst = sh_quote(&store.anchor.display().to_string()),
        refresh = store.refresh_cmd,
    )
}

/// What to tell the user when `certutil` is not installed. Naming the package
/// matters: without it the message is a bare "no such file" for a binary most
/// people have never heard of, attached to a browser failure that looks like a
/// certificate bug.
///
/// The `.deb` depends on it (`src-tauri/tauri.conf.json`), which is where nearly
/// every Linux install comes from, so this is for the ones that route around
/// packaging: the AppImage, a hand-built tarball, `cargo run`.
const NSS_TOOLS_HINT: &str =
    "install certutil (Debian/Ubuntu: libnss3-tools, Fedora/RHEL: nss-tools) and retry";

/// Every per-user NSS database a Chromium-based browser might read user-added
/// roots from, whether or not it exists. Pure, and split from [`nss_db_dirs`]
/// so the path set is testable without a browser installed.
///
/// Chromium on Linux does not consult the system CA bundle at all: it uses its
/// own built-in root store plus this database. So the system anchor the rest of
/// this module installs leaves every Chromium browser failing the handshake on
/// intercepted hosts with `ERR_CERT_AUTHORITY_INVALID`, while Firefox works,
/// because Firefox picks the same system anchors up through p11-kit. That
/// asymmetry is the whole reason this exists.
///
/// Enumerated rather than globbed (`~/.var/app/*/.pki/nssdb`) on purpose: a glob
/// would hand our signing root to every confined app that happens to keep an NSS
/// database, browser or not, and the trust here is meant to stay narrow. The
/// cost is that a Chromium-family browser missing from this list fails exactly
/// the way the bug did, so a new one belongs here.
fn nss_db_candidates(home: &Path) -> Vec<PathBuf> {
    [
        // Distro packages (.deb/.rpm) and anything else running with the real
        // HOME, which is the common case for every one of these browsers.
        ".pki/nssdb",
        // Snap and Flatpak confine the browser to a HOME of their own, so the
        // database is not the one above and each has to be named separately.
        "snap/chromium/current/.pki/nssdb",
        "snap/brave/current/.pki/nssdb",
        ".var/app/org.chromium.Chromium/.pki/nssdb",
        ".var/app/com.google.Chrome/.pki/nssdb",
        ".var/app/com.google.ChromeDev/.pki/nssdb",
        ".var/app/com.brave.Browser/.pki/nssdb",
        ".var/app/com.microsoft.Edge/.pki/nssdb",
        ".var/app/com.vivaldi.Vivaldi/.pki/nssdb",
    ]
    .iter()
    .map(|rel| home.join(rel))
    .collect()
}

/// The subset of [`nss_db_candidates`] that exists. Empty when no Chromium
/// browser has ever run for this user - the database is created on first
/// launch, so there is nothing to trust into and nothing to warn about.
fn nss_db_dirs() -> Vec<PathBuf> {
    let Some(home) = std::env::var_os("HOME").map(PathBuf::from) else {
        return Vec::new();
    };
    nss_db_candidates(&home)
        .into_iter()
        .filter(|dir| dir.is_dir())
        .collect()
}

/// Why a `certutil` call did not succeed. The missing-binary case is split out
/// because it is the only one [`NSS_TOOLS_HINT`] answers: telling someone to
/// install a package they already have, because their database was locked,
/// sends them the wrong way at the one moment they are reading closely.
enum CertutilFailure {
    /// `certutil` is not installed.
    Missing,
    /// It ran and refused, or could not be run for some other reason.
    Failed(String),
}

impl CertutilFailure {
    /// [`NSS_TOOLS_HINT`], spliced ready for the tail of a message - empty for
    /// every failure a package would not fix.
    fn tools_hint(&self) -> String {
        match self {
            Self::Missing => format!(" - {NSS_TOOLS_HINT}"),
            Self::Failed(_) => String::new(),
        }
    }
}

impl std::fmt::Display for CertutilFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Missing => write!(f, "certutil is not installed"),
            Self::Failed(msg) => write!(f, "{msg}"),
        }
    }
}

/// One `certutil` invocation against a database, returning its stdout.
/// Unprivileged by construction: these are per-user stores, and running them
/// under the escalation the system anchor needs would write into root's HOME
/// instead of the user's.
fn certutil_output(db: &Path, args: &[&str]) -> std::result::Result<String, CertutilFailure> {
    let out = Command::new("certutil")
        .arg("-d")
        // `sql:` selects the modern cert9.db format. Chromium has written that
        // format for years, and naming it explicitly avoids certutil falling
        // back to the legacy cert8.db pair on an empty directory.
        .arg(format!("sql:{}", db.display()))
        .args(args)
        // A database with a password set makes certutil prompt for it on stdin.
        // Under the GUI that reads EOF, but the CLI would hand it the user's
        // terminal and block there, so close it and let the call fail instead.
        .stdin(Stdio::null())
        .output();
    let out = match out {
        Ok(out) => out,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Err(CertutilFailure::Missing),
        Err(e) => return Err(CertutilFailure::Failed(format!("running certutil: {e}"))),
    };
    if !out.status.success() {
        return Err(CertutilFailure::Failed(format!(
            "certutil {} exited {}: {}",
            args.join(" "),
            out.status,
            String::from_utf8_lossy(&out.stderr).trim()
        )));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// [`certutil_output`] where only success matters.
fn certutil(db: &Path, args: &[&str]) -> std::result::Result<(), CertutilFailure> {
    certutil_output(db, args).map(|_| ())
}

/// The PEM certutil holds under our nickname in `db`, or `None` when there is no
/// such entry - and also when there is no `certutil` to ask with, which callers
/// that care about the difference have to read from [`certutil_output`] instead.
fn nss_entry_pem(db: &Path) -> Option<String> {
    certutil_output(db, &["-L", "-n", CA_COMMON_NAME, "-a"]).ok()
}

/// Whether `db` already holds exactly the certificate in `pem` under our
/// nickname. False when it holds something else, holds nothing, or when we
/// cannot ask at all - all three want the same next move.
fn nss_holds(db: &Path, pem: &str) -> bool {
    nss_entry_pem(db).is_some_and(|held| pem_body(&held) == pem_body(pem))
}

/// Whether every per-user NSS database found holds our *current* CA, for the
/// diagnostics report. `Some(false)` beside a `ca_trusted` of true is the state
/// this module learned the hard way: the OS trusts the root, Chromium does not,
/// and only Chromium-based browsers fail. A missing `certutil` reads as false,
/// which is accurate - without it [`ensure_trusted_nss`] never installed
/// anything.
///
/// `None` where the question does not apply: no Chromium browser has ever run
/// for this user, so there is no database to be in. Also `None` when the cert
/// itself cannot be read, which `ca_cert_present` already reports.
pub fn nss_ca_trusted() -> Option<bool> {
    let dirs = nss_db_dirs();
    if dirs.is_empty() {
        return None;
    }
    let pem = cert_path().ok().and_then(|p| fs::read_to_string(p).ok())?;
    Some(dirs.iter().all(|dir| nss_holds(dir, &pem)))
}

/// The base64 payload of a PEM block, with the armour and all whitespace
/// dropped. Comparing this rather than the raw text lets an NSS export and our
/// own file on disk be recognised as the same certificate despite differing
/// line wrapping, line endings, or trailing newline.
fn pem_body(pem: &str) -> String {
    pem.lines()
        .filter(|line| !line.starts_with("-----"))
        .flat_map(|line| line.chars())
        .filter(|c| !c.is_whitespace())
        .collect()
}

/// Add the CA to every per-user NSS database found, so Chromium accepts the
/// leaves the engine mints.
///
/// Best-effort and infallible by design: the system anchor is what trust really
/// rests on, and a browser-specific store that cannot be written must not fail
/// enabling the proxy. Failures are reported rather than swallowed, because the
/// symptom otherwise lands in the browser as a certificate error with nothing
/// connecting it to Gate.
fn ensure_trusted_nss() {
    let dirs = nss_db_dirs();
    if dirs.is_empty() {
        return;
    }
    // Read the cert before touching any database. The add hands certutil the
    // same file through `-i`, so a cert we cannot read is a rewrite that fails
    // on every store - after the delete below has already landed on each.
    let (cert_arg, cert_pem) = match cert_path().and_then(|path| {
        let pem =
            fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
        Ok((path.display().to_string(), pem))
    }) {
        Ok(cert) => cert,
        Err(e) => {
            eprintln!("gate proxy: no readable CA cert for the NSS trust store ({e})");
            return;
        }
    };
    for dir in dirs {
        // Nothing to do where the database already holds exactly our current
        // CA, which is the steady state on every enable after the first. Worth
        // the extra call: the rewrite below is briefly destructive, and skipping
        // it keeps that window out of the common path altogether.
        if nss_holds(&dir, &cert_pem) {
            continue;
        }
        // Delete first. `certutil -A` appends under a duplicate nickname rather
        // than replacing, so a regenerated CA would leave the stale root sitting
        // in the database beside the new one, and the browser would keep
        // offering both. A missing entry fails here, harmlessly.
        let dropped = certutil(&dir, &["-D", "-n", CA_COMMON_NAME]).is_ok();
        // `-t "C,,"`: trusted to issue SSL server certs, with no S/MIME and no
        // object-signing trust. The same flags mkcert uses for the same job.
        let args = ["-A", "-t", "C,,", "-n", CA_COMMON_NAME, "-i", &cert_arg];
        if let Err(e) = certutil(&dir, &args) {
            // Say so when the delete landed and the add did not: that leaves the
            // store worse than we found it, and a browser that stopped working
            // *because* of this reads nothing like one that never worked.
            let dropped = if dropped {
                ", and the entry that was there has been dropped"
            } else {
                ""
            };
            eprintln!(
                "gate proxy: could not add the CA to the NSS store at {dir}{dropped} ({e}); \
                 Chromium-based browsers will reject intercepted hosts{hint}",
                dir = dir.display(),
                hint = e.tools_hint(),
            );
        }
    }
}

/// Drop the CA from every per-user NSS database.
///
/// Best-effort like the install, but not silent: an entry that survives an
/// explicit untrust leaves a root that can sign for any host trusted in the
/// browser while the app reports the CA removed, and that is the one failure
/// here with a security edge rather than a usability one.
///
/// Probing first keeps the ordinary "was never there" case quiet - `certutil -D`
/// fails on a nickname that is absent, and that failure is not news. A probe
/// that fails for any *other* reason reads as absent too, which is the common
/// meaning and the only one distinguishable without parsing NSS error strings;
/// a missing `certutil` is separated out, since it means we could neither look
/// nor remove and anything the install put there is still there.
fn untrust_nss() {
    for dir in nss_db_dirs() {
        match certutil_output(&dir, &["-L", "-n", CA_COMMON_NAME, "-a"]) {
            Ok(_) => {
                if let Err(e) = certutil(&dir, &["-D", "-n", CA_COMMON_NAME]) {
                    eprintln!(
                        "gate proxy: could not remove the CA from the NSS store at {dir} ({e}); \
                         Chromium-based browsers still trust it",
                        dir = dir.display(),
                    );
                }
            }
            Err(e @ CertutilFailure::Missing) => eprintln!(
                "gate proxy: could not remove the CA from the NSS store at {dir} ({e}); \
                 Chromium-based browsers may still trust it - {NSS_TOOLS_HINT}",
                dir = dir.display(),
            ),
            Err(CertutilFailure::Failed(_)) => {}
        }
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn debian_store() -> TrustStore {
        TrustStore {
            anchor: PathBuf::from("/usr/local/share/ca-certificates/Gate CA.crt"),
            install_cmd: "update-ca-certificates",
            refresh_cmd: "update-ca-certificates --fresh",
        }
    }

    /// The install is one shell command on purpose: it runs behind a single
    /// escalation, so a missing `&&` would rebuild the bundle even when the
    /// copy failed, leaving `is_trusted` false and nothing saying why.
    #[test]
    fn the_anchor_install_copies_then_rebuilds_under_one_escalation() {
        let script = anchor_install_script(&debian_store(), std::path::Path::new("/tmp/ca.pem"));
        let (copy, update) = script
            .split_once("&& update-ca-certificates")
            .expect(&script);
        assert!(
            copy.contains("/usr/bin/install -m 0644 '/tmp/ca.pem'"),
            "{script}"
        );
        assert!(
            copy.contains("/bin/mkdir -p '/usr/local/share/ca-certificates'"),
            "{script}"
        );
        assert!(update.is_empty(), "{script}");
    }

    /// The anchor filename carries the CA's common name, which has a space in
    /// it. Unquoted, `install` would see two paths and the rebuild would run
    /// over a file that was never written.
    #[test]
    fn the_anchor_path_survives_the_space_in_the_ca_name() {
        let store = debian_store();
        let install = anchor_install_script(&store, std::path::Path::new("/tmp/ca.pem"));
        let remove = anchor_remove_script(&store);
        let quoted = "'/usr/local/share/ca-certificates/Gate CA.crt'";
        assert!(install.contains(quoted), "{install}");
        assert!(remove.contains(quoted), "{remove}");
    }

    /// Removal has to rebuild with the *refresh* command: on Debian
    /// `update-ca-certificates` only adds, so dropping the anchor without
    /// `--fresh` leaves the CA in the consolidated bundle and still trusted.
    #[test]
    fn the_anchor_removal_rebuilds_the_bundle_from_scratch() {
        let script = anchor_remove_script(&debian_store());
        assert!(script.starts_with("/bin/rm -f "), "{script}");
        assert!(
            script.ends_with("&& update-ca-certificates --fresh"),
            "{script}"
        );
    }

    /// The plain `~/.pki/nssdb` is the one that matters on a distro-packaged
    /// browser, and it is the case this whole path exists to fix, so pin it
    /// rather than only asserting the list is non-empty.
    #[test]
    fn the_nss_candidates_lead_with_the_unconfined_database() {
        let dirs = nss_db_candidates(std::path::Path::new("/home/u"));
        assert_eq!(
            dirs.first().unwrap(),
            std::path::Path::new("/home/u/.pki/nssdb")
        );
    }

    /// Snap and Flatpak browsers read a database under their own confined HOME,
    /// so the unconfined path alone would silently miss them - the failure would
    /// look identical to the bug this fixes.
    #[test]
    fn the_nss_candidates_cover_the_confined_browser_homes() {
        let dirs = nss_db_candidates(std::path::Path::new("/home/u"));
        for expected in [
            "/home/u/snap/chromium/current/.pki/nssdb",
            "/home/u/snap/brave/current/.pki/nssdb",
            "/home/u/.var/app/org.chromium.Chromium/.pki/nssdb",
            "/home/u/.var/app/com.google.Chrome/.pki/nssdb",
            "/home/u/.var/app/com.google.ChromeDev/.pki/nssdb",
            "/home/u/.var/app/com.brave.Browser/.pki/nssdb",
            "/home/u/.var/app/com.microsoft.Edge/.pki/nssdb",
            "/home/u/.var/app/com.vivaldi.Vivaldi/.pki/nssdb",
        ] {
            assert!(
                dirs.iter().any(|d| d == std::path::Path::new(expected)),
                "{expected} missing from {dirs:?}"
            );
        }
    }

    /// Every candidate has to hang off the home passed in. A hardcoded `/home`
    /// or a stray absolute path would write into another user's store, which is
    /// the one outcome worse than not writing at all.
    #[test]
    fn the_nss_candidates_are_all_under_the_given_home() {
        let home = std::path::Path::new("/tmp/someone");
        for dir in nss_db_candidates(home) {
            assert!(dir.starts_with(home), "{dir:?} escaped {home:?}");
        }
    }

    /// The "already trusted here" probe compares certutil's export against our
    /// own file, and the two differ in wrapping and line endings even when the
    /// certificate is identical. Compared raw, every enable would take the
    /// destructive delete-then-add path on a store that was already correct.
    #[test]
    fn the_nss_pem_comparison_ignores_armour_and_wrapping() {
        let ours = "-----BEGIN CERTIFICATE-----\nMIIB\nAgIU\n-----END CERTIFICATE-----\n";
        let exported = "-----BEGIN CERTIFICATE-----\r\nMIIBAgIU\r\n-----END CERTIFICATE-----";
        assert_eq!(pem_body(ours), pem_body(exported));
    }

    /// ...and a different certificate still has to read as different, or a
    /// regenerated CA would never replace the stale root and every handshake
    /// would keep failing with the old one still in the database.
    #[test]
    fn the_nss_pem_comparison_still_separates_different_certs() {
        assert_ne!(
            pem_body("-----BEGIN CERTIFICATE-----\nMIIB\n-----END CERTIFICATE-----\n"),
            pem_body("-----BEGIN CERTIFICATE-----\nMIIC\n-----END CERTIFICATE-----\n")
        );
    }

    /// The package hint is the answer to a missing `certutil` and to nothing
    /// else. Telling someone to install a package they already have, because
    /// their database was locked, sends them the wrong way.
    #[test]
    fn the_certutil_package_hint_is_only_for_a_missing_binary() {
        assert!(CertutilFailure::Missing
            .tools_hint()
            .contains("libnss3-tools"));
        assert!(CertutilFailure::Failed("locked".into())
            .tools_hint()
            .is_empty());
    }
}
