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
#[cfg(test)]
use std::sync::Mutex;
use std::time::Duration;

use anyhow::{Context, Result};
use hudsucker::rcgen::KeyPair;

use crate::env;
use crate::keychain;
use crate::primitives::{run_as_admin, run_as_root_noninteractive, sh_quote};
use crate::proxy::cert_authority;
use crate::proxy::{NssProbe, NssReading, NssRefusal, NssTrust};

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
    if !is_trusted()? {
        let store = trust_store()?;
        run_as_root_noninteractive(&anchor_install_script(&store, &cert_path()?))
            .context("installing the proxy CA into the system trust store")?;
    }
    // Outside the short-circuit, and present at all, for the same reason
    // [`ensure_trusted`] has it: Chromium reads its own store and never the
    // system one. A headless host has no such store and this is a no-op there,
    // which is the case this function was written for - but the flag is
    // reachable from a desktop, and leaving it out meant `trust-ca
    // --system-trust` installed the anchor, wrote no Chromium store, recorded
    // no reading, and left the window telling the user to reopen a browser
    // that was never going to work.
    ensure_trusted_nss();
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

/// How long one `certutil` call gets before it is killed. Generous next to
/// Windows' 10s because every call here is against a local database and the
/// slow case is a lock, not a service - but bounded, because `enable` waits on
/// these and the user waits on `enable`.
const CERTUTIL_TIMEOUT: Duration = Duration::from_secs(5);

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

/// Test seam: the home [`nss_db_dirs`] enumerates under, absent in every normal
/// build. A `Mutex` static rather than a `HOME` mutation for the same reason
/// [`CERTUTIL_OVERRIDE`] is one.
#[cfg(test)]
static NSS_HOME_OVERRIDE: Mutex<Option<PathBuf>> = Mutex::new(None);

/// The subset of [`nss_db_candidates`] that exists. Empty when no Chromium
/// browser has ever run for this user - the database is created on first
/// launch, so there is nothing to trust into and nothing to warn about.
///
/// [`env::home`] rather than `$HOME` directly, so the e2e harness's redirected
/// home redirects this too. Reading the variable raw left the one path in this
/// module that could reach the developer's own `~/.pki/nssdb` from a test run.
fn nss_db_dirs() -> Vec<PathBuf> {
    #[cfg(test)]
    let home = NSS_HOME_OVERRIDE
        .lock()
        .expect("nss home override mutex poisoned")
        .clone();
    #[cfg(not(test))]
    let home: Option<PathBuf> = None;
    let home = match home.map(Ok).unwrap_or_else(env::home) {
        Ok(home) => home,
        Err(_) => return Vec::new(),
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

/// Test seam: the `certutil` to run, absent in every normal build.
///
/// A static rather than a `PATH` override, which is what the tests reached for
/// first. `std::env::set_var` is not thread safe - `unsafe` as of edition 2024 -
/// and `Command::spawn` reads the environment to build the child's, so a `PATH`
/// mutation here races every other test that spawns. `env::path_env_lock`
/// serialises the ones that take it, and several tests that spawn do not, which
/// makes the lock an incomplete defence rather than a sufficient one.
/// `env::APP_SUPPORT_OVERRIDE` exists for the same class of reason.
///
/// `#[cfg(test)]` so it is not compiled into a shipped build at all, which is
/// stricter than the env seams elsewhere and costs nothing here: the only
/// callers are in this file.
#[cfg(test)]
static CERTUTIL_OVERRIDE: Mutex<Option<PathBuf>> = Mutex::new(None);

/// The binary [`certutil_output`] runs.
///
/// Resolved to an absolute path under the system directories rather than left
/// for `Command` to search `PATH` for - see [`crate::primitives::system_program`]
/// for why, and for what happens on a distro that keeps it elsewhere.
fn certutil_program() -> PathBuf {
    #[cfg(test)]
    if let Some(path) = CERTUTIL_OVERRIDE
        .lock()
        .expect("certutil override mutex poisoned")
        .clone()
    {
        return path;
    }
    crate::primitives::system_program("certutil")
}

/// Collapse machine output to one line and cap it, for a value that lands in a
/// fixed-column report.
///
/// certutil's stderr is routinely two lines, and `NssRefusal.reason` is
/// interpolated into a `row(label, value)` the diagnostics report pads to a
/// column - so an embedded newline emits an unaligned continuation line into a
/// report somebody is reading down. The cap is the other half: nothing bounds
/// how much a child writes before its deadline, and this is the one path that
/// puts that text in front of a person.
fn one_line_capped(text: &str) -> String {
    const MAX: usize = 300;
    let one_line = text.split_whitespace().collect::<Vec<_>>().join(" ");
    match one_line.char_indices().nth(MAX) {
        Some((cut, _)) => format!("{}...", &one_line[..cut]),
        None => one_line,
    }
}

/// One `certutil` invocation against a database, returning its stdout.
/// Unprivileged by construction: these are per-user stores, and running them
/// under the escalation the system anchor needs would write into root's HOME
/// instead of the user's.
fn certutil_output(db: &Path, args: &[&str]) -> std::result::Result<String, CertutilFailure> {
    let mut cmd = Command::new(certutil_program());
    // `sql:` selects the modern cert9.db format. Chromium has written that
    // format for years, and naming it explicitly avoids certutil falling back
    // to the legacy cert8.db pair on an empty directory.
    //
    // Built as an `OsString` rather than through `format!`: a home directory
    // with non-UTF-8 bytes in it would come out of `display()` with U+FFFD
    // substitutions, and certutil would then be pointed at a directory that is
    // not the one we meant.
    let mut db_arg = std::ffi::OsString::from("sql:");
    db_arg.push(db.as_os_str());
    cmd.arg("-d")
        .arg(db_arg)
        .args(args)
        // A database with a password set makes certutil prompt for it on stdin.
        // Under the GUI that reads EOF, but the CLI would hand it the user's
        // terminal and block there, so close it and let the call fail instead.
        // `output_bounded` nulls stdin as well; kept here because this is the
        // caller that knows why it matters.
        .stdin(Stdio::null());
    // Bounded, for the reason `ca_windows`' `certutil_bounded` is: stdin being
    // closed turns the password prompt into an EOF rather than a wait, but a
    // database another process holds locked, or one on a stalled network mount,
    // blocks in the open instead - and this runs where a user is waiting on a
    // switch. A killed call is reported as a failure, which is what it is; the
    // caller's own hint machinery then keeps it out of the missing-package
    // advice.
    //
    // `output_bounded` rather than a loop here: it is the same shape
    // `ca_windows` wrote first, and it reaps the child after the kill, which
    // neither hand-rolled copy did. Its own note covers why this is only safe
    // for a small output - one certificate, here.
    let out = match crate::primitives::output_bounded(cmd, CERTUTIL_TIMEOUT) {
        Ok(Some(out)) => out,
        Ok(None) => {
            return Err(CertutilFailure::Failed(format!(
                "certutil {} did not finish within {}s and was killed",
                args.join(" "),
                CERTUTIL_TIMEOUT.as_secs()
            )))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Err(CertutilFailure::Missing),
        Err(e) => return Err(CertutilFailure::Failed(format!("running certutil: {e}"))),
    };
    if !out.status.success() {
        return Err(CertutilFailure::Failed(format!(
            "certutil {} exited {}: {}",
            args.join(" "),
            out.status,
            one_line_capped(&String::from_utf8_lossy(&out.stderr))
        )));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// [`certutil_output`] where only success matters.
fn certutil(db: &Path, args: &[&str]) -> std::result::Result<(), CertutilFailure> {
    certutil_output(db, args).map(|_| ())
}

/// What one database says about our nickname.
///
/// The distinction the whole tri-state rests on: `Absent` is an answer, and a
/// [`CertutilFailure`] is the lack of one. Folding them together is what let
/// the report print "CA MISSING" for a database it never managed to open.
enum NssEntry {
    /// Our nickname is not in this database.
    Absent,
    /// It is, with these trust flags: certutil's three comma-separated fields,
    /// e.g. `C,,` for the SSL-CA trust the add asks for.
    Present { flags: String },
}

impl NssEntry {
    /// Whether the entry carries CA trust for SSL, which is the only one of the
    /// three fields that decides whether Chromium accepts an intercepted host.
    ///
    /// A certificate sitting in the store with flags `,,` is present, exports
    /// byte-identical to ours, and is trusted for nothing - so a check that
    /// compares the certificate alone reports `Trusted` over a browser that
    /// rejects every host Gate intercepts.
    ///
    /// `C` in the SSL field and nothing else. `T` there is trust for issuing
    /// *client* certificates, which does not make a server leaf validate, so
    /// reading it as trust would report the same false `Trusted` one field
    /// over. The flags the add asks for are `C,,`.
    fn ssl_ca_trusted(&self) -> bool {
        match self {
            Self::Absent => false,
            Self::Present { flags } => flags.split(',').next().is_some_and(|ssl| ssl.contains('C')),
        }
    }
}

/// Look our nickname up in `db`, reading the trust flags with it.
///
/// `certutil -L` with no `-n` lists the whole database, and its exit status is
/// therefore about the *database*: zero means we opened and read it, non-zero
/// means we could not. That is what makes this the readable question. Asking
/// for the nickname directly (`-L -n <nick>`) cannot answer it - certutil exits
/// non-zero both for "no such nickname" and for "no such database", and telling
/// them apart would mean matching on NSS error strings.
///
/// The listing is two header lines and then one row per certificate, the trust
/// flags last and the nickname - which contains spaces - everything before
/// them:
///
/// ```text
/// Certificate Nickname                    Trust Attributes
///                                         SSL,S/MIME,JAR/XPI
///
/// Gate Connect Local CA                   C,,
/// ```
fn nss_lookup(db: &Path) -> std::result::Result<NssEntry, CertutilFailure> {
    let listing = certutil_output(db, &["-L"])?;
    for line in listing.lines() {
        let Some((nickname, flags)) = line.rsplit_once(char::is_whitespace) else {
            continue;
        };
        // The header's second line is the legend `SSL,S/MIME,JAR/XPI` sitting
        // in the flags column with nothing before it, so an empty nickname is
        // not a row.
        let nickname = nickname.trim();
        if nickname == CA_COMMON_NAME {
            return Ok(NssEntry::Present {
                flags: flags.trim().to_string(),
            });
        }
    }
    Ok(NssEntry::Absent)
}

/// Whether `db` holds exactly the certificate in `pem` under our nickname,
/// **and** trusts it to issue SSL server certs.
///
/// Both halves, because either one alone is a browser that still fails: the
/// wrong certificate is rejected, and the right certificate with no trust flag
/// is rejected the same way and looks identical to an exporter.
///
/// `Err` is not `false`. A caller that wants "rewrite it then" can treat the
/// two alike; a caller that reports a reading to a person cannot.
fn nss_holds(db: &Path, pem: &str) -> std::result::Result<bool, CertutilFailure> {
    let entry = nss_lookup(db)?;
    if !entry.ssl_ca_trusted() {
        return Ok(false);
    }
    let held = certutil_output(db, &["-L", "-n", CA_COMMON_NAME, "-a"])?;
    Ok(pem_body(&held) == pem_body(pem))
}

/// What a live read of every per-user NSS database found says about our CA, for
/// the diagnostics report. See [`NssProbe`] for why three answers rather than a
/// bool, and for the precedence between the two negatives.
///
/// `None` where the question does not apply: no Chromium browser has ever run
/// for this user, so there is no database to be in. Also `None` when the cert
/// itself cannot be read, which `ca_cert_present` already reports.
///
/// Shells out once or twice per database, so it belongs on a user action and
/// not on the polled path - `status` serves [`recorded_nss_trust`] instead.
pub fn nss_ca_trusted() -> Option<NssProbe> {
    let dirs = nss_db_dirs();
    if dirs.is_empty() {
        return None;
    }
    let pem = cert_path().ok().and_then(|p| fs::read_to_string(p).ok())?;
    let mut unreadable = false;
    for dir in dirs {
        match nss_holds(&dir, &pem) {
            Ok(true) => {}
            Ok(false) => return Some(NssProbe::Absent),
            Err(_) => unreadable = true,
        }
    }
    Some(if unreadable {
        NssProbe::Unreadable
    } else {
        NssProbe::Holds
    })
}

/// The same live read, as the [`NssTrust`] the UI switches its copy on.
///
/// Called once, from a user-visible transition, when `status` has no recorded
/// reading to serve: the CLI wrote the store and its record is keyed to a CA
/// this process can read, or `proxy trust-ca --system-trust` installed the
/// system anchor and no Chromium store at all. Both leave the GUI watching
/// `ca_trusted` go true with nothing behind it, and the fall-through sentence
/// there tells the user to reopen a browser - which is wrong advice on a
/// machine with no `certutil`, and the loop the copy exists to prevent.
///
/// `None` for every answer that is not one: no store on this machine, no
/// readable cert, or a store that could not be read. The caller's fall-through
/// says only what `ca_trusted` already established, which is the safe sentence
/// to be left with.
pub fn probe_nss_trust() -> Option<NssTrust> {
    let dirs = nss_db_dirs();
    if dirs.is_empty() {
        return None;
    }
    let pem = cert_path().ok().and_then(|p| fs::read_to_string(p).ok())?;
    let mut absent = false;
    for dir in dirs {
        match nss_holds(&dir, &pem) {
            Ok(true) => {}
            Ok(false) => absent = true,
            // The one failure with its own sentence, and it is about the
            // machine rather than the store: no certutil, nothing was written
            // anywhere, and a package install is the fix.
            Err(CertutilFailure::Missing) => return Some(NssTrust::ToolsMissing),
            Err(CertutilFailure::Failed(_)) => return None,
        }
    }
    Some(if absent {
        NssTrust::NotWritten
    } else {
        NssTrust::Trusted
    })
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

/// What the last NSS write recorded, and the CA it was a write of.
///
/// The fingerprint is what keeps a record from outliving its subject. A
/// regenerated CA leaves the old `Trusted` sitting in the file describing a
/// certificate no store holds any more, and the UI would go on saying the
/// browsers are covered. Read back, a record whose fingerprint is not the
/// current cert's is no record at all.
#[derive(serde::Serialize, serde::Deserialize)]
struct NssRecord {
    /// SHA-256 of the CA cert's PEM body, hex. The body rather than the file,
    /// so line endings and a trailing newline cannot change the identity of a
    /// certificate that has not changed.
    cert_fingerprint: String,
    /// What the write saw. Flattened out of a nested object because this file
    /// is read by a person as often as by us.
    #[serde(flatten)]
    reading: NssReading,
}

/// Where [`record_nss_trust`] keeps its record.
///
/// Beside the proxy's other cross-process state (the snapshot, the port file,
/// the op lock) rather than in the CA directory, because that is what it is:
/// state about a machine, written by whichever process last enabled routing.
fn nss_record_path() -> Result<PathBuf> {
    Ok(env::app_support_dir()?.join("proxy").join("nss-trust.json"))
}

/// The current CA's fingerprint, or `None` when there is no readable cert to
/// take one of.
fn cert_fingerprint() -> Option<String> {
    let pem = cert_path().ok().and_then(|p| fs::read_to_string(p).ok())?;
    Some(pem_fingerprint(&pem))
}

/// SHA-256 of a PEM's body, hex. Pure, so the keying is testable without a CA.
fn pem_fingerprint(pem: &str) -> String {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(pem_body(pem).as_bytes());
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

/// Record what the last NSS write produced, for `status` to serve without
/// shelling out. `None` clears the record - see [`NssTrust`]'s own note on why
/// absence is not a negative reading.
///
/// **A file rather than a static, because the writer and the reader are not
/// always the same process.** `gate-connect proxy trust-ca` runs the write in
/// the CLI and exits; the GUI polling `status` beside it is what draws the copy
/// about the result. A process-scoped record left that GUI with no reading at
/// the exact moment it raises the note, and the note's fall-through tells the
/// user to reopen their browser - wrong advice on a machine where `certutil`
/// is not installed, and precisely the loop the tri-state exists to prevent.
///
/// Best-effort: a record that cannot be written costs the UI its reading, and
/// must not fail the enable that produced it. The failure is logged rather than
/// swallowed, because a reading that silently never appears is the harder bug.
fn record_nss_trust(state: Option<NssReading>) {
    let path = match nss_record_path() {
        Ok(path) => path,
        Err(e) => {
            eprintln!("gate proxy: no path for the NSS trust record ({e})");
            return;
        }
    };
    let Some(reading) = state else {
        // A removal, or a question that stopped applying. Absence of the file
        // is absence of a reading, which is the same thing the reader makes of
        // a record it cannot match to the current CA.
        if let Err(e) = fs::remove_file(&path) {
            if e.kind() != std::io::ErrorKind::NotFound {
                eprintln!(
                    "gate proxy: could not clear the NSS trust record at {} ({e})",
                    path.display()
                );
            }
        }
        return;
    };
    let Some(cert_fingerprint) = cert_fingerprint() else {
        // Nothing to key the record to. Recording it anyway would produce a
        // reading that can never be matched, which reads as no reading with
        // extra steps.
        eprintln!("gate proxy: no readable CA cert to key the NSS trust record to");
        return;
    };
    let record = NssRecord {
        cert_fingerprint,
        reading,
    };
    let raw = match serde_json::to_string_pretty(&record) {
        Ok(raw) => raw,
        Err(e) => {
            eprintln!("gate proxy: could not serialize the NSS trust record ({e})");
            return;
        }
    };
    // Atomic, like the proxy snapshot beside it: a torn record parses as no
    // record, and the enable that wrote it has already happened.
    if let Err(e) = crate::primitives::write_file(&path, raw.as_bytes(), 0o600) {
        eprintln!(
            "gate proxy: could not write the NSS trust record at {} ({e})",
            path.display()
        );
    }
}

/// The recorded reading, for [`super::manager`]'s `status` and for the
/// diagnostics report. `status` takes the outcome alone; the report takes the
/// refusals too, because it is what the copy points at for the store and the
/// reason.
///
/// `None` for every way of not having one: no file, a file that does not parse,
/// no readable cert to check it against, or a record written for a different
/// CA. That last one is the reason the fingerprint is in the file at all.
pub fn recorded_nss_trust() -> Option<NssReading> {
    let raw = fs::read_to_string(nss_record_path().ok()?).ok()?;
    let record: NssRecord = serde_json::from_str(&raw).ok()?;
    (record.cert_fingerprint == cert_fingerprint()?).then_some(record.reading)
}

/// Fold one store's failure into the machine's answer.
///
/// `ToolsMissing` outranks `WriteFailed` and is never overwritten by it: a
/// missing binary fails every store, so it is the whole machine's answer rather
/// than one store's, and it is the only one of the two with a fix the user can
/// act on. A locked store reported beside it must not bury that - which is the
/// same argument `CertutilFailure` splits the two cases for, one level down.
///
/// Pure and split out so the precedence is testable without a database to fail;
/// `nss_db_candidates` is split from `nss_db_dirs` for the same reason.
fn degrade(outcome: NssTrust, failure: &CertutilFailure) -> NssTrust {
    match (failure, outcome) {
        (CertutilFailure::Missing, _) | (_, NssTrust::ToolsMissing) => NssTrust::ToolsMissing,
        _ => NssTrust::WriteFailed,
    }
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
        // No such store on this machine, so there is nothing to report about
        // one. Cleared rather than left alone: a browser installed and removed
        // between two enables would otherwise leave its verdict standing.
        record_nss_trust(None);
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
            // Not a verdict about the store: we never asked it anything. The
            // system anchor install has the same cert problem and says so.
            record_nss_trust(None);
            return;
        }
    };
    // Starts at the answer the steady state gives - every database already
    // holding the CA skips its whole body below - and degrades as stores fail.
    let mut outcome = NssTrust::Trusted;
    // The stores that refused, for the report the copy sends people to. Only
    // the `Failed` cause is collected - see `NssRefusal`.
    let mut refusals: Vec<NssRefusal> = Vec::new();
    for dir in dirs {
        // Nothing to do where the database already holds exactly our current
        // CA, trusted for SSL. Worth the extra call: the rewrite below is
        // briefly destructive, and skipping it keeps that window out of the
        // common path altogether. A store that cannot be read is not skipped -
        // it is rewritten, and the rewrite's own failure is what gets reported.
        if nss_holds(&dir, &cert_pem).unwrap_or(false) {
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
        // Read the store back rather than reporting the exit code. `Trusted`
        // is a claim about what Chromium will accept, and an add that exits
        // zero is only evidence that certutil was happy - the reading is one
        // more call on a path that has already made several, and it is the
        // difference between a verdict and an inference.
        let written = certutil(&dir, &args).and_then(|()| nss_holds(&dir, &cert_pem));
        let failure = match written {
            Ok(true) => None,
            Ok(false) => Some(CertutilFailure::Failed(
                "certutil -A reported success and the store does not hold the CA".to_string(),
            )),
            Err(e) => Some(e),
        };
        if let Some(e) = failure {
            outcome = degrade(outcome, &e);
            if matches!(e, CertutilFailure::Failed(_)) {
                refusals.push(NssRefusal {
                    store: dir.display().to_string(),
                    reason: e.to_string(),
                });
            }
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
    // `refusals` is documented as empty for every outcome but `WriteFailed`,
    // and one store failing with a refusal before a later one loses certutil
    // entirely would otherwise leave the report naming a store under a headline
    // that says the binary is missing.
    if outcome == NssTrust::ToolsMissing {
        refusals.clear();
    }
    record_nss_trust(Some(NssReading { outcome, refusals }));
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
/// that fails for any *other* reason is a store we could not read, which is
/// reported rather than assumed empty; a missing `certutil` is separated out,
/// since it means we could neither look nor remove and anything the install put
/// there is still there.
fn untrust_nss() {
    // Recorded at the end rather than cleared here. Clearing up front made a
    // removal that failed indistinguishable from one that worked: `None` is
    // "no reading", so a Gate root still sitting in Chromium's store - the one
    // state this function's doc calls the security edge - was the single state
    // the reading could not express.
    let mut failure: Option<NssTrust> = None;
    for dir in nss_db_dirs() {
        match nss_lookup(&dir) {
            Ok(NssEntry::Absent) => {}
            Ok(NssEntry::Present { .. }) => {
                if let Err(e) = certutil(&dir, &["-D", "-n", CA_COMMON_NAME]) {
                    failure = Some(NssTrust::WriteFailed);
                    eprintln!(
                        "gate proxy: could not remove the CA from the NSS store at {dir} ({e}); \
                         Chromium-based browsers still trust it",
                        dir = dir.display(),
                    );
                }
            }
            Err(e @ CertutilFailure::Missing) => {
                failure = Some(NssTrust::ToolsMissing);
                eprintln!(
                    "gate proxy: could not remove the CA from the NSS store at {dir} ({e}); \
                     Chromium-based browsers may still trust it - {NSS_TOOLS_HINT}",
                    dir = dir.display(),
                );
            }
            Err(e @ CertutilFailure::Failed(_)) => {
                failure = Some(NssTrust::WriteFailed);
                eprintln!(
                    "gate proxy: could not read the NSS store at {dir} to remove the CA ({e}); \
                     Chromium-based browsers may still trust it",
                    dir = dir.display(),
                );
            }
        }
    }
    // A clean removal is a question that stopped applying, not a verdict: the
    // CA is going on purpose and `ca_trusted` goes false beside it. A
    // `WriteFailed` left standing there would describe a removal as a fault.
    //
    // `remove_ca_material` runs right after this and deletes the cert, so a
    // record kept here survives only as long as its fingerprint can be
    // matched - which is to say, not past the deletion. It is the log line
    // that carries this past the process either way; the record is for the
    // window that is still open.
    record_nss_trust(failure.map(|outcome| NssReading {
        outcome,
        refusals: Vec::new(),
    }));
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

    /// A stand-in `certutil` that behaves however the test needs.
    ///
    /// Drives the real code path, which is the point: what wants testing is the
    /// wiring from a killed call to the error the caller reports, and that is
    /// Gate's code, not NSS's.
    ///
    /// Installed through [`CERTUTIL_OVERRIDE`] rather than by replacing `PATH`,
    /// which is the version this started as: mutating the environment races
    /// every other test that spawns, and the path lock only covers the ones that
    /// take it. See the override's own note.
    ///
    /// Its own lock, because the override is process-global and the three tests
    /// below run in parallel.
    struct CertutilShim {
        dir: PathBuf,
        _lock: std::sync::MutexGuard<'static, ()>,
    }

    impl CertutilShim {
        /// `script` is the body of a `sh` program installed as the stand-in.
        /// `None` writes nothing and points the override at the path anyway,
        /// which is how the missing-binary branch is reached: spawning something
        /// that is not there yields the same `NotFound` a `certutil` that is not
        /// installed does.
        fn new(tag: &str, script: Option<&str>) -> Self {
            static SHIM_LOCK: Mutex<()> = Mutex::new(());
            let lock = SHIM_LOCK.lock().unwrap_or_else(|e| e.into_inner());
            let dir = std::env::temp_dir()
                .join(format!("gate-certutil-shim-{}-{tag}", std::process::id()));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).expect("create shim dir");
            let bin = dir.join("certutil");
            if let Some(script) = script {
                fs::write(&bin, format!("#!/bin/sh\n{script}\n")).expect("write shim");
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    fs::set_permissions(&bin, fs::Permissions::from_mode(0o755))
                        .expect("chmod shim");
                }
            }
            *CERTUTIL_OVERRIDE
                .lock()
                .expect("certutil override mutex poisoned") = Some(bin);
            Self { dir, _lock: lock }
        }
    }

    impl Drop for CertutilShim {
        fn drop(&mut self) {
            *CERTUTIL_OVERRIDE
                .lock()
                .expect("certutil override mutex poisoned") = None;
            let _ = fs::remove_dir_all(&self.dir);
        }
    }

    /// A `certutil` that never returns is killed, and the caller is told that
    /// rather than being left waiting on it.
    ///
    /// The path the branch bounded: a database another process holds locked, or
    /// one on a stalled network mount, blocks in the open. This is the only test
    /// that exercises the timeout through the real caller, so it also pins that
    /// the message names the timeout rather than reading like a certutil error.
    #[test]
    fn a_certutil_that_hangs_is_killed_and_reported() {
        let _shim = CertutilShim::new("hang", Some("sleep 30"));
        let started = std::time::Instant::now();
        let err = certutil_output(Path::new("/nonexistent/db"), &["-L"])
            .expect_err("a killed call is a failure");
        // Near the deadline, not merely under the child's own 30s: at three
        // times the budget this passed a deadline that had grown by an order
        // of magnitude, which is the regression worth catching. A second of
        // slack is ample on a loaded runner for a call that is killed.
        assert!(
            started.elapsed() < CERTUTIL_TIMEOUT + Duration::from_secs(1),
            "did not return near the deadline"
        );
        match &err {
            CertutilFailure::Failed(msg) => {
                assert!(msg.contains("did not finish"), "got {msg}");
                assert!(msg.contains("was killed"), "got {msg}");
            }
            CertutilFailure::Missing => panic!("a hang is not a missing binary"),
        }
        // The half that matters downstream: a hang must not be prescribed a
        // package install. `degrade` turns this into `WriteFailed`, not
        // `ToolsMissing`.
        assert_eq!(err.tools_hint(), "");
        assert_eq!(
            degrade(NssTrust::Trusted, &err),
            NssTrust::WriteFailed,
            "a timeout read as a missing package would send the user to install \
             one they already have"
        );
    }

    /// A `certutil` that is not there at all is the other branch, and it is the
    /// one the package hint answers.
    #[test]
    fn an_absent_certutil_is_reported_as_missing() {
        let _shim = CertutilShim::new("absent", None);
        let err = certutil_output(Path::new("/nonexistent/db"), &["-L"])
            .expect_err("no binary is a failure");
        // `matches!` rather than `assert_eq!`: `CertutilFailure` carries no
        // `PartialEq`, and deriving one so a test can use a nicer macro is the
        // wrong way round.
        assert!(
            matches!(err, CertutilFailure::Missing),
            "a binary that is not there is the Missing branch, not a failed call"
        );
        assert!(err.tools_hint().contains("libnss3-tools"));
    }

    /// A `certutil` that fails on its own terms is neither of the above: it
    /// answered, so the exit status and its stderr are the report.
    #[test]
    fn a_failing_certutil_reports_its_own_words() {
        let _shim = CertutilShim::new("fail", Some("echo 'SEC_ERROR_BAD_DATABASE' >&2; exit 255"));
        let err = certutil_output(Path::new("/nonexistent/db"), &["-L"])
            .expect_err("a non-zero exit is a failure");
        match &err {
            CertutilFailure::Failed(msg) => {
                assert!(msg.contains("exited"), "got {msg}");
                assert!(msg.contains("SEC_ERROR_BAD_DATABASE"), "got {msg}");
            }
            CertutilFailure::Missing => panic!("an exit status is not a missing binary"),
        }
    }

    fn debian_store() -> TrustStore {
        TrustStore {
            anchor: PathBuf::from("/usr/local/share/ca-certificates/Gate CA.crt"),
            install_cmd: "update-ca-certificates",
            refresh_cmd: "update-ca-certificates --fresh",
        }
    }

    /// A machine with an NSS database, a CA cert on disk, and somewhere for the
    /// record to go - none of it the developer's own.
    ///
    /// Holds three process-global seams at once: the app-support dir (so
    /// `cert_path` and `nss_record_path` land in the temp root), the NSS home
    /// (so `nss_db_dirs` enumerates the temp database), and the `certutil`
    /// override the shim installs. `path_env_lock` is the lock every
    /// path-redirecting test in the crate takes; the shim's own lock nests
    /// inside it, always in that order.
    struct NssFixture {
        root: PathBuf,
        marker: PathBuf,
        db: PathBuf,
        _shim: CertutilShim,
        _lock: std::sync::MutexGuard<'static, ()>,
    }

    impl NssFixture {
        /// `script` is the stand-in certutil's body, usually from
        /// [`store_shim`]. The store starts out empty unless `holds` is set,
        /// which writes the marker the shim keys its listing off.
        fn new(tag: &str, holds: bool, script: impl Fn(&Path, &Path) -> String) -> Self {
            let lock = crate::env::path_env_lock();
            let root = std::env::temp_dir().join(format!(
                "gate-nss-{tag}-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .expect("clock before the epoch")
                    .as_nanos()
            ));
            let db = root.join("home").join(".pki").join("nssdb");
            fs::create_dir_all(&db).expect("create nss db dir");
            let support = root.join("support");
            fs::create_dir_all(support.join("proxy")).expect("create support dir");
            crate::env::set_app_support_dir_for_tests(Some(support));
            *NSS_HOME_OVERRIDE
                .lock()
                .expect("nss home override mutex poisoned") = Some(root.join("home"));
            let cert = cert_path().expect("cert path");
            fs::write(&cert, TEST_CERT).expect("write cert");
            let marker = root.join("held");
            if holds {
                fs::write(&marker, "1").expect("write marker");
            }
            let shim = CertutilShim::new(tag, Some(&script(&cert, &marker)));
            Self {
                root,
                marker,
                db,
                _shim: shim,
                _lock: lock,
            }
        }

        /// Whether the stand-in store holds our nickname right now.
        fn store_holds(&self) -> bool {
            self.marker.exists()
        }
    }

    impl Drop for NssFixture {
        fn drop(&mut self) {
            *NSS_HOME_OVERRIDE
                .lock()
                .expect("nss home override mutex poisoned") = None;
            crate::env::set_app_support_dir_for_tests(None);
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    /// Any PEM will do: every comparison here is [`pem_body`] against itself.
    const TEST_CERT: &str =
        "-----BEGIN CERTIFICATE-----\nMIIBTESTBODY\n-----END CERTIFICATE-----\n";

    /// How the stand-in `certutil -A` behaves.
    enum Add {
        /// Exits zero and the certificate is in the store afterwards.
        Lands,
        /// Exits zero and nothing changed - the case an exit code cannot see.
        Silent,
        /// Exits non-zero with a reason, like a locked database.
        Refuses,
    }

    /// A stand-in `certutil` backed by a marker file: `-L` lists our nickname
    /// while the marker is there, `-A` creates it (or does not), `-D` removes
    /// it, and `-L -n ... -a` exports the cert.
    ///
    /// Switching on `$3` rather than pattern-matching the whole command line:
    /// the nickname contains spaces and the export flag is `-a` to the
    /// listing's `-A`, and both had a way of matching the wrong arm.
    fn store_shim(flags: &'static str, add: Add) -> impl Fn(&Path, &Path) -> String {
        let add_body = match add {
            Add::Lands => "touch '{marker}'; exit 0",
            Add::Silent => "exit 0",
            Add::Refuses => "echo 'SEC_ERROR_TOKEN_NOT_LOGGED_IN' >&2; exit 255",
        };
        move |cert: &Path, marker: &Path| {
            let cert = cert.display();
            let marker_s = marker.display();
            let add_body = add_body.replace("{marker}", &marker_s.to_string());
            format!(
                r#"
case "$3" in
  -L)
    if [ "$4" = "-n" ]; then
      if [ -f '{marker_s}' ]; then cat '{cert}'; exit 0; fi
      echo 'certutil: Could not find cert' >&2
      exit 255
    fi
    echo 'Certificate Nickname                             Trust Attributes'
    echo '                                                 SSL,S/MIME,JAR/XPI'
    echo ''
    if [ -f '{marker_s}' ]; then
      echo '{CA_COMMON_NAME}                             {flags}'
    fi
    exit 0
    ;;
  -A)
    {add_body}
    ;;
  -D)
    if [ -f '{marker_s}' ]; then rm -f '{marker_s}'; exit 0; fi
    echo 'certutil: could not find cert to delete' >&2
    exit 255
    ;;
esac
exit 0
"#,
            )
        }
    }

    /// A store that already holds the current CA with SSL-CA trust is the
    /// steady state: `Trusted`, and no write.
    ///
    /// The no-write half is what keeps the briefly destructive delete-then-add
    /// out of every enable after the first.
    #[test]
    fn a_store_that_already_holds_the_ca_is_trusted_without_a_write() {
        let fixture = NssFixture::new("steady", true, store_shim("C,,", Add::Refuses));
        ensure_trusted_nss();
        let reading = recorded_nss_trust().expect("a write records a reading");
        assert_eq!(reading.outcome, NssTrust::Trusted);
        assert!(reading.refusals.is_empty());
        // The shim's `-A` refuses, so a `Trusted` here proves the add was never
        // reached rather than that it succeeded.
        assert!(fixture.store_holds());
    }

    /// The same certificate with no SSL trust flag is not trusted, however
    /// identical it looks to an exporter.
    ///
    /// Chromium rejects every intercepted host in this state, and comparing the
    /// certificate alone reported it as fine.
    #[test]
    fn a_cert_present_without_ssl_trust_is_rewritten_rather_than_skipped() {
        let fixture = NssFixture::new("noflags", true, store_shim(",,", Add::Lands));
        ensure_trusted_nss();
        let reading = recorded_nss_trust().expect("a write records a reading");
        // The skip did not fire, the rewrite did, and the shim's listing still
        // reports `,,` - so the read-back says it did not land.
        assert_eq!(reading.outcome, NssTrust::WriteFailed);
        assert!(fixture.store_holds());
    }

    /// An add that exits zero and changes nothing is a failure, and reporting
    /// the exit code alone called it `Trusted`.
    #[test]
    fn an_add_that_exits_zero_without_landing_is_not_trusted() {
        let _fixture = NssFixture::new("silent", false, store_shim("C,,", Add::Silent));
        ensure_trusted_nss();
        let reading = recorded_nss_trust().expect("a write records a reading");
        assert_eq!(reading.outcome, NssTrust::WriteFailed);
        assert_eq!(reading.refusals.len(), 1, "{:?}", reading.refusals);
        assert!(
            reading.refusals[0].reason.contains("does not hold the CA"),
            "{:?}",
            reading.refusals[0]
        );
    }

    /// A store that refuses the add is `WriteFailed`, and the report can name
    /// which store and why - which is what the copy raised on this state tells
    /// the user the report does.
    #[test]
    fn a_store_that_refuses_is_write_failed_and_names_itself() {
        let fixture = NssFixture::new("refuse", false, store_shim("C,,", Add::Refuses));
        ensure_trusted_nss();
        let reading = recorded_nss_trust().expect("a write records a reading");
        assert_eq!(reading.outcome, NssTrust::WriteFailed);
        assert_eq!(reading.refusals.len(), 1);
        assert_eq!(reading.refusals[0].store, fixture.db.display().to_string());
        assert!(
            reading.refusals[0]
                .reason
                .contains("SEC_ERROR_TOKEN_NOT_LOGGED_IN"),
            "{:?}",
            reading.refusals[0]
        );
    }

    /// No `certutil` is the whole machine's answer rather than one store's, and
    /// it carries no refusals - the report's headline would otherwise name a
    /// store under a sentence saying the binary is missing.
    #[test]
    fn a_missing_certutil_is_tools_missing_with_no_refusals() {
        let _fixture = NssFixture::new("notools", false, |_, _| String::new());
        // The shim directory exists and the binary in it does not, which is the
        // same `NotFound` an uninstalled certutil gives.
        *CERTUTIL_OVERRIDE
            .lock()
            .expect("certutil override mutex poisoned") =
            Some(PathBuf::from("/nonexistent/certutil"));
        ensure_trusted_nss();
        let reading = recorded_nss_trust().expect("a write records a reading");
        assert_eq!(reading.outcome, NssTrust::ToolsMissing);
        assert!(
            reading.refusals.is_empty(),
            "a missing binary is not a store refusing: {:?}",
            reading.refusals
        );
    }

    /// No Chromium store on the machine is not a verdict about one. A browser
    /// installed and removed between two enables has to clear the old reading
    /// rather than leave it standing.
    #[test]
    fn no_nss_stores_records_nothing() {
        let fixture = NssFixture::new("nostores", true, store_shim("C,,", Add::Lands));
        ensure_trusted_nss();
        assert!(recorded_nss_trust().is_some(), "the store was there");
        fs::remove_dir_all(&fixture.db).expect("remove the database");
        ensure_trusted_nss();
        assert!(
            recorded_nss_trust().is_none(),
            "a store that is gone leaves no verdict behind"
        );
    }

    /// A cert we cannot read is not a store verdict either: nothing was asked
    /// of any database.
    #[test]
    fn an_unreadable_cert_is_not_a_store_verdict() {
        let _fixture = NssFixture::new("nocert", false, store_shim("C,,", Add::Lands));
        fs::remove_file(cert_path().expect("cert path")).expect("remove the cert");
        ensure_trusted_nss();
        assert!(recorded_nss_trust().is_none());
    }

    /// The record is a file, so the process that reads it does not have to be
    /// the one that wrote it - which is the whole point: `proxy trust-ca` runs
    /// the write in the CLI and exits, and the window is what draws the copy.
    #[test]
    fn the_recording_survives_the_process_that_took_it() {
        let _fixture = NssFixture::new("shared", false, store_shim("C,,", Add::Lands));
        ensure_trusted_nss();
        let path = nss_record_path().expect("record path");
        assert!(
            path.exists(),
            "the reading is on disk at {}",
            path.display()
        );
        let raw = fs::read_to_string(&path).expect("read the record");
        assert!(raw.contains("\"outcome\": \"trusted\""), "{raw}");
        assert!(raw.contains("cert_fingerprint"), "{raw}");
    }

    /// ...and it is keyed to the certificate it describes, so a regenerated CA
    /// retires the old reading instead of carrying it forward over a store that
    /// holds a root nobody trusts any more.
    #[test]
    fn a_record_for_a_different_ca_is_not_a_reading() {
        let _fixture = NssFixture::new("rekey", false, store_shim("C,,", Add::Lands));
        ensure_trusted_nss();
        assert!(recorded_nss_trust().is_some());
        fs::write(
            cert_path().expect("cert path"),
            "-----BEGIN CERTIFICATE-----\nMIIBDIFFERENT\n-----END CERTIFICATE-----\n",
        )
        .expect("regenerate the cert");
        assert!(
            recorded_nss_trust().is_none(),
            "a reading about the old CA is not a reading about this one"
        );
    }

    /// A removal that fails is the one NSS state with a security edge - a root
    /// that can still sign for any host the browser trusts - so it has to be
    /// expressible. Clearing the record up front made it the one state that was
    /// not.
    #[test]
    fn an_untrust_that_cannot_remove_records_the_failure() {
        let _fixture = NssFixture::new("untrust-fail", true, |cert, _marker| {
            // Lists the entry, refuses to delete it.
            format!(
                r#"
case "$3" in
  -L)
    if [ "$4" = "-n" ]; then cat '{cert}'; exit 0; fi
    echo 'Certificate Nickname                             Trust Attributes'
    echo ''
    echo '{nick}                             C,,'
    exit 0
    ;;
  -D)
    echo 'SEC_ERROR_READ_ONLY' >&2
    exit 255
    ;;
esac
exit 0
"#,
                cert = cert.display(),
                nick = CA_COMMON_NAME,
            )
        });
        untrust_nss();
        let reading = recorded_nss_trust().expect("a failed removal is a reading");
        assert_eq!(reading.outcome, NssTrust::WriteFailed);
    }

    /// A removal that works is a question that stopped applying, not a fault.
    #[test]
    fn a_clean_untrust_leaves_no_reading() {
        let _fixture = NssFixture::new("untrust-ok", true, store_shim("C,,", Add::Lands));
        untrust_nss();
        assert!(recorded_nss_trust().is_none());
    }

    /// The probe the window falls back to when nothing recorded a write: a
    /// store that never took the CA is `NotWritten`, which is a retry rather
    /// than a package install or a trip to the report.
    #[test]
    fn the_probe_reports_a_store_that_never_took_the_ca_as_not_written() {
        let _fixture = NssFixture::new("probe-empty", false, store_shim("C,,", Add::Lands));
        assert_eq!(probe_nss_trust(), Some(NssTrust::NotWritten));
        assert_eq!(nss_ca_trusted(), Some(NssProbe::Absent));
    }

    /// A store that holds it reads as trusted from the probe too, so the note
    /// the window raises is the plain "reopen your browser" one.
    #[test]
    fn the_probe_reports_a_store_that_holds_the_ca_as_trusted() {
        let _fixture = NssFixture::new("probe-holds", true, store_shim("C,,", Add::Lands));
        assert_eq!(probe_nss_trust(), Some(NssTrust::Trusted));
        assert_eq!(nss_ca_trusted(), Some(NssProbe::Holds));
    }

    /// A store that could not be read is **not** a store missing the CA. The
    /// report printed `CA MISSING` for both, and this branch gave the second a
    /// new way to happen by killing certutil at a deadline.
    #[test]
    fn a_store_that_cannot_be_read_is_not_a_store_missing_the_ca() {
        let _fixture = NssFixture::new("probe-locked", false, |_, _| {
            "echo 'SEC_ERROR_BAD_DATABASE' >&2; exit 255".to_string()
        });
        assert_eq!(
            nss_ca_trusted(),
            Some(NssProbe::Unreadable),
            "a database that would not open is not a database without the CA"
        );
        assert_eq!(
            probe_nss_trust(),
            None,
            "and it is no reading at all for the copy, which has a safe fall-through"
        );
    }

    /// No `certutil` has its own answer from the probe, because it is the one
    /// the package hint fixes.
    #[test]
    fn the_probe_reports_a_missing_certutil_as_tools_missing() {
        let _fixture = NssFixture::new("probe-notools", false, |_, _| String::new());
        *CERTUTIL_OVERRIDE
            .lock()
            .expect("certutil override mutex poisoned") =
            Some(PathBuf::from("/nonexistent/certutil"));
        assert_eq!(probe_nss_trust(), Some(NssTrust::ToolsMissing));
    }

    /// The listing parser has to survive the nickname containing spaces, which
    /// is what rules out splitting on the first gap.
    #[test]
    fn the_listing_parser_reads_the_flags_beside_a_spaced_nickname() {
        let fixture = NssFixture::new("listing", true, store_shim("CT,c,", Add::Lands));
        // `Ok(..)` rather than `expect`: `CertutilFailure` carries no `Debug`,
        // and deriving one so a test can use a nicer macro is the wrong way
        // round - the same call the three shim tests above make.
        match nss_lookup(&fixture.db) {
            Ok(NssEntry::Present { flags }) => assert_eq!(flags, "CT,c,"),
            Ok(NssEntry::Absent) => panic!("the entry is right there in the listing"),
            Err(e) => panic!("the shim answers: {e}"),
        }
    }

    /// Only the SSL field decides whether Chromium accepts an intercepted host,
    /// so trust in the other two is not trust.
    #[test]
    fn only_the_ssl_field_counts_as_ca_trust() {
        let ssl = |flags: &str| {
            NssEntry::Present {
                flags: flags.to_string(),
            }
            .ssl_ca_trusted()
        };
        assert!(ssl("C,,"));
        assert!(ssl("CT,C,C"));
        assert!(!ssl(",,"));
        assert!(!ssl(",C,C"), "S/MIME and object signing are not this");
        assert!(
            !ssl("T,,"),
            "client-certificate trust does not validate a server leaf"
        );
        assert!(!NssEntry::Absent.ssl_ca_trusted());
    }

    /// certutil's stderr is routinely two lines, and the report pads this into
    /// a fixed column - so a newline in it breaks the layout of the thing
    /// somebody is reading down.
    #[test]
    fn a_refusal_reason_is_one_line_and_bounded() {
        let messy = "certutil: could not open\n: SEC_ERROR_BAD_DATABASE:   the\tdatabase\n";
        assert_eq!(
            one_line_capped(messy),
            "certutil: could not open : SEC_ERROR_BAD_DATABASE: the database"
        );
        let long = "x".repeat(1000);
        let capped = one_line_capped(&long);
        assert!(capped.len() < 400, "{} chars", capped.len());
        assert!(capped.ends_with("..."));
    }

    /// The fingerprint is over the PEM body, so the same certificate written
    /// with different line endings is the same record - and a different
    /// certificate never is.
    #[test]
    fn the_record_fingerprint_follows_the_certificate_not_its_formatting() {
        let wrapped = "-----BEGIN CERTIFICATE-----\nMIIB\nAgIU\n-----END CERTIFICATE-----\n";
        let flat = "-----BEGIN CERTIFICATE-----\r\nMIIBAgIU\r\n-----END CERTIFICATE-----";
        assert_eq!(pem_fingerprint(wrapped), pem_fingerprint(flat));
        assert_ne!(
            pem_fingerprint(wrapped),
            pem_fingerprint("-----BEGIN CERTIFICATE-----\nMIIC\n-----END CERTIFICATE-----\n")
        );
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
    fn a_missing_certutil_outranks_a_store_that_refused() {
        // The whole machine's answer, not one store's: nothing was written
        // anywhere, and the install is what fixes every one of them. Order must
        // not matter, so both directions are pinned.
        let locked = CertutilFailure::Failed("locked".into());
        assert_eq!(
            degrade(
                degrade(NssTrust::Trusted, &locked),
                &CertutilFailure::Missing
            ),
            NssTrust::ToolsMissing
        );
        assert_eq!(
            degrade(
                degrade(NssTrust::Trusted, &CertutilFailure::Missing),
                &locked
            ),
            NssTrust::ToolsMissing
        );
    }

    #[test]
    fn a_store_that_refused_is_not_a_missing_package() {
        // The finding this split exists for: prescribing libnss3-tools to
        // somebody whose database was locked sends them to install a package
        // they already have.
        assert_eq!(
            degrade(NssTrust::Trusted, &CertutilFailure::Failed("locked".into())),
            NssTrust::WriteFailed
        );
    }

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
