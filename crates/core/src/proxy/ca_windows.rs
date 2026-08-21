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
//!
//! [`ensure_trusted_system`] is the exception, and the one path with no dialog:
//! an opt-in, CLI-only install into the machine (`LocalMachine`) root store from
//! an already-elevated process, for machines with nobody to answer a
//! confirmation (build agents, headless boxes). It is never the default and
//! never reachable from the GUI. [`is_trusted`] consults both stores so a
//! machine-wide install actually satisfies the product, rather than leaving
//! `enable` re-prompting over a root the OS already trusts.

use std::fs;
use std::os::windows::process::CommandExt;
use std::path::PathBuf;
use std::process::{Command, ExitStatus, Stdio};
use std::sync::Mutex;
use std::thread;
use std::time::{Duration, Instant};

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

/// How long a **non-interactive** `certutil` call may run before we stop waiting
/// on it and kill it.
///
/// None of the bounded calls below wait on a human - they are root-store lookups
/// and deletes that answer in milliseconds on a healthy machine - so a wait this
/// long already means the child is never coming back. Leaving it unbounded is
/// what turns a `certutil` that dies badly into an unusable machine: a process
/// that crashes here stays alive while Windows Error Reporting collects its
/// dump, and the status path re-spawns it on a timer. `disable_quiet` documents
/// the same hazard, worked around for the exit path only.
const CERTUTIL_TIMEOUT: Duration = Duration::from_secs(10);

/// Gap between `try_wait` polls while waiting out [`CERTUTIL_TIMEOUT`].
const CERTUTIL_POLL: Duration = Duration::from_millis(50);

/// Run a **non-interactive** `certutil` invocation to completion, or kill it and
/// answer `None` once it outlives [`CERTUTIL_TIMEOUT`].
///
/// Deliberately not used for `-addstore Root` in [`ensure_trusted`]: that call
/// raises Windows' native trust dialog and legitimately blocks on a human, so a
/// timeout would kill a prompt the user is still reading.
///
/// Stdio is `null` rather than piped. No bounded caller reads the output (they
/// all key on the exit status), and piping a child we may stop waiting on would
/// leave us holding the pipe ends of a process we just killed. `kill` is
/// `TerminateProcess` and returns without blocking; the handle is closed on
/// drop, so there is deliberately no `wait()` behind it - reaping is a Unix
/// concern, and waiting on a kill that failed would reintroduce the hang this
/// exists to remove.
fn certutil_bounded(mut cmd: Command) -> Result<Option<ExitStatus>> {
    let mut child = cmd
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .context("spawning certutil")?;
    let deadline = Instant::now() + CERTUTIL_TIMEOUT;
    loop {
        if let Some(status) = child.try_wait().context("waiting on certutil")? {
            return Ok(Some(status));
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            return Ok(None);
        }
        thread::sleep(CERTUTIL_POLL);
    }
}

/// The error for a bounded call we had to kill, named after the verb that hung.
fn certutil_timed_out(verb: &str) -> anyhow::Error {
    anyhow::anyhow!(
        "certutil {verb} did not finish within {}s and was killed",
        CERTUTIL_TIMEOUT.as_secs()
    )
}

/// How long [`is_trusted`] stops shelling out after `certutil` fails to answer.
///
/// The bounded wait above keeps one bad call from wedging the app, but it does
/// not stop the *next* one: the window polls `proxy_status` every few seconds,
/// so a host where `certutil` dies on every invocation was still handed a dozen
/// or more doomed spawns a minute, each one potentially its own Windows Error
/// Reporting dump. Five minutes is long enough that the churn stops mattering
/// and short enough that a machine which recovers is noticed without a restart.
/// Any explicit trust change resets it - see [`forget_trust_reading`] - so a user
/// who acts on the error is never waiting out the cooldown.
const CERTUTIL_COOLDOWN: Duration = Duration::from_secs(300);

/// Last answer [`is_trusted`] got out of `certutil`, and when it last failed to
/// get one.
struct TrustProbe {
    /// The last successful reading, with the cert thumbprint it was taken for.
    /// Keyed on the thumbprint for the same reason the lookup is: a reading for
    /// the *previous* CA says nothing about the one now on disk.
    last: Option<(String, bool)>,
    /// When `certutil` most recently failed to answer (killed, or unspawnable).
    /// `None` once it answers again.
    failed_at: Option<Instant>,
}

static TRUST_PROBE: Mutex<TrustProbe> = Mutex::new(TrustProbe {
    last: None,
    failed_at: None,
});

/// The reading to serve without spawning anything. `Ok(None)` means go ahead and
/// probe; `Ok(Some(answer))` is the cached reading while the cooldown holds; an
/// error means the cooldown holds and there is nothing cached to serve.
fn trust_reading_during_cooldown(thumb: &str) -> Result<Option<bool>> {
    let probe = TRUST_PROBE.lock().expect("trust probe mutex poisoned");
    let Some(failed_at) = probe.failed_at else {
        return Ok(None);
    };
    let waited = failed_at.elapsed();
    if waited >= CERTUTIL_COOLDOWN {
        return Ok(None);
    }
    match probe.last.as_ref() {
        Some((cached, answer)) if cached == thumb => Ok(Some(*answer)),
        // Nothing to serve: either no call ever succeeded, or the one that did
        // was about a different CA. Reporting that beats inventing a "no", which
        // would advertise an untrusted CA on the one screen the user checks it on
        // and send `ensure_trusted` at a re-install with its dialog.
        _ => Err(anyhow::anyhow!(
            "certutil failed to answer {}s ago and is not being re-run for another {}s",
            waited.as_secs(),
            CERTUTIL_COOLDOWN.saturating_sub(waited).as_secs()
        )),
    }
}

/// Record what a probe produced: a reading closes the breaker, a failure to
/// answer opens it for [`CERTUTIL_COOLDOWN`].
fn record_trust_probe(thumb: &str, answer: &Result<bool>) {
    let mut probe = TRUST_PROBE.lock().expect("trust probe mutex poisoned");
    match answer {
        Ok(hit) => {
            probe.last = Some((thumb.to_string(), *hit));
            probe.failed_at = None;
        }
        Err(_) => probe.failed_at = Some(Instant::now()),
    }
}

/// Drop the cached reading and close the breaker, after this process changes the
/// trust itself.
///
/// Two jobs. The cached answer is now wrong (we just installed or removed the
/// very root it describes, without the thumbprint changing), and a user who
/// reached one of those paths has asked for something specific, which is reason
/// enough to let the next status read pay for a real probe rather than serving
/// them a cooldown.
fn forget_trust_reading() {
    let mut probe = TRUST_PROBE.lock().expect("trust probe mutex poisoned");
    probe.last = None;
    probe.failed_at = None;
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
    let mut stale = certutil();
    stale.args(["-user", "-delstore", "Root", CA_COMMON_NAME]);
    let _ = certutil_bounded(stale);

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
/// root store (`certutil -store Root <thumbprint>`), which exits 0 only when a
/// cert with that exact thumbprint is present.
///
/// Both stores count: the per-user one (`HKCU`, where [`ensure_trusted`] puts
/// it) and the machine one (`LocalMachine`, where [`ensure_trusted_system`]
/// does). Checking only the per-user store - which is what this did - would
/// report "not trusted" after a successful `--system-trust` install and make
/// every `enable` re-prompt for a root the OS already trusts. Unlike macOS,
/// where `verify-cert` is a policy evaluation that honours the admin domain for
/// free, a `certutil` store lookup sees exactly the store it is pointed at.
///
/// The previous check matched the CN, which a stale root from a prior install
/// satisfies - it shares our CN but has a different key/fingerprint. That made
/// `ensure_trusted` no-op while the engine signed leaves with a *different*,
/// untrusted CA, so every MITM handshake failed with no recovery. Matching the
/// thumbprint catches the mismatch and lets `ensure_trusted` re-install.
/// Read-only / non-privileged in both stores.
pub fn is_trusted() -> Result<bool> {
    let thumb = match cert_thumbprint()? {
        Some(t) => t,
        None => return Ok(false),
    };
    // Ask the breaker before certutil: on a host where certutil cannot answer,
    // the status poll's cadence would otherwise keep spawning children that only
    // ever get killed.
    if let Some(cached) = trust_reading_during_cooldown(&thumb)? {
        return Ok(cached);
    }
    let answer = root_store_has(Scope::User, &thumb).and_then(|user_store| {
        if user_store {
            Ok(true)
        } else {
            root_store_has(Scope::Machine, &thumb)
        }
    });
    record_trust_probe(&thumb, &answer);
    answer
}

/// Which root store a `certutil` call addresses. `-user` is the per-user store
/// (`HKCU`, no admin); its absence means the machine store (`LocalMachine`,
/// admin to write, world-readable).
#[derive(Clone, Copy)]
enum Scope {
    User,
    Machine,
}

impl Scope {
    /// The `certutil` prefix args for this scope, ahead of the verb.
    fn args(self) -> &'static [&'static str] {
        match self {
            Scope::User => &["-user"],
            Scope::Machine => &[],
        }
    }
}

/// Whether `id` (a thumbprint or a CN) matches a cert in this scope's root
/// store. Read-only, so the machine store needs no elevation.
fn root_store_has(scope: Scope, id: &str) -> Result<bool> {
    let mut cmd = certutil();
    cmd.args(scope.args()).args(["-store", "Root", id]);
    let finished = certutil_bounded(cmd).context("running certutil -store Root")?;
    // A lookup we had to kill is unanswerable, not a "no". Answering false would
    // report an installed root as untrusted and send `ensure_trusted` at a
    // re-install - and its dialog - over a cert the OS already holds. Callers
    // that can live without an answer fold this to false themselves; see
    // `store_has_our_ca`.
    let status = finished.ok_or_else(|| certutil_timed_out("-store Root"))?;
    Ok(status.success())
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
    let mut stale = certutil();
    stale.args(["-user", "-delstore", "Root", CA_COMMON_NAME]);
    let _ = certutil_bounded(stale);
    let cert = cert_path()?;
    // `output()` rather than `status()`: the dialog is a window certutil raises,
    // so capturing the pipes does not suppress it, and without them certutil's
    // own complaint went to an inherited stderr nobody reads. It is the only
    // description of a failure that is *not* the user saying no.
    let out = certutil()
        .args(["-user", "-addstore", "Root"])
        .arg(&cert)
        .output()
        .context("running certutil -addstore Root")?;
    if !out.status.success() {
        // A declined dialog and a crashed certutil are both "non-zero", and
        // reporting the second as the first told the user to click Trust again -
        // advice that cannot work, on the failure where the detail matters most.
        // Only the decline keeps the old sentence, which is what
        // `lib/errors.ts` matches to name the dialog and its button.
        if let Some(code) = crash_code(&out.status) {
            anyhow::bail!(
                "couldn't trust the proxy CA: certutil did not exit on its own, \
                 it was terminated by Windows with {code:#010x}{}",
                certutil_detail(&out)
            );
        }
        anyhow::bail!(
            "couldn't trust the proxy CA \u{2014} the certificate trust dialog was cancelled or denied{}",
            certutil_detail(&out)
        );
    }
    forget_trust_reading();
    Ok(())
}

/// Remove the CA's trust and its key material. `certutil -user -delstore Root
/// <CN>` deletes our cert from the per-user root store by common name; then the
/// private key and public cert are torn down so an explicit "remove" leaves
/// nothing behind.
///
/// One thing it cannot remove: a machine-wide root installed by
/// [`ensure_trusted_system`]. That store needs an elevated process, and this is
/// the path the GUI runs, where a UAC prompt for a state only a CLI flag can
/// create would be a surprise. So it finishes everything it can and then *says*
/// what is left rather than reporting a clean removal over a root that is still
/// trusted for every user on the box.
pub fn untrust() -> Result<()> {
    // Scoped to the per-user store, not `is_trusted()`: that now answers for
    // both stores, so on a machine where only the `--system-trust` copy exists
    // it would send `certutil -user -delstore` after a cert that isn't there
    // and turn a nothing-to-do into a hard failure.
    if per_user_store_has_our_ca() {
        let mut cmd = certutil();
        cmd.args(["-user", "-delstore", "Root", CA_COMMON_NAME]);
        let finished = certutil_bounded(cmd).context("running certutil -delstore Root")?;
        let status = finished.ok_or_else(|| certutil_timed_out("-delstore Root"))?;
        if !status.success() {
            anyhow::bail!("couldn't untrust the proxy CA (certutil -delstore Root failed)");
        }
        forget_trust_reading();
    }
    // Read before the teardown: the lookup is thumbprint-keyed on the cert file
    // that `remove_ca_material` is about to delete.
    let machine_wide = machine_store_has_our_ca();
    remove_ca_material()?;
    if machine_wide {
        anyhow::bail!(
            "the per-user trust and the CA's key material are gone, but a machine-wide copy of the CA is still in the LocalMachine Root store (installed with --system-trust). Remove it with `gate-connect proxy untrust-ca --system-trust` from an elevated prompt."
        );
    }
    Ok(())
}

/// Install the CA into the **machine** root store, without any dialog. The
/// headless counterpart of [`ensure_trusted`], reached only from `proxy
/// trust-ca --system-trust`.
///
/// `certutil -addstore Root` (no `-user`) writes `LocalMachine`, which requires
/// an elevated process and, precisely because the caller already holds
/// administrator rights, shows none of the "you are about to install a
/// certificate…" confirmation the per-user path triggers. That confirmation has
/// no keyboard-free answer, which is what makes the per-user path unusable on a
/// build agent.
///
/// What it widens: the CA becomes a trusted TLS root for **every user on this
/// machine**, where [`ensure_trusted`] deliberately stays per-user. `-f`
/// overwrites an existing entry rather than failing, so a re-run after the CA is
/// regenerated converges instead of stacking a second same-CN root.
///
/// Not elevation-detected up front: there is no reliable non-`unsafe` check, so
/// we run the command and translate its failure into an instruction.
pub fn ensure_trusted_system() -> Result<()> {
    if is_trusted()? {
        return Ok(());
    }
    // A prior install may have left a same-CN root (different key) in the
    // per-user store, where it would keep shadowing ours. Best-effort, and
    // non-privileged - this is the user's own store.
    let mut stale = certutil();
    stale.args(["-user", "-delstore", "Root", CA_COMMON_NAME]);
    let _ = certutil_bounded(stale);
    let cert = cert_path()?;
    let out = certutil()
        .args(["-addstore", "-f", "Root"])
        .arg(&cert)
        .output()
        .context("running certutil -addstore Root")?;
    if !out.status.success() {
        anyhow::bail!(
            "couldn't install the proxy CA machine-wide; `certutil -addstore Root` needs an elevated (Administrator) prompt{}",
            certutil_detail(&out)
        );
    }
    forget_trust_reading();
    Ok(())
}

/// Remove the machine-wide CA installed by [`ensure_trusted_system`], then the
/// key material. The headless counterpart of [`untrust`], and elevated for the
/// same reason the install is.
pub fn untrust_system() -> Result<()> {
    if machine_store_has_our_ca() {
        let out = certutil()
            .args(["-delstore", "Root", CA_COMMON_NAME])
            .output()
            .context("running certutil -delstore Root")?;
        if !out.status.success() {
            anyhow::bail!(
                "couldn't remove the machine-wide proxy CA; `certutil -delstore Root` needs an elevated (Administrator) prompt{}",
                certutil_detail(&out)
            );
        }
    }
    // A desktop Windows box can hold both installs at once, and the per-user
    // half is this process's to remove. Read before the teardown, which deletes
    // the cert file the thumbprint is computed from.
    if per_user_store_has_our_ca() {
        let mut stale = certutil();
        stale.args(["-user", "-delstore", "Root", CA_COMMON_NAME]);
        let _ = certutil_bounded(stale);
    }
    forget_trust_reading();
    remove_ca_material()
}

/// Whether the per-user root store holds our *current* CA, by thumbprint.
/// Unanswerable reads as "no" - see [`machine_store_has_our_ca`].
fn per_user_store_has_our_ca() -> bool {
    store_has_our_ca(Scope::User)
}

/// Whether the machine root store holds our *current* CA, by thumbprint. Any
/// failure to answer (no cert on disk, certutil missing) reads as "no", because
/// every caller uses this to decide whether to attempt an elevated removal or to
/// warn about a leftover - and inventing a leftover would be the worse error.
fn machine_store_has_our_ca() -> bool {
    store_has_our_ca(Scope::Machine)
}

fn store_has_our_ca(scope: Scope) -> bool {
    cert_thumbprint()
        .ok()
        .flatten()
        .and_then(|t| root_store_has(scope, &t).ok())
        .unwrap_or(false)
}

/// The NTSTATUS code behind an exit that was a crash rather than a decision, or
/// `None` for an ordinary non-zero exit.
///
/// A process killed by an unhandled exception reports the exception as its exit
/// code, and those all carry NTSTATUS severity `ERROR` (the top two bits set:
/// `0xC0000005` access violation, `0xC0000409` stack buffer overrun). Nothing
/// certutil chooses to exit with looks like that, so the shape is what separates
/// "this host cannot run certutil" from "the user clicked No".
fn crash_code(status: &ExitStatus) -> Option<u32> {
    let code = status.code()? as u32;
    (code >> 30 == 0b11).then_some(code)
}

/// certutil's own complaint, appended to our message when there is one. Its
/// output is localized and often UTF-16, so this is `from_utf8_lossy` on
/// purpose: a mangled hint is still a hint, and it never feeds a decision (the
/// thumbprint path in [`cert_thumbprint`] is the one that must not be scraped).
fn certutil_detail(out: &std::process::Output) -> String {
    let text = String::from_utf8_lossy(&out.stderr);
    let text = text.trim();
    if text.is_empty() {
        String::new()
    } else {
        format!(": {text}")
    }
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
