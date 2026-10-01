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
//! The system store is not the whole job, though. Browsers on Linux mostly
//! never read it. Chromium-based ones use their own built-in roots plus a
//! per-user NSS database at `~/.pki/nssdb`; Firefox uses its own built-in roots
//! plus each profile's `cert9.db`, and only sees the system anchors where the
//! distro swaps p11-kit in for those built-ins (Fedora, Arch - not Ubuntu,
//! whose Firefox is the Mozilla snap). So a system-only install leaves both
//! failing every intercepted host with `ERR_CERT_AUTHORITY_INVALID` /
//! `SEC_ERROR_UNKNOWN_ISSUER` while curl is happy, and [`ensure_trusted`]
//! writes those databases too, unprivileged and best-effort, via `certutil`.
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

use std::ffi::OsStr;
use std::fs;
use std::io::{Read, Write};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use hudsucker::rcgen::KeyPair;

use crate::env;
use crate::keychain;
use crate::primitives::{run_as_admin, run_as_root_noninteractive, sh_quote};
use crate::proxy::cert_authority;

/// Subject CN of our CA. Used both as the cert subject and as the basename of
/// the installed anchor file. A dev run has its own, so its anchor is a
/// different file from the release install's; see
/// [`cert_authority::ca_common_name`].
fn ca_common_name() -> &'static str {
    cert_authority::ca_common_name()
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
    Ok(env::ca_material_dir()?.join("ca-cert.pem"))
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
    let anchor_file = format!("{}.crt", ca_common_name());
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

    // Certificate first, because it witnesses the key: the pair is always
    // written together below, so a byte-identical cert on disk means the key in
    // the store is the one that goes with it. That keeps this off the secret
    // store on the repeat calls - the proxy manager makes one every 30 seconds
    // to re-push the intercept config - where each read would cost the daemon
    // ~8 KB it never returns. See `keychain::get_cached`.
    let existing_cert = match fs::read_to_string(&path) {
        Ok(c) => Some(c),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
    };
    let existing_key = match existing_cert.as_deref() {
        Some(cert_pem) => keychain::get_cached(&service, &user, cert_pem)?,
        // No cert to vouch for it: both halves are about to be regenerated.
        None => None,
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
        // And the key has to belong to the certificate: see
        // `cert_authority::key_matches_cert` for the mismatch this catches.
        if cert_authority::host_fingerprint_is_current(&path)
            && cert_authority::key_matches_cert(&key_pem, &cert_pem)
        {
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
    // A new root is a new question for every browser store, including one the
    // user took the old root out of; see `nss_action`.
    clear_nss_ledger();
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
/// [`untrust`]: same anchor, same rebuild, non-interactive escalation. The
/// browser stores go too, although [`ensure_trusted_system`] never writes them:
/// `--system-trust` is the removal the CLI names for this host, and the GUI or
/// a plain `trust-ca` may have written them. That step needs no privilege, so
/// it keeps to the no-prompt contract.
pub fn untrust_system() -> Result<()> {
    let store = trust_store()?;
    if store.anchor.exists() {
        run_as_root_noninteractive(&anchor_remove_script(&store))
            .context("removing the proxy CA from the system trust store")?;
    }
    untrust_nss();
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
/// roots from, whether or not it exists. Pure, and split from
/// [`BrowserEnv::stores`] so the path set is testable without a browser
/// installed.
///
/// Chromium on Linux does not consult the system CA bundle at all: it uses its
/// own built-in root store plus this database. So the system anchor the rest of
/// this module installs leaves every Chromium browser failing the handshake on
/// intercepted hosts with `ERR_CERT_AUTHORITY_INVALID` while curl is happy.
/// Firefox has the same problem with stores of its own; see
/// [`firefox_profile_roots`].
///
/// Enumerated rather than globbed (`~/.var/app/*/.pki/nssdb`) on purpose: a glob
/// would hand our signing root to every confined app that happens to keep an NSS
/// database, browser or not, and the trust here is meant to stay narrow. The
/// cost is that a Chromium-family browser missing from this list fails exactly
/// the way the bug did, so a new one belongs here. The first entry is the
/// exception to "narrow": `~/.pki/nssdb` is the shared per-user NSS default, so
/// any NSS program that opens it (Evolution, an NSS-built curl) trusts what it
/// holds - the same programs the system anchor already reaches through p11-kit.
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

/// Where each Firefox build keeps its profile directories, whether or not they
/// exist. Pure for the same reason as [`nss_db_candidates`].
///
/// Firefox does not read `~/.pki/nssdb`. Every profile has its own `cert9.db`,
/// and outside the distros that wire p11-kit in place of its built-in roots it
/// never sees the system anchor either - which is Ubuntu, where the default
/// Firefox is the Mozilla snap. So without these, Firefox rejects every
/// intercepted host exactly the way Chromium did.
fn firefox_profile_roots(home: &Path) -> Vec<PathBuf> {
    [
        // Distro packages and Mozilla's own tarball.
        ".mozilla/firefox",
        // The snap keeps its profiles under `common`, which survives refreshes,
        // rather than the per-revision `current`.
        "snap/firefox/common/.mozilla/firefox",
        ".var/app/org.mozilla.firefox/.mozilla/firefox",
    ]
    .iter()
    .map(|rel| home.join(rel))
    .collect()
}

/// The profile directories a `profiles.ini` names, resolved against `root`.
/// Pure, so it is testable without a Firefox.
///
/// A relative `Path=` has to be a single plain component: anything with a
/// separator or `..` could walk out of `root`. An absolute one - a profile the
/// user moved with Firefox's profile manager - counts only when
/// `allow_absolute`, which the caller grants for the unconfined root alone. A
/// snap or Flatpak Firefox can write its own `profiles.ini`, and an absolute
/// path there would let the sandbox choose where this unconfined process runs
/// certutil.
fn firefox_ini_profiles(ini: &str, root: &Path, allow_absolute: bool) -> Vec<PathBuf> {
    // (Path=, IsRelative) per section. `[Install…]` sections carry `Default=`
    // rather than `Path=`, so they fall out as `None`.
    let mut sections: Vec<(Option<&str>, bool)> = Vec::new();
    for line in ini.lines().map(str::trim) {
        if line.starts_with('[') {
            sections.push((None, true));
        } else if let (Some(section), Some((key, value))) =
            (sections.last_mut(), line.split_once('='))
        {
            match key.trim() {
                "Path" => section.0 = Some(value.trim()),
                "IsRelative" => section.1 = value.trim() != "0",
                _ => {}
            }
        }
    }
    sections
        .into_iter()
        .filter_map(|(path, relative)| {
            let path = Path::new(path?);
            if relative {
                let mut parts = path.components();
                let single = matches!(
                    (parts.next(), parts.next()),
                    (Some(std::path::Component::Normal(_)), None)
                );
                single.then(|| root.join(path))
            } else {
                (allow_absolute && path.is_absolute()).then(|| path.to_path_buf())
            }
        })
        .collect()
}

/// Whether `dir` is a real directory holding a real `cert9.db`, with neither
/// one a symlink. Two of the Firefox roots belong to a confined browser, which
/// can plant `profile -> /anywhere`; following it would point the unconfined
/// certutil at a directory the sandbox picked.
fn is_profile_store(dir: &Path) -> bool {
    fs::symlink_metadata(dir).is_ok_and(|m| m.is_dir())
        && fs::symlink_metadata(dir.join("cert9.db")).is_ok_and(|m| m.is_file())
}

/// Every Firefox profile under `home` that has a certificate database: the
/// direct children of each root, plus whatever its `profiles.ini` names (see
/// [`firefox_ini_profiles`]). Keyed on `cert9.db` so the siblings Firefox keeps
/// beside its profiles (`Crash Reports`, `Pending Pings`, `Profile Groups`) are
/// left alone.
fn firefox_profile_dbs(home: &Path) -> Vec<PathBuf> {
    let unconfined = home.join(".mozilla/firefox");
    let mut profiles = Vec::new();
    for root in firefox_profile_roots(home) {
        if let Ok(entries) = fs::read_dir(&root) {
            profiles.extend(entries.filter_map(|entry| entry.ok().map(|e| e.path())));
        }
        if let Ok(ini) = fs::read_to_string(root.join("profiles.ini")) {
            profiles.extend(firefox_ini_profiles(&ini, &root, root == unconfined));
        }
    }
    profiles.retain(|p| is_profile_store(p));
    profiles.sort();
    profiles.dedup();
    profiles
}

/// Launcher names of the Chromium-family browsers that read `~/.pki/nssdb`.
/// A snap build can put a launcher of the same name on `PATH` (`/snap/bin/chromium`),
/// but it reads a database under its own confined HOME instead, so
/// [`chromium_on_path`] skips launchers that resolve to `snap` itself. Flatpak
/// exports use reverse-DNS names, which never match.
const CHROMIUM_LAUNCHERS: &[&str] = &[
    "google-chrome",
    "google-chrome-stable",
    "google-chrome-beta",
    "google-chrome-unstable",
    "chromium",
    "brave-browser",
    "microsoft-edge",
    "microsoft-edge-stable",
    "vivaldi",
    "vivaldi-stable",
];

/// Whether any of [`CHROMIUM_LAUNCHERS`] is on `path` as something other than a
/// snap shim. Takes the `PATH` value so it is testable.
fn chromium_on_path(path: &OsStr) -> bool {
    std::env::split_paths(path).any(|dir| {
        CHROMIUM_LAUNCHERS.iter().any(|bin| {
            let launcher = dir.join(bin);
            launcher.is_file()
                && !fs::canonicalize(&launcher)
                    .is_ok_and(|target| target.file_name().is_some_and(|n| n == "snap"))
        })
    })
}

/// The two facts every browser-store decision reads, gathered once per call so
/// a test can supply its own.
struct BrowserEnv {
    home: PathBuf,
    chromium_installed: bool,
}

impl BrowserEnv {
    fn current() -> Option<Self> {
        Some(Self {
            home: std::env::var_os("HOME").map(PathBuf::from)?,
            chromium_installed: std::env::var_os("PATH").is_some_and(|p| chromium_on_path(&p)),
        })
    }

    fn chromium_db(&self) -> PathBuf {
        self.home.join(".pki/nssdb")
    }

    /// Whether a Chromium browser is installed and `~/.pki/nssdb` has no
    /// database yet.
    ///
    /// Chrome does not always create it at startup: a fresh Ubuntu VM with
    /// Chrome open on an intercepted host had none, so the app had set up trust
    /// with nowhere to put it. The browser opens the database once it exists, so
    /// [`ensure_trusted_nss`] creates it rather than waiting for one that may
    /// never come. Keyed on `cert9.db` rather than the directory, so a creation
    /// that got as far as `mkdir` - certutil missing, say - is retried once the
    /// user installs it, instead of leaving an empty directory that reads as
    /// done forever.
    fn chromium_db_missing(&self) -> bool {
        self.chromium_installed && !self.chromium_db().join("cert9.db").is_file()
    }

    /// Every browser NSS database that exists for this user: the Chromium ones
    /// from [`nss_db_candidates`] and each Firefox profile. Empty when no browser
    /// has a store here, so there is nothing to trust into.
    fn stores(&self) -> Vec<PathBuf> {
        nss_db_candidates(&self.home)
            .into_iter()
            .filter(|dir| dir.join("cert9.db").is_file())
            .chain(firefox_profile_dbs(&self.home))
            .collect()
    }
}

/// Create an empty, passwordless `sql:` database at `dir` for Chromium to open.
/// Owner-only, like the one Chrome would have made itself.
fn create_chromium_db(dir: &Path) {
    use std::os::unix::fs::DirBuilderExt;
    if let Err(e) = fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir)
    {
        eprintln!(
            "gate proxy: could not create the NSS store at {dir} ({e}); \
             Chromium-based browsers will reject intercepted hosts",
            dir = dir.display(),
        );
        return;
    }
    let created = StoreCopy::open(dir).and_then(|copy| {
        certutil(copy.path(), &["-N", "--empty-password"])?;
        copy.commit()
    });
    if let Err(e) = created {
        eprintln!(
            "gate proxy: could not initialise the NSS store at {dir} ({e}); \
             Chromium-based browsers will reject intercepted hosts{hint}",
            dir = dir.display(),
            hint = e.tools_hint(),
        );
    }
}

/// Why a `certutil` call did not succeed. The missing-binary case is split out
/// because it is the only one [`NSS_TOOLS_HINT`] answers: telling someone to
/// install a package they already have, because their database was locked,
/// sends them the wrong way at the one moment they are reading closely.
#[derive(Debug, PartialEq)]
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

/// How long one certutil call may take. `status()` reads every store on each
/// popover reopen, and a store on a stalled filesystem (an NFS home, a FUSE
/// mount) would otherwise hold that read for as long as the mount hangs - the
/// reason Windows bounds its own certutil the same way.
const CERTUTIL_TIMEOUT: Duration = Duration::from_secs(10);

/// One `certutil` invocation against a database, returning its stdout.
/// Unprivileged by construction: these are per-user stores, and running them
/// under the escalation the system anchor needs would write into root's HOME
/// instead of the user's.
fn certutil_output(db: &Path, args: &[&str]) -> std::result::Result<String, CertutilFailure> {
    let child = Command::new("certutil")
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
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn();
    let mut child = match child {
        Ok(child) => child,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Err(CertutilFailure::Missing),
        Err(e) => return Err(CertutilFailure::Failed(format!("running certutil: {e}"))),
    };
    // Drained on their own threads so a chatty call cannot fill a pipe and
    // stall before it exits, which the deadline below would misread as a hang.
    let drain = |pipe: Option<Box<dyn Read + Send>>| {
        std::thread::spawn(move || {
            let mut buf = Vec::new();
            if let Some(mut pipe) = pipe {
                let _ = pipe.read_to_end(&mut buf);
            }
            String::from_utf8_lossy(&buf).into_owned()
        })
    };
    let stdout = drain(
        child
            .stdout
            .take()
            .map(|p| Box::new(p) as Box<dyn Read + Send>),
    );
    let stderr = drain(
        child
            .stderr
            .take()
            .map(|p| Box::new(p) as Box<dyn Read + Send>),
    );
    let started = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if started.elapsed() < CERTUTIL_TIMEOUT => {
                std::thread::sleep(Duration::from_millis(10));
            }
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(CertutilFailure::Failed(format!(
                    "certutil {} timed out after {}s",
                    args.join(" "),
                    CERTUTIL_TIMEOUT.as_secs()
                )));
            }
            Err(e) => {
                return Err(CertutilFailure::Failed(format!(
                    "waiting for certutil: {e}"
                )))
            }
        }
    };
    let stdout = stdout.join().unwrap_or_default();
    let stderr = stderr.join().unwrap_or_default();
    if !status.success() {
        return Err(CertutilFailure::Failed(format!(
            "certutil {} exited {}: {}",
            args.join(" "),
            status,
            stderr.trim()
        )));
    }
    Ok(stdout)
}

/// [`certutil_output`] where only success matters.
fn certutil(db: &Path, args: &[&str]) -> std::result::Result<(), CertutilFailure> {
    certutil_output(db, args).map(|_| ())
}

/// The trust attributes of every entry `certutil -L` lists under `nickname`,
/// one per entry. Pure, so the parsing is testable without certutil.
///
/// The listing is `<nickname><padding><attrs>` per line, and the nickname has
/// spaces in it, so the line has to start with ours and the remainder be a
/// single token: that keeps a longer nickname with ours as its prefix - the dev
/// build's root beside the release one - from being read as ours.
fn nss_trust_attrs<'a>(listing: &'a str, nickname: &str) -> Vec<&'a str> {
    listing
        .lines()
        .filter_map(|line| line.trim_end().strip_prefix(nickname))
        .filter(|rest| rest.starts_with(char::is_whitespace))
        .map(str::trim)
        .filter(|attrs| !attrs.is_empty() && !attrs.contains(char::is_whitespace))
        .collect()
}

/// The files of an NSS `sql:` database that certutil reads and writes. The
/// directory's third file, `pkcs11.txt`, is the module database, and is
/// deliberately not among them; see [`StoreCopy`].
const NSS_DB_FILES: [&str; 2] = ["cert9.db", "key4.db"];

/// One store file as copied: its bytes and permission bits.
type Original = (Vec<u8>, u32);

/// A private working copy of one browser store. Every certutil call in this
/// module runs against one of these, never against a store in place.
///
/// **Why not in place.** NSS opening a `sql:` directory reads that directory's
/// `pkcs11.txt` as its module database and `dlopen`s every `library=` line in
/// it, read-only or not, and certutil has no flag to skip it. Two of the
/// Firefox roots and several Chromium ones belong to a snap or Flatpak browser,
/// which can write that file, so certutil run in place would load whatever
/// library a compromised sandbox named - unconfined, in the user's session, one
/// call from the Secret Service and this CA's private key. The copy is a fresh
/// 0700 directory with no `pkcs11.txt`, so NSS uses its built-in defaults and
/// loads nothing it was told to.
///
/// **And why through one directory handle.** The store is opened once with
/// `O_NOFOLLOW`, and both files are read and written relative to that handle,
/// `O_NOFOLLOW` too. A symlinked store, `cert9.db` or `key4.db` is refused
/// rather than followed, and a directory swapped for a symlink between our
/// calls changes nothing, because the handle still names the directory we
/// opened. Before this, certutil reopened the store by path for up to a dozen
/// runs, and a dangling `key4.db` link made NSS create a file wherever it
/// pointed.
///
/// Written back by [`StoreCopy::commit`]: per file, only where the copy
/// changed, as a temp file in the store renamed over the original - and only
/// while the original still holds what we copied, so a browser that wrote its
/// store meanwhile is not overwritten with an older copy. A browser with the
/// store open keeps the file it opened until it restarts, which is when it
/// would read our change anyway; anything it writes before then lands in the
/// file it has open, not the one we put in place.
struct StoreCopy {
    dir: OwnedFd,
    work: PathBuf,
    /// What each of [`NSS_DB_FILES`] held when copied.
    originals: Vec<(&'static str, Option<Original>)>,
}

impl StoreCopy {
    fn open(store: &Path) -> std::result::Result<Self, CertutilFailure> {
        Self::try_open(store).map_err(|e| {
            CertutilFailure::Failed(format!("could not copy {}: {e}", store.display()))
        })
    }

    fn try_open(store: &Path) -> std::io::Result<Self> {
        let mut copy = Self {
            dir: open_dir_nofollow(store)?,
            work: new_work_dir()?,
            originals: Vec::new(),
        };
        // From here a failure drops `copy`, which removes the work directory.
        for name in NSS_DB_FILES {
            let original = read_at(&copy.dir, name)?;
            if let Some((bytes, _)) = &original {
                fs::write(copy.work.join(name), bytes)?;
            }
            copy.originals.push((name, original));
        }
        Ok(copy)
    }

    /// The directory to hand certutil.
    fn path(&self) -> &Path {
        &self.work
    }

    fn commit(&self) -> std::result::Result<(), CertutilFailure> {
        self.try_commit()
            .map_err(|e| CertutilFailure::Failed(format!("could not write the store back: {e}")))
    }

    fn try_commit(&self) -> std::io::Result<()> {
        for (name, original) in &self.originals {
            let bytes = match fs::read(self.work.join(name)) {
                Ok(bytes) => bytes,
                // certutil did not produce this file, so there is nothing of
                // ours to put in the store.
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
                Err(e) => return Err(e),
            };
            let before = original.as_ref().map(|(bytes, _)| bytes.as_slice());
            if before == Some(bytes.as_slice()) {
                continue;
            }
            let now = read_at(&self.dir, name)?.map(|(bytes, _)| bytes);
            if now.as_deref() != before {
                return Err(std::io::Error::other(format!(
                    "{name} changed while Gate was writing it; try again"
                )));
            }
            let mode = original.as_ref().map_or(0o600, |(_, mode)| *mode);
            write_at(&self.dir, name, &bytes, mode)?;
        }
        Ok(())
    }
}

impl Drop for StoreCopy {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.work);
    }
}

/// A fresh, private directory for one [`StoreCopy`], under the app's own data
/// directory rather than `/tmp`: owner-only, no other user's files beside it,
/// and not a directory a snap or Flatpak browser can write unless it was given
/// the whole home - at which point it could write `~/.bashrc` too.
fn new_work_dir() -> std::io::Result<PathBuf> {
    use std::os::unix::fs::DirBuilderExt;
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let parent = env::app_support_dir()
        .map_err(std::io::Error::other)?
        .join("proxy")
        .join("nss-work");
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(&parent)?;
    let work = parent.join(format!(
        "{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    // Left by a run that crashed under the same process id.
    let _ = fs::remove_dir_all(&work);
    fs::DirBuilder::new().mode(0o700).create(&work)?;
    Ok(work)
}

fn c_string(bytes: &[u8]) -> std::io::Result<std::ffi::CString> {
    std::ffi::CString::new(bytes)
        .map_err(|_| std::io::Error::new(std::io::ErrorKind::InvalidInput, "path has a NUL byte"))
}

/// Open `path` as a directory, refusing a symlink in its last component.
fn open_dir_nofollow(path: &Path) -> std::io::Result<OwnedFd> {
    use std::os::unix::ffi::OsStrExt;
    let path = c_string(path.as_os_str().as_bytes())?;
    // SAFETY: `path` is a NUL-terminated string that outlives the call.
    let fd = unsafe {
        libc::open(
            path.as_ptr(),
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
        )
    };
    if fd < 0 {
        return Err(std::io::Error::last_os_error());
    }
    // SAFETY: `fd` was just opened and nothing else owns it.
    Ok(unsafe { OwnedFd::from_raw_fd(fd) })
}

/// Open `name` inside `dir`, never following a symlink.
fn open_at(
    dir: &OwnedFd,
    name: &str,
    flags: libc::c_int,
    mode: libc::c_uint,
) -> std::io::Result<fs::File> {
    let name = c_string(name.as_bytes())?;
    // SAFETY: `dir` is an open directory and `name` a NUL-terminated string,
    // both alive for the call.
    let fd = unsafe {
        libc::openat(
            dir.as_raw_fd(),
            name.as_ptr(),
            flags | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            mode,
        )
    };
    if fd < 0 {
        return Err(std::io::Error::last_os_error());
    }
    // SAFETY: `fd` was just opened and nothing else owns it.
    Ok(unsafe { fs::File::from_raw_fd(fd) })
}

/// The bytes and permission bits of the regular file `name` in `dir`,
/// or `None` where there is no such file. A symlink there is an error.
fn read_at(dir: &OwnedFd, name: &str) -> std::io::Result<Option<(Vec<u8>, u32)>> {
    use std::os::unix::fs::PermissionsExt;
    let mut file = match open_at(dir, name, libc::O_RDONLY, 0) {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e),
    };
    let meta = file.metadata()?;
    if !meta.is_file() {
        return Err(std::io::Error::other(format!(
            "{name} is not a regular file"
        )));
    }
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;
    Ok(Some((bytes, meta.permissions().mode() & 0o777)))
}

/// Replace `name` in `dir` with `bytes`: a temp file beside it, synced, then
/// renamed over it, so a browser opening the store sees the old file or the
/// new one and never half of either.
fn write_at(dir: &OwnedFd, name: &str, bytes: &[u8], mode: u32) -> std::io::Result<()> {
    let tmp = format!(".gate-{name}.{}.tmp", std::process::id());
    let tmp_c = c_string(tmp.as_bytes())?;
    let name_c = c_string(name.as_bytes())?;
    // A temp file left by a crash under the same process id.
    // SAFETY: `dir` is open and `tmp_c` NUL-terminated, both alive for the call.
    unsafe { libc::unlinkat(dir.as_raw_fd(), tmp_c.as_ptr(), 0) };
    let mut file = open_at(
        dir,
        &tmp,
        libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL,
        mode,
    )?;
    let written = file.write_all(bytes).and_then(|()| file.sync_all());
    let renamed = written.and_then(|()| {
        // SAFETY: as above; `renameat` replaces the entry and follows nothing.
        let rc = unsafe {
            libc::renameat(
                dir.as_raw_fd(),
                tmp_c.as_ptr(),
                dir.as_raw_fd(),
                name_c.as_ptr(),
            )
        };
        if rc == 0 {
            Ok(())
        } else {
            Err(std::io::Error::last_os_error())
        }
    });
    if renamed.is_err() {
        // SAFETY: as above.
        unsafe { libc::unlinkat(dir.as_raw_fd(), tmp_c.as_ptr(), 0) };
    }
    renamed
}

/// What one browser store holds under our nickname.
#[derive(Debug, PartialEq)]
enum NssEntry {
    /// Exactly our current CA, trusted to identify websites.
    Trusted,
    /// Our current CA, with the website trust taken off - which only the
    /// browser's own certificate manager does.
    Distrusted,
    /// Something under our nickname that is not exactly our current CA once: a
    /// root from before a regeneration, or two entries an older build appended.
    Stale,
    /// Nothing under our nickname.
    Absent,
    /// We could not ask.
    Unknown(CertutilFailure),
}

/// Read what the store at `dir` holds under our nickname, through a
/// [`StoreCopy`] - never in place.
fn nss_entry(dir: &Path, pem: &str) -> NssEntry {
    match StoreCopy::open(dir) {
        Ok(copy) => nss_entry_in(copy.path(), pem),
        Err(e) => NssEntry::Unknown(e),
    }
}

/// Read what the database at `db` - a [`StoreCopy`]'s directory - holds under
/// our nickname.
fn nss_entry_in(db: &Path, pem: &str) -> NssEntry {
    let listing = match certutil_output(db, &["-L"]) {
        Ok(listing) => listing,
        Err(e) => return NssEntry::Unknown(e),
    };
    let attrs = nss_trust_attrs(&listing, ca_common_name());
    let [attrs] = attrs.as_slice() else {
        return if attrs.is_empty() {
            NssEntry::Absent
        } else {
            NssEntry::Stale
        };
    };
    match certutil_output(db, &["-L", "-n", ca_common_name(), "-a"]) {
        Ok(held) if pem_body(&held) == pem_body(pem) => {
            // The SSL field comes first; `C` is "trusted CA for websites".
            if attrs.split(',').next().is_some_and(|ssl| ssl.contains('C')) {
                NssEntry::Trusted
            } else {
                NssEntry::Distrusted
            }
        }
        Ok(_) => NssEntry::Stale,
        Err(e) => NssEntry::Unknown(e),
    }
}

/// What [`ensure_trusted_nss`] does with one store.
#[derive(Debug, PartialEq)]
enum NssAction {
    /// Nothing.
    Leave,
    /// Nothing to the store, but record it: it already holds our CA.
    Adopt,
    /// Add our CA (replacing whatever is under our nickname) and record it.
    Write,
}

/// Decide what to do with a store from what it holds and whether this CA was
/// written to it before (see [`nss_ledger_path`]). Pure, so the policy is
/// testable without certutil.
///
/// A recorded store that no longer holds our CA, or holds it distrusted, was
/// changed in the browser's own certificate manager, and putting it back on the
/// next enable would overrule the user on a root that can sign for the
/// intercepted hosts. So it is left alone - as is a distrusted entry we never
/// recorded, since only the user takes trust off. A stale entry is ours and
/// outdated, so it is rewritten whatever the record says.
fn nss_action(entry: &NssEntry, recorded: bool) -> NssAction {
    match entry {
        NssEntry::Trusted | NssEntry::Distrusted if recorded => NssAction::Leave,
        NssEntry::Trusted | NssEntry::Distrusted => NssAction::Adopt,
        NssEntry::Absent if recorded => NssAction::Leave,
        NssEntry::Absent | NssEntry::Stale => NssAction::Write,
        NssEntry::Unknown(_) => NssAction::Leave,
    }
}

/// Whether a store is as it should be, for [`nss_ca_trusted`]. A store the user
/// changed in the browser counts as fine: the card that reads this offers a
/// Retry, and Retry deliberately does not override the user (see
/// [`nss_action`]), so a store it would never touch must not keep the card up.
fn nss_store_ok(entry: &NssEntry, recorded: bool) -> bool {
    match entry {
        NssEntry::Trusted | NssEntry::Distrusted => true,
        NssEntry::Absent => recorded,
        NssEntry::Stale | NssEntry::Unknown(_) => false,
    }
}

/// The record of stores this CA has been written to or found in, one path per
/// line, beside the cert. What tells "never had it" from "had it and the user
/// took it out" (see [`nss_action`]). Cleared with the CA, on removal and on
/// regeneration, because a new root is a new question.
fn nss_ledger_path() -> Result<PathBuf> {
    Ok(env::ca_material_dir()?.join("nss-stores"))
}

fn read_nss_ledger() -> Vec<PathBuf> {
    nss_ledger_path()
        .ok()
        .and_then(|path| fs::read_to_string(path).ok())
        .map(|body| {
            body.lines()
                .filter(|l| !l.is_empty())
                .map(PathBuf::from)
                .collect()
        })
        .unwrap_or_default()
}

/// Add `dir` to the ledger, both in memory and on disk.
fn record_in_nss_ledger(ledger: &mut Vec<PathBuf>, dir: &Path) {
    if ledger.iter().any(|d| d == dir) {
        return;
    }
    ledger.push(dir.to_path_buf());
    let body: String = ledger
        .iter()
        .map(|d| format!("{}\n", d.display()))
        .collect();
    let written = nss_ledger_path().and_then(|path| {
        fs::write(&path, body).with_context(|| format!("writing {}", path.display()))
    });
    if let Err(e) = written {
        eprintln!(
            "gate proxy: could not record the NSS stores holding the CA ({e:#}); \
             a CA removed in a browser may be added back on the next enable"
        );
    }
}

fn clear_nss_ledger() {
    if let Ok(path) = nss_ledger_path() {
        let _ = fs::remove_file(path);
    }
}

/// How many browser NSS databases this process has added the CA to. A browser
/// only picks a new root up when it restarts, so the GUI raises its restart
/// notice whenever this moves - including for the write the startup reconcile
/// makes before any window has asked for anything, which is why it counts from
/// zero per process rather than being a flag someone clears. It counts stores,
/// not trust actions: one enable that writes Chrome and Firefox adds two, and
/// the GUI only ever asks whether it went up.
static NSS_WRITES: AtomicU64 = AtomicU64::new(0);

/// [`NSS_WRITES`], for `ProxyState::ca_nss_writes`.
pub fn nss_writes() -> u64 {
    NSS_WRITES.load(Ordering::Relaxed)
}

/// Whether every browser NSS database is as it should be, for the diagnostics
/// report and the Home card. `Some(false)` beside a `ca_trusted` of true is the
/// state this module learned the hard way: the OS trusts the root, the browsers
/// do not, and only they fail. A missing `certutil` reads as false, which is
/// accurate - without it [`ensure_trusted_nss`] never installed anything. So
/// does a Chromium browser with no database yet: that is the state
/// [`ensure_trusted_nss`] repairs, and reading it as "does not apply" hid it
/// entirely. A store the user changed in the browser reads as fine; see
/// [`nss_store_ok`].
///
/// `None` where the question does not apply: no browser keeps a store for this
/// user. Also `None` when the cert itself cannot be read, which
/// `ca_cert_present` already reports.
pub fn nss_ca_trusted() -> Option<bool> {
    let browsers = BrowserEnv::current()?;
    let dirs = browsers.stores();
    let missing = browsers.chromium_db_missing();
    if dirs.is_empty() && !missing {
        return None;
    }
    let pem = cert_path().ok().and_then(|p| fs::read_to_string(p).ok())?;
    let ledger = read_nss_ledger();
    Some(
        !missing
            && dirs
                .iter()
                .all(|dir| nss_store_ok(&nss_entry(dir, &pem), ledger.contains(dir))),
    )
}

/// One line per browser store and what it holds, for the diagnostics report.
/// `ca_nss_trusted` says whether anything is wrong; this says which store, so a
/// support report can tell a missing Chrome database from a Firefox profile
/// that refused the write.
pub fn nss_store_report() -> Vec<String> {
    let Some(browsers) = BrowserEnv::current() else {
        return Vec::new();
    };
    let mut lines = Vec::new();
    if browsers.chromium_db_missing() {
        lines.push(format!(
            "{}: no database yet (a Chromium browser is installed)",
            browsers.chromium_db().display()
        ));
    }
    let Some(pem) = cert_path().ok().and_then(|p| fs::read_to_string(p).ok()) else {
        return lines;
    };
    let ledger = read_nss_ledger();
    for dir in browsers.stores() {
        let recorded = ledger.contains(&dir);
        let state = match nss_entry(&dir, &pem) {
            NssEntry::Trusted => "trusted".to_string(),
            NssEntry::Distrusted => "distrusted in the browser".to_string(),
            NssEntry::Stale => "outdated".to_string(),
            NssEntry::Absent if recorded => "removed in the browser".to_string(),
            NssEntry::Absent => "missing".to_string(),
            NssEntry::Unknown(e) => format!("unreadable ({e})"),
        };
        lines.push(format!("{}: {state}", dir.display()));
    }
    lines
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

/// Delete every entry under our nickname in `db`, returning whether any went.
/// `certutil -D` removes one entry per call, and `-A` appends under a duplicate
/// nickname rather than replacing, so a store can hold more than one. Bounded,
/// because a store that keeps reporting success is broken in a way more calls
/// will not fix.
fn drop_nss_entries(db: &Path) -> bool {
    let mut dropped = false;
    for _ in 0..8 {
        if certutil(db, &["-D", "-n", ca_common_name()]).is_err() {
            break;
        }
        dropped = true;
    }
    dropped
}

/// Add the CA to every browser NSS database that should have it, so Chromium
/// and Firefox accept the leaves the engine mints. Creates `~/.pki/nssdb` first
/// when a Chromium browser is installed without one (see
/// [`BrowserEnv::chromium_db_missing`]), and leaves alone any store the user
/// changed in the browser (see [`nss_action`]).
///
/// Best-effort and infallible by design: the system anchor is what trust really
/// rests on, and a browser-specific store that cannot be written must not fail
/// enabling the proxy. Failures are reported rather than swallowed, because the
/// symptom otherwise lands in the browser as a certificate error with nothing
/// connecting it to Gate.
fn ensure_trusted_nss() {
    let Some(browsers) = BrowserEnv::current() else {
        return;
    };
    if browsers.chromium_db_missing() {
        create_chromium_db(&browsers.chromium_db());
    }
    let dirs = browsers.stores();
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
    let mut ledger = read_nss_ledger();
    for dir in dirs {
        let entry = nss_entry(&dir, &cert_pem);
        match nss_action(&entry, ledger.contains(&dir)) {
            NssAction::Leave => {
                if let NssEntry::Unknown(e) = &entry {
                    eprintln!(
                        "gate proxy: could not read the NSS store at {dir} ({e}); \
                         the browser reading it may reject intercepted hosts{hint}",
                        dir = dir.display(),
                        hint = e.tools_hint(),
                    );
                }
            }
            NssAction::Adopt => record_in_nss_ledger(&mut ledger, &dir),
            NssAction::Write => {
                // `-t "C,,"`: trusted to issue SSL server certs, with no S/MIME
                // and no object-signing trust. The same flags mkcert uses for
                // the same job.
                let args = ["-A", "-t", "C,,", "-n", ca_common_name(), "-i", &cert_arg];
                // On a private copy, committed only once the copy holds the
                // CA: see `StoreCopy` for why certutil never runs in place. A
                // failure anywhere leaves the store exactly as it was.
                let written = StoreCopy::open(&dir).and_then(|copy| {
                    drop_nss_entries(copy.path());
                    certutil(copy.path(), &args)?;
                    match nss_entry_in(copy.path(), &cert_pem) {
                        NssEntry::Trusted => copy.commit(),
                        NssEntry::Unknown(e) => Err(e),
                        _ => Err(CertutilFailure::Failed(
                            "certutil -A reported success and the store does not hold the CA"
                                .to_string(),
                        )),
                    }
                });
                match written {
                    Ok(()) => {
                        NSS_WRITES.fetch_add(1, Ordering::Relaxed);
                        record_in_nss_ledger(&mut ledger, &dir);
                    }
                    Err(e) => {
                        // A refusal from certutil itself is most often a
                        // Firefox Primary Password: changing trust needs it,
                        // and stdin is closed.
                        let hint = match &e {
                            CertutilFailure::Missing => e.tools_hint(),
                            CertutilFailure::Failed(_) => " - if this is a Firefox profile with a \
                                 Primary Password, import the certificate in Firefox's own \
                                 certificate settings"
                                .to_string(),
                        };
                        eprintln!(
                            "gate proxy: could not add the CA to the NSS store at {dir} ({e}); \
                             the browser reading it will reject intercepted hosts{hint}",
                            dir = dir.display(),
                        );
                    }
                }
            }
        }
    }
}

/// Drop the CA from every browser NSS database.
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
    let Some(browsers) = BrowserEnv::current() else {
        return;
    };
    for dir in browsers.stores() {
        // Through a private copy, like every certutil call here; see
        // `StoreCopy`. A store that cannot be copied is one we could not even
        // look in, which is the missing-certutil case's sibling.
        let copy = match StoreCopy::open(&dir) {
            Ok(copy) => copy,
            Err(e) => {
                eprintln!(
                    "gate proxy: could not open the NSS store at {dir} to remove the CA ({e}); \
                     the browser reading it may still trust it",
                    dir = dir.display(),
                );
                continue;
            }
        };
        match certutil_output(copy.path(), &["-L"]) {
            Ok(listing) if !nss_trust_attrs(&listing, ca_common_name()).is_empty() => {
                drop_nss_entries(copy.path());
                let gone = certutil_output(copy.path(), &["-L"])
                    .is_ok_and(|listing| nss_trust_attrs(&listing, ca_common_name()).is_empty());
                if !gone || copy.commit().is_err() {
                    eprintln!(
                        "gate proxy: could not remove the CA from the NSS store at {dir}; \
                         the browser reading it still trusts it",
                        dir = dir.display(),
                    );
                }
            }
            Ok(_) | Err(CertutilFailure::Failed(_)) => {}
            Err(e @ CertutilFailure::Missing) => eprintln!(
                "gate proxy: could not remove the CA from the NSS store at {dir} ({e}); \
                 the browser reading it may still trust it - {NSS_TOOLS_HINT}",
                dir = dir.display(),
            ),
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
    clear_nss_ledger();
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

    /// Ubuntu's default Firefox is the snap, which keeps profiles under its own
    /// confined HOME and ignores the system anchor. Missing that root is the
    /// bug this covers: every intercepted host failing in Firefox while curl
    /// verifies the same leaf fine.
    #[test]
    fn the_firefox_roots_cover_the_snap_and_flatpak_homes() {
        let roots = firefox_profile_roots(std::path::Path::new("/home/u"));
        for expected in [
            "/home/u/.mozilla/firefox",
            "/home/u/snap/firefox/common/.mozilla/firefox",
            "/home/u/.var/app/org.mozilla.firefox/.mozilla/firefox",
        ] {
            assert!(
                roots.iter().any(|r| r == std::path::Path::new(expected)),
                "{expected} missing from {roots:?}"
            );
        }
        let home = std::path::Path::new("/tmp/someone");
        for root in firefox_profile_roots(home) {
            assert!(root.starts_with(home), "{root:?} escaped {home:?}");
        }
    }

    /// A profile root also holds `Crash Reports`, `Pending Pings` and the like.
    /// certutil pointed at one of those would create a fresh database there
    /// that nothing reads, so only directories that already have a `cert9.db`
    /// count - the same rule that keeps the Chromium list from creating stores.
    #[test]
    fn the_firefox_dbs_are_only_profiles_with_a_cert_store() {
        let home = std::env::temp_dir().join(format!("gate_ff_{}", std::process::id()));
        let root = home.join("snap/firefox/common/.mozilla/firefox");
        let profile = root.join("qp1tx1a3.default");
        fs::create_dir_all(&profile).unwrap();
        fs::write(profile.join("cert9.db"), b"").unwrap();
        fs::create_dir_all(root.join("Crash Reports")).unwrap();

        let dbs = firefox_profile_dbs(&home);
        let _ = fs::remove_dir_all(&home);
        assert_eq!(dbs, vec![profile]);
    }

    /// A fresh directory under the temp root, unique per test.
    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("gate_nss_{tag}_{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A confined Firefox can write its own profile root, so a symlinked
    /// "profile" there would point the unconfined certutil wherever the sandbox
    /// liked. Real directories count; links to them do not.
    #[test]
    fn the_firefox_dbs_skip_symlinked_profiles() {
        let home = scratch("ff_link");
        let root = home.join("snap/firefox/common/.mozilla/firefox");
        fs::create_dir_all(&root).unwrap();
        let elsewhere = home.join("elsewhere");
        fs::create_dir_all(&elsewhere).unwrap();
        fs::write(elsewhere.join("cert9.db"), b"").unwrap();
        std::os::unix::fs::symlink(&elsewhere, root.join("planted.default")).unwrap();

        let dbs = firefox_profile_dbs(&home);
        let _ = fs::remove_dir_all(&home);
        assert!(dbs.is_empty(), "{dbs:?}");
    }

    /// `profiles.ini` names profiles Firefox's own listing would not show us: a
    /// relative one in a subdirectory of the root's choosing, or an absolute one
    /// the user moved. Only a single plain component counts as relative, and an
    /// absolute path only where the caller allows it (the unconfined root).
    #[test]
    fn the_profiles_ini_paths_stay_inside_what_the_caller_allows() {
        let root = Path::new("/home/u/.mozilla/firefox");
        let ini = "[Install4F96D1932A9F858E]\nDefault=a.default\n\n\
                   [Profile0]\nName=a\nIsRelative=1\nPath=a.default\n\n\
                   [Profile1]\nIsRelative=0\nPath=/data/ff/b\n\n\
                   [Profile2]\nIsRelative=1\nPath=../escape\n\n\
                   [Profile3]\nIsRelative=1\nPath=nested/c\n";
        assert_eq!(
            firefox_ini_profiles(ini, root, true),
            vec![root.join("a.default"), PathBuf::from("/data/ff/b")]
        );
        assert_eq!(
            firefox_ini_profiles(ini, root, false),
            vec![root.join("a.default")]
        );
    }

    /// A launcher on PATH means Chromium reads `~/.pki/nssdb`, except a snap
    /// shim of the same name, which resolves to `snap` and reads a database in
    /// its own confined HOME.
    #[test]
    fn a_chromium_launcher_counts_unless_it_is_a_snap_shim() {
        let dir = scratch("path");
        let real = dir.join("real");
        let snapbin = dir.join("snapbin");
        fs::create_dir_all(&real).unwrap();
        fs::create_dir_all(&snapbin).unwrap();
        fs::write(real.join("google-chrome"), b"").unwrap();
        fs::write(dir.join("snap"), b"").unwrap();
        std::os::unix::fs::symlink(dir.join("snap"), snapbin.join("chromium")).unwrap();

        let found = chromium_on_path(real.as_os_str());
        let shim = chromium_on_path(snapbin.as_os_str());
        let empty = chromium_on_path(dir.as_os_str());
        let _ = fs::remove_dir_all(&dir);
        assert!(found);
        assert!(!shim);
        assert!(!empty);
    }

    /// Keyed on `cert9.db`, not the directory: a creation that stopped after
    /// `mkdir` has to be retried, or the user who installs certutil as told is
    /// left with an empty directory that reads as done for good.
    #[test]
    fn the_chromium_db_is_missing_until_it_has_a_cert_store() {
        let home = scratch("pki");
        let browsers = |chromium_installed| BrowserEnv {
            home: home.clone(),
            chromium_installed,
        };
        assert!(browsers(true).chromium_db_missing());
        fs::create_dir_all(home.join(".pki/nssdb")).unwrap();
        let half_made = browsers(true).chromium_db_missing();
        fs::write(home.join(".pki/nssdb/cert9.db"), b"").unwrap();
        let made = browsers(true).chromium_db_missing();
        let _ = fs::remove_dir_all(&home);
        assert!(half_made);
        assert!(!made);
        assert!(!browsers(false).chromium_db_missing());
    }

    /// The listing pads the nickname with spaces and the nickname has spaces of
    /// its own, so the parse keys on "ours, then one token". A longer nickname
    /// that starts with ours (the dev root) must not be read as ours, and two
    /// entries under ours must read as two.
    #[test]
    fn the_listing_parse_finds_only_our_entries() {
        let listing = "\nCertificate Nickname                                         Trust Attributes\n\
                       \x20                                                            SSL,S/MIME,JAR/XPI\n\n\
                       Gate CA                                                      C,,\n\
                       Gate CA (dev)                                                C,,\n\
                       Gate CA                                                      ,,\n\
                       Other Root                                                   CT,C,C\n";
        assert_eq!(nss_trust_attrs(listing, "Gate CA"), vec!["C,,", ",,"]);
        assert_eq!(nss_trust_attrs(listing, "Gate CA (dev)"), vec!["C,,"]);
        assert!(nss_trust_attrs(listing, "Missing").is_empty());
    }

    /// The database we create stands in for the one Chrome would have made,
    /// which is owner-only. The directory is made before certutil runs, so this
    /// holds whether or not certutil is installed where the test runs.
    #[test]
    fn the_created_chromium_db_is_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let home = scratch("mode");
        let _support = SupportDir::new(&home);
        let db = home.join(".pki/nssdb");
        create_chromium_db(&db);
        let mode = fs::metadata(&db).map(|m| m.permissions().mode() & 0o777);
        let _ = fs::remove_dir_all(&home);
        assert_eq!(mode.unwrap(), 0o700);
    }

    /// Points the app data directory - where a [`StoreCopy`] makes its work
    /// directory - at a scratch folder for one test, under the lock every
    /// path-redirecting test takes.
    struct SupportDir {
        _lock: std::sync::MutexGuard<'static, ()>,
    }

    impl SupportDir {
        fn new(root: &Path) -> Self {
            let lock = crate::env::path_env_lock();
            crate::env::set_app_support_dir_for_tests(Some(root.join("support")));
            Self { _lock: lock }
        }
    }

    impl Drop for SupportDir {
        fn drop(&mut self) {
            crate::env::set_app_support_dir_for_tests(None);
        }
    }

    /// The sandbox escape the copy exists for: NSS loads every `library=` in
    /// a store's `pkcs11.txt`, and a snap or Flatpak browser can write that
    /// file. The directory certutil is handed must be a different one, with no
    /// `pkcs11.txt` in it.
    #[test]
    fn a_store_copy_leaves_the_module_database_behind() {
        let root = scratch("modules");
        let _support = SupportDir::new(&root);
        let store = root.join("store");
        fs::create_dir_all(&store).unwrap();
        fs::write(store.join("cert9.db"), b"cert").unwrap();
        fs::write(store.join("key4.db"), b"key").unwrap();
        fs::write(store.join("pkcs11.txt"), "library=/nonexistent/evil.so\n").unwrap();

        let copy = StoreCopy::open(&store).expect("open");
        let work = copy.path().to_path_buf();
        assert_ne!(work, store);
        assert_eq!(fs::read(work.join("cert9.db")).unwrap(), b"cert");
        assert_eq!(fs::read(work.join("key4.db")).unwrap(), b"key");
        assert!(!work.join("pkcs11.txt").exists());
        drop(copy);
        assert!(!work.exists(), "the work directory outlived the copy");
        let _ = fs::remove_dir_all(&root);
    }

    /// A symlinked store or `key4.db` is refused rather than followed: NSS
    /// would otherwise create or open whatever file it points at.
    #[test]
    fn a_store_copy_refuses_symlinks() {
        let root = scratch("links");
        let _support = SupportDir::new(&root);
        let store = root.join("store");
        fs::create_dir_all(&store).unwrap();
        fs::write(store.join("cert9.db"), b"cert").unwrap();
        std::os::unix::fs::symlink(root.join("target"), store.join("key4.db")).unwrap();
        assert!(StoreCopy::open(&store).is_err());
        assert!(!root.join("target").exists());

        let linked = root.join("linked-store");
        std::os::unix::fs::symlink(&store, &linked).unwrap();
        fs::remove_file(store.join("key4.db")).unwrap();
        assert!(
            StoreCopy::open(&linked).is_err(),
            "a symlinked store was followed"
        );
        let _ = fs::remove_dir_all(&root);
    }

    /// The commit writes back only what changed, and refuses to overwrite a
    /// store the browser wrote meanwhile.
    #[test]
    fn a_store_copy_commits_changes_and_refuses_a_store_that_moved() {
        let root = scratch("commit");
        let _support = SupportDir::new(&root);
        let store = root.join("store");
        fs::create_dir_all(&store).unwrap();
        fs::write(store.join("cert9.db"), b"old").unwrap();
        fs::write(store.join("key4.db"), b"key").unwrap();

        let copy = StoreCopy::open(&store).expect("open");
        fs::write(copy.path().join("cert9.db"), b"new").unwrap();
        copy.commit().expect("commit");
        assert_eq!(fs::read(store.join("cert9.db")).unwrap(), b"new");
        assert_eq!(fs::read(store.join("key4.db")).unwrap(), b"key");
        drop(copy);

        let copy = StoreCopy::open(&store).expect("open");
        fs::write(copy.path().join("cert9.db"), b"ours").unwrap();
        fs::write(store.join("cert9.db"), b"the browser's").unwrap();
        assert!(copy.commit().is_err(), "an older copy overwrote the store");
        assert_eq!(fs::read(store.join("cert9.db")).unwrap(), b"the browser's");
        drop(copy);
        let _ = fs::remove_dir_all(&root);
    }

    /// The policy that keeps a CA the user removed in a browser removed: a
    /// recorded store that lost it, or holds it distrusted, is left alone and
    /// does not count against the status. A never-recorded store is written,
    /// and a stale entry is rewritten whatever the record says.
    #[test]
    fn a_store_the_user_changed_is_left_alone_and_not_held_against_them() {
        use NssAction::*;
        let cases = [
            (NssEntry::Trusted, false, Adopt, true),
            (NssEntry::Trusted, true, Leave, true),
            (NssEntry::Distrusted, false, Adopt, true),
            (NssEntry::Distrusted, true, Leave, true),
            (NssEntry::Absent, false, Write, false),
            (NssEntry::Absent, true, Leave, true),
            (NssEntry::Stale, false, Write, false),
            (NssEntry::Stale, true, Write, false),
            (
                NssEntry::Unknown(CertutilFailure::Missing),
                false,
                Leave,
                false,
            ),
        ];
        for (entry, recorded, action, ok) in cases {
            assert_eq!(
                nss_action(&entry, recorded),
                action,
                "{entry:?} recorded={recorded}"
            );
            assert_eq!(
                nss_store_ok(&entry, recorded),
                ok,
                "{entry:?} recorded={recorded}"
            );
        }
    }
}
