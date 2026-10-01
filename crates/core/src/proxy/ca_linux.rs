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
    // would otherwise never reach the browser stores, and Chrome and Firefox
    // would keep rejecting intercepted hosts with no way to recover short of
    // removing and re-adding trust.
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
    // [`ensure_trusted`] has it: the browsers read their own stores and never
    // the system one. A headless host has no such store and this is a no-op
    // there, which is the case this function was written for - but the flag is
    // reachable from a desktop, and leaving it out meant `trust-ca
    // --system-trust` installed the anchor, wrote no browser store, recorded no
    // reading, and left the window telling the user to reopen a browser that
    // was never going to work.
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
/// [`untrust`]: same anchor, same rebuild, non-interactive escalation. The
/// browser stores go too: [`ensure_trusted_system`] writes them, and so may the
/// GUI or a plain `trust-ca`, and `--system-trust` is the removal the CLI names
/// for this host. That step needs no privilege, so it keeps to the no-prompt
/// contract.
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
/// intercepted host exactly the way Chromium did: measured on a fresh Ubuntu VM,
/// the snap Firefox and the Google Chrome .deb both refusing claude.ai while
/// curl through the engine verified the same leaf.
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

/// The profiles a `profiles.ini` names by absolute path - a profile the user
/// moved with Firefox's profile manager, which the directory listing in
/// [`firefox_profile_dbs`] cannot see. Pure, so it is testable without a
/// Firefox.
///
/// Relative entries are skipped: Firefox writes them as a single name under the
/// root, which the listing already finds, and anything longer could walk out of
/// it. Absolute ones count only when `allow_absolute`, which the caller grants
/// for the unconfined root alone: a snap or Flatpak Firefox can write its own
/// `profiles.ini`, and an absolute path there would let the sandbox choose a
/// directory for this unconfined process to open.
fn firefox_ini_profiles(ini: &str, allow_absolute: bool) -> Vec<PathBuf> {
    if !allow_absolute {
        return Vec::new();
    }
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
            (!relative && path.is_absolute()).then(|| path.to_path_buf())
        })
        .collect()
}

/// Whether `dir` is a real directory holding a real `cert9.db`, with neither
/// one a symlink. A cheap first filter for enumeration; the guarantee that
/// nothing a sandbox plants is followed is [`StoreCopy`]'s, which opens the
/// store through one `O_NOFOLLOW` handle.
fn is_profile_store(dir: &Path) -> bool {
    fs::symlink_metadata(dir).is_ok_and(|m| m.is_dir())
        && fs::symlink_metadata(dir.join("cert9.db")).is_ok_and(|m| m.is_file())
}

/// Every Firefox profile under `home` that has a certificate database: the
/// direct children of each root, plus the moved profiles the unconfined root's
/// `profiles.ini` names (see [`firefox_ini_profiles`]). Keyed on `cert9.db` so the siblings Firefox keeps
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
            profiles.extend(firefox_ini_profiles(&ini, root == unconfined));
        }
    }
    profiles.retain(|p| is_profile_store(p));
    profiles.sort();
    profiles.dedup();
    profiles
}

/// Launcher names of the Chromium-family browsers that read `~/.pki/nssdb`.
/// A snap build can put a launcher of the same name on `PATH`
/// (`/snap/bin/chromium`), but it reads a database under its own confined HOME
/// instead, so [`chromium_on_path`] skips launchers that resolve to `snap`
/// itself. Flatpak exports use reverse-DNS names, which never match.
const CHROMIUM_LAUNCHERS: &[&str] = &[
    "google-chrome",
    "google-chrome-stable",
    "google-chrome-beta",
    "google-chrome-unstable",
    "chromium",
    // Fedora's and older Debian's name for the same package.
    "chromium-browser",
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

/// Test seam: whether a Chromium browser counts as installed. `false` unless a
/// test says otherwise, so the suite does not depend on what the machine
/// running it has on `PATH` - a CI runner with Chrome installed would otherwise
/// send every NSS fixture through the database creation below.
#[cfg(test)]
static CHROMIUM_INSTALLED_OVERRIDE: Mutex<Option<bool>> = Mutex::new(None);

/// Whether a Chromium-family browser that reads `~/.pki/nssdb` is installed.
fn chromium_installed() -> bool {
    #[cfg(test)]
    return CHROMIUM_INSTALLED_OVERRIDE
        .lock()
        .expect("chromium override mutex poisoned")
        .unwrap_or(false);
    #[cfg(not(test))]
    std::env::var_os("PATH").is_some_and(|path| chromium_on_path(&path))
}

/// Test seam: the home [`nss_db_dirs`] enumerates under, absent in every normal
/// build. A `Mutex` static rather than a `HOME` mutation for the same reason
/// [`CERTUTIL_OVERRIDE`] is one.
#[cfg(test)]
static NSS_HOME_OVERRIDE: Mutex<Option<PathBuf>> = Mutex::new(None);

/// The home every browser store hangs off.
///
/// [`env::home`] rather than `$HOME` directly, so the e2e harness's redirected
/// home redirects this too. Reading the variable raw left the one path in this
/// module that could reach the developer's own `~/.pki/nssdb` from a test run.
fn nss_home() -> Option<PathBuf> {
    #[cfg(test)]
    if let Some(home) = NSS_HOME_OVERRIDE
        .lock()
        .expect("nss home override mutex poisoned")
        .clone()
    {
        return Some(home);
    }
    env::home().ok()
}

/// `~/.pki/nssdb`, the database a distro-packaged Chromium reads.
fn chromium_db(home: &Path) -> PathBuf {
    home.join(".pki/nssdb")
}

/// Whether a Chromium browser is installed and `~/.pki/nssdb` has no database
/// yet.
///
/// Chrome does not always create it at startup: a fresh Ubuntu VM with Chrome
/// open on an intercepted host had none, so trust was set up with nowhere to
/// put it, every candidate filtered out, and the reading said nothing applied.
/// The browser opens the database once it exists, so [`ensure_trusted_nss`]
/// creates it rather than waiting for one that may never come. Keyed on
/// `cert9.db` rather than the directory, so a creation that got as far as
/// `mkdir` - certutil missing, say - is retried once the user installs it,
/// instead of leaving an empty directory that reads as done forever.
fn chromium_db_missing(home: &Path) -> bool {
    !chromium_db(home).join("cert9.db").is_file() && chromium_installed()
}

/// Every browser NSS database that exists for this user: the Chromium ones
/// from [`nss_db_candidates`] and each Firefox profile. Empty when no browser
/// has a store here, so there is nothing to trust into.
///
/// A `~/.pki/nssdb` that [`chromium_db_missing`] still calls missing - a
/// directory left by a creation that stopped before `certutil -N` - is not one
/// of them. It is reported once, as the missing database it is, rather than a
/// second time as a store that refused.
fn nss_db_dirs() -> Vec<PathBuf> {
    let Some(home) = nss_home() else {
        return Vec::new();
    };
    let half_made = chromium_db_missing(&home).then(|| chromium_db(&home));
    nss_db_candidates(&home)
        .into_iter()
        .filter(|dir| dir.is_dir() && Some(dir) != half_made.as_ref())
        .chain(firefox_profile_dbs(&home))
        .collect()
}

/// Create an empty, passwordless `sql:` database at `dir` for Chromium to open.
/// Owner-only, like the one Chrome would have made itself. Made in a
/// [`StoreCopy`] and committed, like every other write here.
fn create_chromium_db(dir: &Path) -> std::result::Result<(), CertutilFailure> {
    use std::os::unix::fs::DirBuilderExt;
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir)
        .map_err(|e| CertutilFailure::Failed(format!("creating {}: {e}", dir.display())))?;
    let mut copy = StoreCopy::open(dir)?;
    certutil(copy.path(), &["-N", "--empty-password"])?;
    copy.commit()
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

/// Whether a `certutil` is there to run. Only for the one reading that has no
/// store to ask - a Chromium with no database yet - where "no certutil" and "not
/// written" are otherwise indistinguishable and want different fixes.
fn certutil_available() -> bool {
    certutil_program().is_file()
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
    /// Which file the store's `cert9.db` is, which the ledger keys a store by
    /// (see [`LedgerEntry`]). Updated by a commit, which replaces the file.
    id: Option<StoreId>,
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
            id: None,
        };
        // From here a failure drops `copy`, which removes the work directory.
        for name in NSS_DB_FILES {
            let original = read_at(&copy.dir, name)?;
            if let Some((bytes, _, id)) = &original {
                fs::write(copy.work.join(name), bytes)?;
                if name == "cert9.db" {
                    copy.id = Some(*id);
                }
            }
            copy.originals
                .push((name, original.map(|(bytes, mode, _)| (bytes, mode))));
        }
        Ok(copy)
    }

    /// The directory to hand certutil.
    fn path(&self) -> &Path {
        &self.work
    }

    fn commit(&mut self) -> std::result::Result<(), CertutilFailure> {
        self.try_commit()
            .map_err(|e| CertutilFailure::Failed(format!("could not write the store back: {e}")))
    }

    fn try_commit(&mut self) -> std::io::Result<()> {
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
            let now = read_at(&self.dir, name)?.map(|(bytes, _, _)| bytes);
            if now.as_deref() != before {
                return Err(std::io::Error::other(format!(
                    "{name} changed while Gate was writing it; try again"
                )));
            }
            let mode = original.as_ref().map_or(0o600, |(_, mode)| *mode);
            write_at(&self.dir, name, &bytes, mode)?;
        }
        self.id = read_at(&self.dir, "cert9.db")?.map(|(_, _, id)| id);
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

/// Which file a store's `cert9.db` is: its inode, and its creation time where
/// the filesystem records one. The inode alone is not enough - a store deleted
/// and made again commonly gets the number just freed - and the birth time is
/// what tells the new file from the old one, while an in-place edit (a removal
/// in the browser's certificate manager) changes neither.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
struct StoreId {
    ino: u64,
    #[serde(default)]
    born: Option<u64>,
}

/// The bytes, permission bits and identity of the regular file `name` in `dir`,
/// or `None` where there is no such file. A symlink there is an error.
fn read_at(dir: &OwnedFd, name: &str) -> std::io::Result<Option<(Vec<u8>, u32, StoreId)>> {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
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
    let id = StoreId {
        ino: meta.ino(),
        born: meta
            .created()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_nanos() as u64),
    };
    Ok(Some((bytes, meta.permissions().mode() & 0o777, id)))
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

/// Read a store's [`NssState`] through a [`StoreCopy`], with the inode the
/// ledger keys it by.
fn read_store(
    dir: &Path,
    pem: &str,
) -> std::result::Result<(NssState, Option<StoreId>), CertutilFailure> {
    let copy = StoreCopy::open(dir)?;
    Ok((nss_state(copy.path(), pem)?, copy.id))
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
    /// More than once. `certutil -A` appends under a duplicate nickname rather
    /// than replacing, so an add that ran without its delete leaves two, and
    /// the export then prints both and matches neither.
    Duplicated,
}

impl NssEntry {
    /// Whether the entry carries CA trust for SSL, which is the only one of the
    /// three fields that decides whether a browser accepts an intercepted host.
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
            Self::Absent | Self::Duplicated => false,
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
    let mut found = None;
    for line in listing.lines() {
        let Some((nickname, flags)) = line.rsplit_once(char::is_whitespace) else {
            continue;
        };
        // The header's second line is the legend `SSL,S/MIME,JAR/XPI` sitting
        // in the flags column with nothing before it, so an empty nickname is
        // not a row.
        let nickname = nickname.trim();
        if nickname == ca_common_name() {
            if found.is_some() {
                return Ok(NssEntry::Duplicated);
            }
            found = Some(NssEntry::Present {
                flags: flags.trim().to_string(),
            });
        }
    }
    Ok(found.unwrap_or(NssEntry::Absent))
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
    nss_state(db, pem).map(|state| state == NssState::Trusted)
}

/// What one store holds under our nickname, compared against the current CA.
#[derive(Debug, PartialEq)]
enum NssState {
    /// Exactly our current CA, trusted to identify websites.
    Trusted,
    /// Our current CA with the website trust taken off - which only a
    /// certificate manager does, Gate never adds it that way.
    Distrusted,
    /// Something under our nickname that is not exactly our current CA once: a
    /// root from before a regeneration, or a duplicated entry.
    Stale,
    /// Nothing under our nickname.
    Absent,
}

/// Read [`NssState`] for `db`. One listing, plus an export when the nickname is
/// there once.
fn nss_state(db: &Path, pem: &str) -> std::result::Result<NssState, CertutilFailure> {
    let entry = match nss_lookup(db)? {
        NssEntry::Absent => return Ok(NssState::Absent),
        NssEntry::Duplicated => return Ok(NssState::Stale),
        entry => entry,
    };
    let held = certutil_output(db, &["-L", "-n", ca_common_name(), "-a"])?;
    Ok(if pem_body(&held) != pem_body(pem) {
        NssState::Stale
    } else if entry.ssl_ca_trusted() {
        NssState::Trusted
    } else {
        NssState::Distrusted
    })
}

/// What [`ensure_trusted_nss`] does with a store it could read.
#[derive(Debug, PartialEq)]
enum NssAction {
    /// Nothing to the store; record it if it is not recorded yet.
    Keep,
    /// Nothing at all: the user changed this store in the browser.
    Leave,
    /// Add our CA, replacing whatever is under our nickname.
    Write,
}

/// Decide what to do with a store from what it holds and whether this CA is
/// recorded in it (see [`nss_ledger_path`]). Pure, so the policy is testable
/// without certutil.
///
/// A recorded store that no longer holds our CA, or holds it distrusted, was
/// changed in the browser's own certificate manager after Gate wrote it, and
/// putting it back on the next enable would overrule the user on a root that
/// can sign for the intercepted hosts. So it is left alone. An *unrecorded*
/// distrusted entry is rewritten: nothing says the user made it, and it is the
/// state that fails every intercepted host while looking identical to an
/// exporter. A stale entry is ours and outdated, so it is rewritten whatever
/// the record says.
fn nss_action(state: &NssState, recorded: bool) -> NssAction {
    match state {
        NssState::Trusted => NssAction::Keep,
        NssState::Distrusted | NssState::Absent if recorded => NssAction::Leave,
        NssState::Distrusted | NssState::Absent | NssState::Stale => NssAction::Write,
    }
}

/// Whether a store is as it should be, for the readings. A store left alone by
/// [`nss_action`] counts as fine: the surfaces that read this offer a retry,
/// and the retry deliberately does not override the user, so a store it would
/// never touch must not keep reporting a fault.
fn nss_store_ok(state: &NssState, recorded: bool) -> bool {
    nss_action(state, recorded) != NssAction::Write
}

/// What a live read of every browser NSS database found says about our CA, for
/// the diagnostics report. See [`NssProbe`] for why three answers rather than a
/// bool, and for the precedence between the two negatives.
///
/// `None` where the question does not apply: no browser keeps a store for this
/// user and no Chromium is waiting for one. Also `None` when the cert itself
/// cannot be read, which `ca_cert_present` already reports. A store the user
/// changed in the browser reads as fine; see [`nss_store_ok`].
///
/// Shells out once or twice per database, so it belongs on a user action and
/// not on the polled path - `status` serves [`recorded_nss_trust`] instead.
pub fn nss_ca_trusted() -> Option<NssProbe> {
    let dirs = nss_db_dirs();
    // A Chromium with no database yet is a store missing the CA, not a store
    // that does not apply: it is the state `ensure_trusted_nss` repairs, and
    // reading it as `None` hid it from every surface.
    let missing = nss_home().is_some_and(|home| chromium_db_missing(&home));
    if dirs.is_empty() && !missing {
        return None;
    }
    let pem = cert_path().ok().and_then(|p| fs::read_to_string(p).ok())?;
    if missing {
        return Some(NssProbe::Absent);
    }
    let ledger = read_nss_ledger();
    let mut unreadable = false;
    for dir in dirs {
        match read_store(&dir, &pem) {
            Ok((state, id)) if nss_store_ok(&state, is_recorded(&ledger, &dir, id)) => {}
            Ok(_) => return Some(NssProbe::Absent),
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
/// reading to serve: the CLI wrote the stores and its record is keyed to a CA
/// this process can read, or `proxy trust-ca --system-trust` installed the
/// system anchor before any browser store existed. Both leave the GUI watching
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
    let missing = nss_home().is_some_and(|home| chromium_db_missing(&home));
    if dirs.is_empty() && !missing {
        return None;
    }
    let pem = cert_path().ok().and_then(|p| fs::read_to_string(p).ok())?;
    // The missing database has no store to fail on, so without this a machine
    // with no certutil read as `NotWritten` - a retry, which can never work -
    // rather than the package install that does.
    if missing && !certutil_available() {
        return Some(NssTrust::ToolsMissing);
    }
    let ledger = read_nss_ledger();
    let mut absent = missing;
    for dir in dirs {
        match read_store(&dir, &pem) {
            Ok((state, id)) if nss_store_ok(&state, is_recorded(&ledger, &dir, id)) => {}
            Ok(_) => absent = true,
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
    /// Whether this is the record of a removal that failed. That one has to
    /// outlive the cert it is keyed to: `untrust` deletes the cert straight
    /// after, and a record nothing could match would leave the window telling
    /// the user the browsers no longer trust a root one of them still holds.
    #[serde(default)]
    removal: bool,
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
    write_nss_record(state, false);
}

/// Record a removal that failed, so it survives the cert's deletion; see
/// [`NssRecord::removal`].
fn record_nss_removal_failure(reading: NssReading) {
    write_nss_record(Some(reading), true);
}

fn write_nss_record(state: Option<NssReading>, removal: bool) {
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
        removal,
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
/// a record written for a different CA - the reason the fingerprint is in the
/// file at all - or no readable cert to check it against. The exception is a
/// failed removal, which is served with no cert at all, because the removal is
/// what deleted it; see [`NssRecord::removal`].
pub fn recorded_nss_trust() -> Option<NssReading> {
    let raw = fs::read_to_string(nss_record_path().ok()?).ok()?;
    let record: NssRecord = serde_json::from_str(&raw).ok()?;
    match cert_fingerprint() {
        Some(current) => (record.cert_fingerprint == current).then_some(record.reading),
        None => record.removal.then_some(record.reading),
    }
}

/// One store the current CA has been written to or found in.
///
/// Keyed by which file its `cert9.db` is ([`StoreId`]) as well as its path,
/// because a path alone cannot tell "the user removed the CA in this store"
/// from "the user deleted the store and something made a fresh one" - `rm -rf
/// ~/.pki/nssdb` is standard advice for a Chrome certificate error. Removing a
/// certificate in the browser edits the file in place, so its identity stays;
/// a new database is a new file and reads as unrecorded, so it is written. Our
/// own writes replace the file, so the entry is taken after them.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
struct LedgerEntry {
    path: PathBuf,
    #[serde(flatten)]
    id: StoreId,
}

/// The stores recorded for one CA. What tells "never had it" from "had it and
/// the user took it out" (see [`nss_action`]). Keyed to the certificate the way
/// [`NssRecord`] is, so a regenerated root starts a new record: a new root is a
/// new question, for a store the user pruned the old one from as much as any
/// other.
#[derive(serde::Serialize, serde::Deserialize)]
struct NssLedger {
    cert_fingerprint: String,
    stores: Vec<LedgerEntry>,
}

/// Where [`NssLedger`] lives, beside [`nss_record_path`] for the same reason.
fn nss_ledger_path() -> Result<PathBuf> {
    Ok(env::app_support_dir()?
        .join("proxy")
        .join("nss-stores.json"))
}

/// The recorded stores for the current CA. Empty for every way of not having a
/// record, including one written for a different CA.
fn read_nss_ledger() -> Vec<LedgerEntry> {
    let Some(fingerprint) = cert_fingerprint() else {
        return Vec::new();
    };
    nss_ledger_path()
        .ok()
        .and_then(|path| fs::read_to_string(path).ok())
        .and_then(|raw| serde_json::from_str::<NssLedger>(&raw).ok())
        .filter(|ledger| ledger.cert_fingerprint == fingerprint)
        .map(|ledger| ledger.stores)
        .unwrap_or_default()
}

/// Whether `dir`, as it is now, is the store the ledger recorded.
fn is_recorded(ledger: &[LedgerEntry], dir: &Path, id: Option<StoreId>) -> bool {
    id.is_some_and(|id| ledger.iter().any(|e| e.path == dir && e.id == id))
}

/// Note `dir` in the in-memory ledger, replacing whatever was recorded for the
/// path. A store with no `cert9.db` has nothing to key by and is not noted.
fn note_store(ledger: &mut Vec<LedgerEntry>, dir: &Path, id: Option<StoreId>) {
    let Some(id) = id else {
        return;
    };
    ledger.retain(|e| e.path != dir);
    ledger.push(LedgerEntry {
        path: dir.to_path_buf(),
        id,
    });
}

/// Write the ledger once, at the end of an enable, dropping stores that are no
/// longer there. Best-effort like [`record_nss_trust`]: a record that cannot be
/// written costs the next enable the knowledge that a later removal was the
/// user's, and must not fail this one.
fn write_nss_ledger(mut stores: Vec<LedgerEntry>) {
    let Some(cert_fingerprint) = cert_fingerprint() else {
        return;
    };
    stores.retain(|e| e.path.is_dir());
    let ledger = NssLedger {
        cert_fingerprint,
        stores,
    };
    let written = nss_ledger_path().and_then(|path| {
        let raw = serde_json::to_string_pretty(&ledger)?;
        crate::primitives::write_file(&path, raw.as_bytes(), 0o600)
    });
    if let Err(e) = written {
        eprintln!(
            "gate proxy: could not record the browser stores holding the CA ({e:#}); \
             a CA removed in a browser may be added back on the next enable"
        );
    }
}

/// Forget every recorded store. An untrust is the user asking for the CA gone
/// everywhere, so the next trust starts from no record.
fn clear_nss_ledger() {
    if let Ok(path) = nss_ledger_path() {
        let _ = fs::remove_file(path);
    }
}

/// When this process last added the CA to a browser store, in milliseconds
/// since the epoch, or 0 if it never has. Strictly increasing per write.
///
/// The window raises its "quit and reopen" note when `ca_trusted` goes true,
/// and that misses the case the Ubuntu report was: the system anchor trusted
/// long ago, and a browser store written on a later enable - a Chromium
/// database just created, a Firefox profile seen for the first time. A browser
/// only reads its store at launch, so each write is news; the window watches
/// this rise. A time rather than a count, so the window can remember the last
/// one it showed across its own reloads and across app restarts without a
/// stale count from an earlier process hiding a new write. Per process, so a
/// write made by the CLI does not move the window's - the CLI prints its own
/// sentence for that.
static NSS_WRITTEN_AT: AtomicU64 = AtomicU64::new(0);

/// [`NSS_WRITTEN_AT`], for `ProxyState::ca_nss_written_at`.
pub fn nss_written_at() -> u64 {
    NSS_WRITTEN_AT.load(Ordering::Relaxed)
}

fn note_nss_write() {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64);
    let _ = NSS_WRITTEN_AT.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |last| {
        Some(now.max(last + 1))
    });
}

/// How many browser stores this process has removed the CA from, so the CLI
/// says to reopen browsers only where a browser lost something.
static NSS_REMOVALS: AtomicU64 = AtomicU64::new(0);

pub fn nss_removals() -> u64 {
    NSS_REMOVALS.load(Ordering::Relaxed)
}

/// `dir` as the report shows it: relative to the home it hangs off, which the
/// report has no reason to repeat on every line.
fn display_store(home: Option<&Path>, dir: &Path) -> String {
    home.and_then(|home| dir.strip_prefix(home).ok())
        .map(|rel| format!("~/{}", rel.display()))
        .unwrap_or_else(|| dir.display().to_string())
}

/// One line per browser store and what it holds, for the diagnostics report.
/// The reading says whether anything is wrong and the refusals name a store
/// that refused; this also names the ones the user changed in the browser,
/// which no reading counts as a fault, and a Chromium database that is not
/// there yet. Shells out per store, so it belongs on the report, not the poll.
pub fn nss_store_report() -> Vec<String> {
    let home = nss_home();
    let mut lines = Vec::new();
    if let Some(home) = home.as_deref().filter(|home| chromium_db_missing(home)) {
        lines.push(format!(
            "{}: no database yet (a Chromium browser is installed)",
            display_store(Some(home), &chromium_db(home))
        ));
    }
    let Some(pem) = cert_path().ok().and_then(|p| fs::read_to_string(p).ok()) else {
        return lines;
    };
    let ledger = read_nss_ledger();
    for dir in nss_db_dirs() {
        let state = match read_store(&dir, &pem) {
            Ok((state, id)) => {
                let recorded = is_recorded(&ledger, &dir, id);
                match state {
                    NssState::Trusted => "trusted",
                    NssState::Distrusted if recorded => "distrusted in the browser",
                    NssState::Distrusted => "present without website trust",
                    NssState::Stale => "outdated",
                    NssState::Absent if recorded => "removed in the browser",
                    NssState::Absent => "missing",
                }
                .to_string()
            }
            Err(e) => format!("unreadable ({})", one_line_capped(&e.to_string())),
        };
        lines.push(format!("{}: {state}", display_store(home.as_deref(), &dir)));
    }
    lines
}

/// Delete every entry under our nickname in `db`, returning whether any went.
/// `certutil -D` removes one entry per call, and a store can hold more than one
/// (see [`NssEntry::Duplicated`]). Bounded, because a store that keeps
/// reporting success is broken in a way more calls will not fix.
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

/// Add the CA to every browser NSS database that should have it, so Chromium
/// and Firefox accept the leaves the engine mints. Creates `~/.pki/nssdb`
/// first when a Chromium browser is installed without one (see
/// [`chromium_db_missing`]), and leaves alone a store the user changed in the
/// browser (see [`nss_action`]).
///
/// Best-effort and infallible by design: the system anchor is what trust really
/// rests on, and a browser-specific store that cannot be written must not fail
/// enabling the proxy. Failures are reported rather than swallowed, because the
/// symptom otherwise lands in the browser as a certificate error with nothing
/// connecting it to Gate.
fn ensure_trusted_nss() {
    // Before the enumeration, so a database made here is one of the stores
    // written below. Its failure is held for the reading rather than dropped:
    // with no other store on the machine it is the only answer there is.
    let created = nss_home()
        .filter(|home| chromium_db_missing(home))
        .map(|home| (chromium_db(&home), create_chromium_db(&chromium_db(&home))));
    let dirs = nss_db_dirs();
    if dirs.is_empty() && !matches!(created, Some((_, Err(_)))) {
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
    if let Some((db, Err(e))) = &created {
        outcome = degrade(outcome, e);
        if matches!(e, CertutilFailure::Failed(_)) {
            refusals.push(NssRefusal {
                store: db.display().to_string(),
                reason: e.to_string(),
            });
        }
        eprintln!(
            "gate proxy: could not create the NSS store at {db} ({e}); \
             Chromium-based browsers will reject intercepted hosts{hint}",
            db = db.display(),
            hint = e.tools_hint(),
        );
    }
    let mut ledger = read_nss_ledger();
    let mut fail = |dir: &Path, e: CertutilFailure| {
        outcome = degrade(outcome, &e);
        // A refusal from certutil itself is most often a Firefox Primary
        // Password: changing trust needs it, and stdin is closed.
        let hint = match &e {
            CertutilFailure::Missing => e.tools_hint(),
            CertutilFailure::Failed(_) => " - if this is a Firefox profile with a Primary \
                 Password, import the certificate in Firefox's own certificate settings"
                .to_string(),
        };
        eprintln!(
            "gate proxy: could not add the CA to the NSS store at {dir} ({e}); \
             the browser reading it will reject intercepted hosts{hint}",
            dir = dir.display(),
        );
        if let CertutilFailure::Failed(reason) = e {
            refusals.push(NssRefusal {
                store: dir.display().to_string(),
                reason,
            });
        }
    };
    for dir in dirs {
        // Every call below runs on a private copy, committed back only once the
        // copy is known to hold the CA - see `StoreCopy` for why certutil never
        // touches a store in place. A store that cannot even be copied is
        // reported, not skipped.
        let mut copy = match StoreCopy::open(&dir) {
            Ok(copy) => copy,
            Err(e) => {
                fail(&dir, e);
                continue;
            }
        };
        // Nothing to do where the database already holds exactly our current
        // CA, trusted for SSL, or where the user changed it in the browser. A
        // store that cannot be read is rewritten, and the rewrite's own failure
        // is what gets reported - unless it is one we recorded, where a
        // rewrite could undo a removal the user made that we can no longer see.
        match nss_state(copy.path(), &cert_pem) {
            Ok(state) => match nss_action(&state, is_recorded(&ledger, &dir, copy.id)) {
                NssAction::Keep => {
                    note_store(&mut ledger, &dir, copy.id);
                    continue;
                }
                NssAction::Leave => continue,
                NssAction::Write => {}
            },
            Err(e) if ledger.iter().any(|entry| entry.path == dir) => {
                fail(&dir, e);
                continue;
            }
            Err(_) => {}
        }
        // Delete first, every copy. `certutil -A` appends under a duplicate
        // nickname rather than replacing, so a regenerated CA would leave the
        // stale root sitting in the database beside the new one, and the
        // browser would keep offering both. A missing entry fails here,
        // harmlessly. Nothing reaches the store unless the add lands.
        drop_nss_entries(copy.path());
        // `-t "C,,"`: trusted to issue SSL server certs, with no S/MIME and no
        // object-signing trust. The same flags mkcert uses for the same job.
        let args = ["-A", "-t", "C,,", "-n", ca_common_name(), "-i", &cert_arg];
        // Read the copy back rather than reporting the exit code. `Trusted` is
        // a claim about what the browser will accept, and an add that exits
        // zero is only evidence that certutil was happy.
        let written = certutil(copy.path(), &args)
            .and_then(|()| nss_holds(copy.path(), &cert_pem))
            .and_then(|held| {
                if held {
                    copy.commit()
                } else {
                    Err(CertutilFailure::Failed(
                        "certutil -A reported success and the store does not hold the CA"
                            .to_string(),
                    ))
                }
            });
        match written {
            Ok(()) => {
                note_nss_write();
                note_store(&mut ledger, &dir, copy.id);
            }
            Err(e) => fail(&dir, e),
        }
    }
    write_nss_ledger(ledger);
    // `refusals` is documented as empty for every outcome but `WriteFailed`,
    // and one store failing with a refusal before a later one loses certutil
    // entirely would otherwise leave the report naming a store under a headline
    // that says the binary is missing.
    if outcome == NssTrust::ToolsMissing {
        refusals.clear();
    }
    record_nss_trust(Some(NssReading { outcome, refusals }));
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
/// that fails for any *other* reason is a store we could not read, which is
/// reported rather than assumed empty; a missing `certutil` is separated out,
/// since it means we could neither look nor remove and anything the install put
/// there is still there.
/// The stores an untrust visits: every store found now, and every store
/// recorded as holding this CA that is still there but no longer found - a
/// profile dropped from `profiles.ini`, say. The ledger is cleared after, so a
/// store it alone knew about would otherwise keep the root and be forgotten.
/// Safe to hand straight to [`StoreCopy`]: the ledger is only ever opened
/// through it, like everything else.
fn untrust_dirs() -> Vec<PathBuf> {
    let mut dirs = nss_db_dirs();
    for entry in read_nss_ledger() {
        if entry.path.is_dir() && !dirs.contains(&entry.path) {
            dirs.push(entry.path);
        }
    }
    dirs
}

fn untrust_nss() {
    // Recorded at the end rather than cleared here. Clearing up front made a
    // removal that failed indistinguishable from one that worked: `None` is
    // "no reading", so a Gate root still sitting in a browser's store - the one
    // state this function's doc calls the security edge - was the single state
    // the reading could not express.
    let mut failure: Option<NssTrust> = None;
    let mut fail = |e: Option<&CertutilFailure>| {
        let so_far = failure.unwrap_or(NssTrust::WriteFailed);
        failure = Some(e.map_or(so_far, |e| degrade(so_far, e)));
    };
    for dir in untrust_dirs() {
        let mut copy = match StoreCopy::open(&dir) {
            Ok(copy) => copy,
            Err(e) => {
                eprintln!(
                    "gate proxy: could not open the NSS store at {dir} to remove the CA ({e}); \
                     the browser reading it may still trust it",
                    dir = dir.display(),
                );
                fail(Some(&e));
                continue;
            }
        };
        match nss_lookup(copy.path()) {
            Ok(NssEntry::Absent) => {}
            Ok(NssEntry::Present { .. } | NssEntry::Duplicated) => {
                // Every copy, then a read-back: a duplicated entry needs one
                // `-D` per copy, and "the delete ran" is not "it is gone".
                drop_nss_entries(copy.path());
                let removed = match nss_lookup(copy.path()) {
                    Ok(NssEntry::Absent) => copy.commit(),
                    Ok(_) => Err(CertutilFailure::Failed(
                        "the entry was still there after deleting it".to_string(),
                    )),
                    Err(e) => Err(e),
                };
                match removed {
                    Ok(()) => {
                        NSS_REMOVALS.fetch_add(1, Ordering::Relaxed);
                    }
                    Err(e) => {
                        eprintln!(
                            "gate proxy: could not remove the CA from the NSS store at {dir} \
                             ({e}); the browser reading it still trusts it",
                            dir = dir.display(),
                        );
                        fail(Some(&e));
                    }
                }
            }
            Err(e @ CertutilFailure::Missing) => {
                eprintln!(
                    "gate proxy: could not remove the CA from the NSS store at {dir} ({e}); \
                     the browser reading it may still trust it - {NSS_TOOLS_HINT}",
                    dir = dir.display(),
                );
                fail(Some(&e));
            }
            Err(e @ CertutilFailure::Failed(_)) => {
                eprintln!(
                    "gate proxy: could not read the NSS store at {dir} to remove the CA ({e}); \
                     the browser reading it may still trust it",
                    dir = dir.display(),
                );
                fail(None);
            }
        }
    }
    // A clean removal is a question that stopped applying, not a verdict: the
    // CA is going on purpose and `ca_trusted` goes false beside it. A failed
    // one is recorded so that it outlives `remove_ca_material`, which deletes
    // the cert straight after - see `NssRecord::removal`.
    match failure {
        Some(outcome) => record_nss_removal_failure(NssReading {
            outcome,
            refusals: Vec::new(),
        }),
        None => record_nss_trust(None),
    }
    clear_nss_ledger();
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
            // An empty stand-in: the shim never reads it, but a store needs a
            // `cert9.db` for the ledger to key it by, as a real one has.
            fs::write(db.join("cert9.db"), b"").expect("create cert9.db");
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
            *CHROMIUM_INSTALLED_OVERRIDE
                .lock()
                .expect("chromium override mutex poisoned") = None;
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
            let nick = ca_common_name();
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
      echo '{nick}                             {flags}'
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
  -N)
    touch "${{2#sql:}}/cert9.db"
    exit 0
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
                nick = ca_common_name(),
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
            Ok(NssEntry::Absent | NssEntry::Duplicated) => {
                panic!("the entry is right there in the listing, once")
            }
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

    /// Whether `f` wrote the CA to a browser store, read off the write time:
    /// it is process-wide and strictly increasing, and every fixture test
    /// holds the path lock, so nothing else moves it while one runs.
    fn wrote_during(f: impl FnOnce()) -> bool {
        let before = nss_written_at();
        f();
        nss_written_at() > before
    }

    /// The recorded store paths, without the inodes that key them.
    fn ledger_paths() -> Vec<PathBuf> {
        read_nss_ledger().into_iter().map(|e| e.path).collect()
    }

    /// A store the user took the CA out of after Gate wrote it is theirs: the
    /// next enable leaves it out, and the reading does not call it a fault -
    /// the retry the window offers would not override them either.
    #[test]
    fn a_store_the_user_emptied_stays_empty() {
        let fixture = NssFixture::new("pruned", false, store_shim("C,,", Add::Lands));
        assert!(wrote_during(ensure_trusted_nss));
        assert!(fixture.store_holds());
        // The certificate manager's Delete.
        fs::remove_file(&fixture.marker).expect("remove the entry");
        assert!(!wrote_during(ensure_trusted_nss));
        assert!(
            !fixture.store_holds(),
            "a removal in the browser is put back"
        );
        let reading = recorded_nss_trust().expect("a write records a reading");
        assert_eq!(reading.outcome, NssTrust::Trusted);
        assert_eq!(probe_nss_trust(), Some(NssTrust::Trusted));
    }

    /// ...and an untrust is the user asking for it gone everywhere, so the
    /// record goes with it and the next trust writes every store again.
    #[test]
    fn an_untrust_forgets_which_stores_held_it() {
        let fixture = NssFixture::new("forget", false, store_shim("C,,", Add::Lands));
        ensure_trusted_nss();
        untrust_nss();
        assert!(!fixture.store_holds());
        assert!(wrote_during(ensure_trusted_nss));
        assert!(fixture.store_holds());
    }

    /// A store already holding the CA is recorded without a write, so a later
    /// removal in the browser is recognised as one even though Gate never
    /// wrote that store in this record's life.
    #[test]
    fn a_store_found_holding_the_ca_is_recorded_without_a_write() {
        let fixture = NssFixture::new("adopt", true, store_shim("C,,", Add::Lands));
        assert!(!wrote_during(ensure_trusted_nss));
        assert_eq!(ledger_paths(), vec![fixture.db.clone()]);
    }

    /// The measured bug: Chrome installed, `~/.pki/nssdb` never created, and
    /// every candidate filtered out, so nothing was written and nothing was
    /// said. Now the database is made, written, and counted.
    #[test]
    fn a_chromium_with_no_database_gets_one() {
        let fixture = NssFixture::new("create", false, store_shim("C,,", Add::Lands));
        fs::remove_dir_all(&fixture.db).expect("start with no database");
        assert_eq!(probe_nss_trust(), None, "no Chromium, nothing applies");
        *CHROMIUM_INSTALLED_OVERRIDE
            .lock()
            .expect("chromium override mutex poisoned") = Some(true);
        assert_eq!(nss_ca_trusted(), Some(NssProbe::Absent));
        assert_eq!(probe_nss_trust(), Some(NssTrust::NotWritten));
        assert!(wrote_during(ensure_trusted_nss));
        assert!(fixture.db.join("cert9.db").is_file());
        assert!(fixture.store_holds());
        assert_eq!(
            recorded_nss_trust().map(|r| r.outcome),
            Some(NssTrust::Trusted)
        );
    }

    /// The database we create stands in for the one Chrome would have made,
    /// which is owner-only. The directory is made before certutil runs, so a
    /// missing certutil still leaves it - and the reading says so rather than
    /// that nothing applies.
    #[test]
    fn the_created_database_is_owner_only_and_a_failed_init_is_reported() {
        use std::os::unix::fs::PermissionsExt;
        let fixture = NssFixture::new("mode", false, |_, _| String::new());
        fs::remove_dir_all(&fixture.db).expect("start with no database");
        *CHROMIUM_INSTALLED_OVERRIDE
            .lock()
            .expect("chromium override mutex poisoned") = Some(true);
        *CERTUTIL_OVERRIDE
            .lock()
            .expect("certutil override mutex poisoned") =
            Some(PathBuf::from("/nonexistent/certutil"));
        ensure_trusted_nss();
        let mode = fs::metadata(&fixture.db).map(|m| m.permissions().mode() & 0o777);
        assert_eq!(mode.expect("the directory was made"), 0o700);
        assert_eq!(
            recorded_nss_trust().map(|r| r.outcome),
            Some(NssTrust::ToolsMissing)
        );
        // Keyed on `cert9.db`, so the half-made directory is still missing and
        // the creation runs again once certutil is there.
        assert!(chromium_db_missing(&fixture.root.join("home")));
    }

    /// Two entries under our nickname - an add that ran without its delete -
    /// read as one stale store, and every copy goes before the add.
    #[test]
    fn a_duplicated_entry_is_stale_and_fully_replaced() {
        let fixture = NssFixture::new("dup", false, |cert, marker| {
            let (cert, marker) = (cert.display(), marker.display());
            format!(
                r#"
case "$3" in
  -L)
    if [ "$4" = "-n" ]; then cat '{cert}'; exit 0; fi
    echo 'Certificate Nickname                             Trust Attributes'
    echo ''
    n=$(cat '{marker}' 2>/dev/null || echo 2)
    i=0; while [ $i -lt $n ]; do echo '{nick}                             C,,'; i=$((i+1)); done
    exit 0
    ;;
  -D)
    n=$(cat '{marker}' 2>/dev/null || echo 2)
    [ $n -gt 0 ] || exit 255
    echo $((n-1)) > '{marker}'; exit 0
    ;;
  -A)
    n=$(cat '{marker}' 2>/dev/null || echo 2)
    echo $((n+1)) > '{marker}'; exit 0
    ;;
esac
exit 0
"#,
                nick = ca_common_name(),
            )
        });
        assert!(matches!(nss_lookup(&fixture.db), Ok(NssEntry::Duplicated)));
        ensure_trusted_nss();
        assert_eq!(
            fs::read_to_string(&fixture.marker).expect("count").trim(),
            "1",
            "both copies dropped, one added"
        );
        assert_eq!(
            recorded_nss_trust().map(|r| r.outcome),
            Some(NssTrust::Trusted)
        );
    }

    /// The policy table: a recorded store the user emptied or distrusted is
    /// left alone and not held against them; an unrecorded one is written, as
    /// is a stale entry whatever the record says.
    #[test]
    fn the_store_policy_respects_only_what_the_user_did() {
        use NssAction::*;
        let cases = [
            (NssState::Trusted, false, Keep, true),
            (NssState::Trusted, true, Keep, true),
            (NssState::Distrusted, false, Write, false),
            (NssState::Distrusted, true, Leave, true),
            (NssState::Absent, false, Write, false),
            (NssState::Absent, true, Leave, true),
            (NssState::Stale, false, Write, false),
            (NssState::Stale, true, Write, false),
        ];
        for (state, recorded, action, ok) in cases {
            assert_eq!(
                nss_action(&state, recorded),
                action,
                "{state:?} recorded={recorded}"
            );
            assert_eq!(
                nss_store_ok(&state, recorded),
                ok,
                "{state:?} recorded={recorded}"
            );
        }
    }

    /// A fresh directory under the temp root, unique per test.
    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("gate-nss-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("create scratch dir");
        dir
    }

    /// Ubuntu's default Firefox is the snap, which keeps profiles under its own
    /// confined HOME and ignores the system anchor.
    #[test]
    fn the_firefox_roots_cover_the_snap_and_flatpak_homes() {
        let roots = firefox_profile_roots(Path::new("/home/u"));
        for expected in [
            "/home/u/.mozilla/firefox",
            "/home/u/snap/firefox/common/.mozilla/firefox",
            "/home/u/.var/app/org.mozilla.firefox/.mozilla/firefox",
        ] {
            assert!(
                roots.iter().any(|r| r == Path::new(expected)),
                "{expected} missing from {roots:?}"
            );
        }
        let home = Path::new("/tmp/someone");
        for root in firefox_profile_roots(home) {
            assert!(root.starts_with(home), "{root:?} escaped {home:?}");
        }
    }

    /// Only real profile directories with a real `cert9.db` count: not the
    /// siblings Firefox keeps beside its profiles, and not a symlink a confined
    /// Firefox could plant to steer the unconfined certutil.
    #[test]
    fn the_firefox_dbs_are_real_profiles_with_a_cert_store() {
        let home = scratch("ff");
        let root = home.join("snap/firefox/common/.mozilla/firefox");
        let profile = root.join("qp1tx1a3.default");
        fs::create_dir_all(&profile).expect("profile");
        fs::write(profile.join("cert9.db"), b"").expect("cert9");
        fs::create_dir_all(root.join("Crash Reports")).expect("sibling");
        let elsewhere = home.join("elsewhere");
        fs::create_dir_all(&elsewhere).expect("elsewhere");
        fs::write(elsewhere.join("cert9.db"), b"").expect("cert9");
        std::os::unix::fs::symlink(&elsewhere, root.join("planted.default")).expect("link");

        let dbs = firefox_profile_dbs(&home);
        let _ = fs::remove_dir_all(&home);
        assert_eq!(dbs, vec![profile]);
    }

    /// `profiles.ini` adds only what the directory listing cannot see: a
    /// profile the user moved, named by absolute path - and only from the
    /// unconfined root, since a sandboxed Firefox writes its own file.
    /// Relative entries are the listing's job, and the ones that could walk
    /// out of the root are never followed.
    #[test]
    fn the_profiles_ini_adds_only_moved_profiles_from_the_unconfined_root() {
        let ini = "[Install4F96D1932A9F858E]\nDefault=a.default\n\n\
                   [Profile0]\nName=a\nIsRelative=1\nPath=a.default\n\n\
                   [Profile1]\nIsRelative=0\nPath=/data/ff/b\n\n\
                   [Profile2]\nIsRelative=1\nPath=../escape\n\n\
                   [Profile3]\nIsRelative=0\nPath=relative/but/marked\n";
        assert_eq!(
            firefox_ini_profiles(ini, true),
            vec![PathBuf::from("/data/ff/b")]
        );
        assert!(firefox_ini_profiles(ini, false).is_empty());
    }

    /// A launcher on PATH means Chromium reads `~/.pki/nssdb`, except a snap
    /// shim of the same name, which resolves to `snap` and reads a database in
    /// its own confined HOME.
    #[test]
    fn a_chromium_launcher_counts_unless_it_is_a_snap_shim() {
        let dir = scratch("path");
        let real = dir.join("real");
        let snapbin = dir.join("snapbin");
        fs::create_dir_all(&real).expect("real");
        fs::create_dir_all(&snapbin).expect("snapbin");
        fs::write(real.join("google-chrome"), b"").expect("launcher");
        fs::write(dir.join("snap"), b"").expect("snap");
        std::os::unix::fs::symlink(dir.join("snap"), snapbin.join("chromium")).expect("link");

        let found = chromium_on_path(real.as_os_str());
        let shim = chromium_on_path(snapbin.as_os_str());
        let empty = chromium_on_path(dir.as_os_str());
        let _ = fs::remove_dir_all(&dir);
        assert!(found);
        assert!(!shim);
        assert!(!empty);
    }

    /// The sandbox-escape the copy exists for: a confined browser can write
    /// its store's `pkcs11.txt`, and NSS loads every `library=` in it. So
    /// certutil must never be pointed at the store itself, and the directory
    /// it is pointed at must have no `pkcs11.txt` of the store's.
    #[test]
    fn certutil_never_runs_in_the_store_or_sees_its_module_database() {
        let fixture = NssFixture::new("modules", false, |cert, marker| {
            let log = marker.with_extension("log");
            let inner = store_shim("C,,", Add::Lands)(cert, marker);
            format!(
                "d=\"${{2#sql:}}\"\n\
                 echo \"$d\" >> '{log}'\n\
                 [ -e \"$d/pkcs11.txt\" ] && echo PKCS11 >> '{log}'\n\
                 {inner}",
                log = log.display(),
            )
        });
        fs::write(
            fixture.db.join("pkcs11.txt"),
            "library=/nonexistent/evil.so\nname=evil\n",
        )
        .expect("plant pkcs11.txt");
        ensure_trusted_nss();
        untrust_nss();
        let log = fs::read_to_string(fixture.marker.with_extension("log")).expect("shim ran");
        assert!(
            !log.contains("PKCS11"),
            "certutil saw the store's module database:\n{log}"
        );
        let store = fixture.db.display().to_string();
        assert!(
            log.lines().all(|dir| dir != store),
            "certutil ran in the store itself:\n{log}"
        );
    }

    /// A symlinked `key4.db` is refused rather than followed: NSS would
    /// otherwise create or open whatever file it points at.
    #[test]
    fn a_symlinked_key_database_is_refused_not_followed() {
        let fixture = NssFixture::new("keylink", false, store_shim("C,,", Add::Lands));
        let target = fixture.root.join("elsewhere").join("created-by-nss");
        std::os::unix::fs::symlink(&target, fixture.db.join("key4.db")).expect("plant link");
        ensure_trusted_nss();
        assert!(!target.exists(), "the link was followed");
        assert!(!fixture.store_holds());
        let reading = recorded_nss_trust().expect("a write records a reading");
        assert_eq!(reading.outcome, NssTrust::WriteFailed);
        assert_eq!(reading.refusals.len(), 1, "{:?}", reading.refusals);
    }

    /// The commit writes back only what changed, replaces the file (so the
    /// inode moves), and refuses to overwrite a store that changed under it.
    #[test]
    fn a_commit_writes_changes_and_refuses_a_store_that_moved() {
        let _fixture = NssFixture::new("commit", false, store_shim("C,,", Add::Lands));
        let store = scratch("commit-store");
        fs::write(store.join("cert9.db"), b"old").expect("cert9");
        fs::write(store.join("key4.db"), b"key").expect("key4");

        let mut copy = StoreCopy::open(&store).ok().expect("open");
        let before = copy.id;
        fs::write(copy.path().join("cert9.db"), b"new").expect("change the copy");
        copy.commit().ok().expect("commit");
        assert_eq!(fs::read(store.join("cert9.db")).expect("read"), b"new");
        assert_eq!(fs::read(store.join("key4.db")).expect("read"), b"key");
        assert_ne!(copy.id, before, "a commit replaces the file");
        drop(copy);

        let mut copy = StoreCopy::open(&store).ok().expect("open");
        fs::write(copy.path().join("cert9.db"), b"ours").expect("change the copy");
        fs::write(store.join("cert9.db"), b"the browser's").expect("change the store");
        assert!(copy.commit().is_err(), "an older copy overwrote the store");
        assert_eq!(
            fs::read(store.join("cert9.db")).expect("read"),
            b"the browser's"
        );
        drop(copy);
        let _ = fs::remove_dir_all(&store);
    }

    /// A store deleted and made again is a new store, not one the user
    /// emptied: the record keys on the database file, so the fresh one is
    /// written. `rm -rf ~/.pki/nssdb` is standard advice for Chrome cert errors.
    #[test]
    fn a_recreated_store_is_written_again() {
        let fixture = NssFixture::new("recreate", false, store_shim("C,,", Add::Lands));
        assert!(wrote_during(ensure_trusted_nss));
        fs::remove_dir_all(&fixture.db).expect("delete the store");
        fs::remove_file(&fixture.marker).expect("its entry went with it");
        fs::create_dir_all(&fixture.db).expect("recreate");
        fs::write(fixture.db.join("cert9.db"), b"fresh").expect("fresh cert9.db");
        assert!(wrote_during(ensure_trusted_nss));
        assert!(fixture.store_holds());
    }

    /// A Firefox profile goes through the same write and record as Chrome's
    /// database.
    #[test]
    fn a_firefox_profile_is_written_and_recorded() {
        let fixture = NssFixture::new("firefox", false, store_shim("C,,", Add::Lands));
        let profile = fixture
            .root
            .join("home/.mozilla/firefox/abcd1234.default-release");
        fs::create_dir_all(&profile).expect("profile");
        fs::write(profile.join("cert9.db"), b"").expect("cert9");
        ensure_trusted_nss();
        let mut paths = ledger_paths();
        paths.sort();
        let mut expected = vec![fixture.db.clone(), profile];
        expected.sort();
        assert_eq!(paths, expected);
    }

    /// The report names each store relative to home and says what it holds,
    /// including the one state no reading counts as a fault.
    #[test]
    fn the_store_report_names_each_store_and_its_state() {
        let fixture = NssFixture::new("report", false, store_shim("C,,", Add::Lands));
        ensure_trusted_nss();
        assert_eq!(
            nss_store_report(),
            vec!["~/.pki/nssdb: trusted".to_string()]
        );
        fs::remove_file(&fixture.marker).expect("removed in the browser");
        assert_eq!(
            nss_store_report(),
            vec!["~/.pki/nssdb: removed in the browser".to_string()]
        );
    }

    /// The record of stores belongs to one certificate: a regenerated root
    /// starts from none.
    #[test]
    fn the_store_record_is_keyed_to_the_certificate() {
        let _fixture = NssFixture::new("ledger-key", false, store_shim("C,,", Add::Lands));
        ensure_trusted_nss();
        assert_eq!(ledger_paths().len(), 1);
        fs::write(
            cert_path().expect("cert path"),
            "-----BEGIN CERTIFICATE-----\nMIIBDIFFERENT\n-----END CERTIFICATE-----\n",
        )
        .expect("regenerate the cert");
        assert!(read_nss_ledger().is_empty());
    }

    /// A removal that fails has to stay readable after the untrust deletes the
    /// cert it was keyed to, or the window says the browsers let go of a root
    /// one of them still holds.
    #[test]
    fn a_failed_removal_outlives_the_certificate() {
        let _fixture = NssFixture::new("removal-kept", true, |cert, _marker| {
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
  -D) echo 'SEC_ERROR_READ_ONLY' >&2; exit 255 ;;
esac
exit 0
"#,
                cert = cert.display(),
                nick = ca_common_name(),
            )
        });
        untrust_nss();
        fs::remove_file(cert_path().expect("cert path")).expect("untrust deletes the cert");
        assert_eq!(
            recorded_nss_trust().map(|r| r.outcome),
            Some(NssTrust::WriteFailed)
        );
    }

    /// A Chromium with no database and no certutil is a package install, not a
    /// retry: there is no store to fail on, so the probe has to ask.
    #[test]
    fn a_missing_database_with_no_certutil_is_tools_missing() {
        let fixture = NssFixture::new("probe-notools", false, |_, _| String::new());
        fs::remove_dir_all(&fixture.db).expect("no database");
        *CHROMIUM_INSTALLED_OVERRIDE
            .lock()
            .expect("chromium override mutex poisoned") = Some(true);
        *CERTUTIL_OVERRIDE
            .lock()
            .expect("certutil override mutex poisoned") =
            Some(PathBuf::from("/nonexistent/certutil"));
        assert_eq!(probe_nss_trust(), Some(NssTrust::ToolsMissing));
    }

    /// A creation that stops after `mkdir` is reported once, as the missing
    /// database it is, not again as a store that refused.
    #[test]
    fn a_half_made_database_is_reported_once() {
        let fixture = NssFixture::new("half", false, |_, _| {
            "echo 'SEC_ERROR_IO' >&2; exit 255".to_string()
        });
        fs::remove_dir_all(&fixture.db).expect("no database");
        *CHROMIUM_INSTALLED_OVERRIDE
            .lock()
            .expect("chromium override mutex poisoned") = Some(true);
        ensure_trusted_nss();
        let reading = recorded_nss_trust().expect("a failed creation is a reading");
        assert_eq!(reading.outcome, NssTrust::WriteFailed);
        assert_eq!(reading.refusals.len(), 1, "{:?}", reading.refusals);
        assert_eq!(nss_store_report().len(), 1, "{:?}", nss_store_report());
    }

    /// Untrust reaches a store the record knows about even once it drops out of
    /// enumeration, so the root is not left behind in a store nobody lists.
    #[test]
    fn untrust_visits_recorded_stores_no_longer_listed() {
        let fixture = NssFixture::new("unlisted", false, store_shim("C,,", Add::Lands));
        ensure_trusted_nss();
        let moved = fixture.root.join("moved-profile");
        fs::create_dir_all(&moved).expect("moved profile");
        fs::write(moved.join("cert9.db"), b"").expect("cert9");
        let mut ledger = read_nss_ledger();
        let id = StoreCopy::open(&moved).ok().expect("open").id;
        note_store(&mut ledger, &moved, id);
        write_nss_ledger(ledger);
        assert!(untrust_dirs().contains(&moved));
        assert!(untrust_dirs().contains(&fixture.db));
    }
}
