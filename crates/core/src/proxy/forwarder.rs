//! Lifecycle for `gate-connect-forwarder`, the small separate binary the
//! machine-wide environment variables point at.
//!
//! The forwarding itself lives in that binary; this module only starts one,
//! proves the thing answering is ours, and stops it. See the binary's own
//! module docs for why it exists at all, and `docs/security-notes-loopback.md`
//! for its posture.

// Only the desktop managers run a forwarder: Linux routes through a daemon that
// already outlives the GUI, so it has never had the failure this fixes. Kept
// compiled everywhere rather than `cfg`-ed out, so a Linux build still
// type-checks it and its tests run on every OS - the same reason
// `manager_core::hosts_live_engine` carries this attribute.
#![cfg_attr(not(any(target_os = "macos", target_os = "windows")), allow(dead_code))]

use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context, Result};

/// How long to wait for a freshly spawned forwarder to be answering.
const SPAWN_TIMEOUT: Duration = Duration::from_secs(5);

/// Name of the sidecar binary, installed beside the app's own executable.
#[cfg(windows)]
const FORWARDER_BIN: &str = "gate-connect-forwarder.exe";
#[cfg(not(windows))]
const FORWARDER_BIN: &str = "gate-connect-forwarder";

fn proxy_file(name: &str) -> Result<PathBuf> {
    Ok(crate::env::app_support_dir()?.join("proxy").join(name))
}

/// Marker file whose presence means "the forwarder should be running". A file
/// rather than a signal: the same on all three platforms, and it cannot
/// mis-target a recycled PID.
///
/// The forwarder reads only whether it exists. Its *content* is for this
/// module: empty is wanted, [`DRAINING`] is a forwarder left to serve out the
/// login session after a disconnect ([`drain`]).
fn marker_path() -> Result<PathBuf> {
    proxy_file("forwarder-wanted")
}

/// Marker content meaning "keep serving what still holds our address, and ask
/// nobody to start another". See [`drain`].
const DRAINING: &[u8] = b"draining";

fn token_path() -> Result<PathBuf> {
    proxy_file("forwarder.token")
}

/// Whether anything has asked for a forwarder and nothing has since asked for
/// it to go: the marker is present and not [`DRAINING`]. [`stop`] removes it
/// and [`drain`] marks it, so a `false` here after an enable means "the user
/// asked Gate to let go of this machine", which is exactly what a supervisor
/// must not undo.
///
/// Only ever read under [`ENSURE_LOCK`], by [`ensure_running_supervised`].
/// Read outside it the answer is worth nothing: a `stop` landing between the
/// question and the marker write that follows would be overwritten by the
/// answer.
fn wanted() -> bool {
    marker_body().is_some_and(|body| body != DRAINING)
}

/// Whether the last word on the forwarder was [`drain`].
fn draining() -> bool {
    marker_body().is_some_and(|body| body == DRAINING)
}

/// The marker's content, `None` when there is no marker.
fn marker_body() -> Option<Vec<u8>> {
    std::fs::read(marker_path().ok()?).ok()
}

/// Serialises every write to the marker, the forwarder process and (on macOS)
/// the launch agent. Two callers racing on a dead forwarder would both fail
/// the health check and both spawn; the second binds a fresh port and
/// overwrites the port file under the first, leaving two forwarders resident
/// and the PAC and the exported variables naming different ports. The
/// manager's watcher runs an ensure on a timer, `enable` runs one under a
/// different lock, and [`stop`] tears the same state down, so all three can
/// meet here.
///
/// Held across a spawn and its answer-poll, so an ensure can own it for
/// [`SPAWN_TIMEOUT`] (twice that on macOS, where the launch agent is tried
/// first). Two consequences worth knowing rather than fixing: a `stop` on the
/// quit path waits that long in the worst case, which is the price of it being
/// airtight, and a user-facing `enable` can wait behind a supervisory pass
/// that is mid-spawn. The reverse cannot happen, because a supervisor takes
/// this lock with `try_lock` and skips its pass instead of queueing.
///
/// **Within this process only.** The CLI reaches `ensure_running` through
/// `tool_proxy_url` from a process of its own, so the two-forwarders race
/// above is still open across processes; `bind_preferred` keeps them on
/// distinct ports, so the cost is one orphaned forwarder on a port nothing
/// names until the marker goes. Closing it needs a cross-platform file lock
/// (`proxy::flock` is Linux-only, and this subsystem is not), which is not
/// worth it for a window that needs the app and the CLI to start an ensure in
/// the same few seconds.
static ENSURE_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// What one supervisory pass found. See [`ensure_running_supervised`].
pub enum Supervision {
    /// Another caller is already inside an ensure, so this pass did nothing
    /// and does not need to: that caller's work covers this moment.
    Busy,
    /// Nobody wants a forwarder, because [`stop`] retired the marker. Leaving
    /// it retired is the whole reason a supervisor asks.
    NotWanted,
    /// A forwarder is answering on this port, found or freshly started.
    Running(u16),
    /// One is wanted and could not be started.
    Failed(anyhow::Error),
}

/// The port the forwarder last bound, if it has ever run.
pub(crate) fn persisted_port() -> Option<u16> {
    super::port_persist::load("forwarder-port").ok().flatten()
}

/// Read the shared secret, minting one if this install has none.
///
/// 0600, because it is the only thing separating "our forwarder holds that
/// port" from "something else got there first". A local process running as the
/// owner can read it, which is the same-user boundary this subsystem already
/// accepts; a *different* local user cannot, which is the case that matters.
pub(super) fn load_or_create_token() -> Result<String> {
    let path = token_path()?;
    if let Ok(existing) = std::fs::read_to_string(&path) {
        let existing = existing.trim().to_string();
        if !existing.is_empty() {
            return Ok(existing);
        }
    }
    use rand::Rng;
    let token: String = {
        let mut rng = rand::thread_rng();
        (0..32)
            .map(|_| format!("{:02x}", rng.gen::<u8>()))
            .collect()
    };
    crate::primitives::write_file(&path, token.as_bytes(), 0o600)
        .with_context(|| format!("writing {}", path.display()))?;
    Ok(token)
}

/// Whether the listener on `port` can prove it holds our token.
///
/// A plain TCP connect proves only that *something* is listening, which after a
/// port reuse is not a claim worth making - and here it decides what gets
/// exported machine-wide as `HTTPS_PROXY`, so adopting a stranger's listener
/// would hand it every tool's destinations and the cleartext of every
/// plain-HTTP request.
///
/// Challenge-response rather than sending the token. An earlier version put the
/// secret on the request and accepted any `204`, which proved nothing - any
/// listener can answer `204` - and handed the secret to the very process it was
/// trying to identify, so a squatter both passed the check and harvested the
/// token for next time. Now the app sends a fresh random challenge and requires
/// the SHA-256 of token-then-challenge back; only a process that can read the
/// 0600 token file can produce it, and a captured reply cannot be replayed
/// against the next probe.
///
/// Hand-rolled over a `TcpStream` rather than through `reqwest`: it is one
/// request on loopback, and a client here would have to be told `.no_proxy()`
/// anyway, because the app may have just pointed `HTTPS_PROXY` at this port.
fn health_ok(port: u16, token: &str) -> bool {
    gate_connect_paths::proves_ours(port, gate_connect_paths::FORWARDER_HEALTH_PATH, token)
}

/// What a forwarder that proved itself says about the relay port.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RelayClaim {
    /// A build from before the forwarder fronted the relay, still running
    /// because nothing retires a forwarder on an app update: it answers only
    /// the proof that predates path binding, or it sent no readable relay
    /// header. It has to be replaced, or the relay stays in the GUI.
    Stale,
    /// A current build that holds no relay port right now.
    Nothing,
    /// A current build holding this relay port.
    Holds(u16),
}

/// [`health_ok`], keeping what the forwarder said about the relay. `None` when
/// nothing on `port` proves it is ours.
fn probe(port: u16, token: &str) -> Option<RelayClaim> {
    let path = gate_connect_paths::FORWARDER_HEALTH_PATH;
    let Some(headers) = gate_connect_paths::probe_with_proof(port, path, token) else {
        // A forwarder from before the proof bound its path answers only the
        // old proof. That is enough to know it is ours and due for retiring,
        // and it is never enough to trust it with anything.
        return gate_connect_paths::probe_with_legacy_proof(port, path, token)
            .map(|_| RelayClaim::Stale);
    };
    let claim = match headers
        .iter()
        .find(|(name, _)| name == gate_connect_paths::FORWARDER_RELAY_HEADER)
        .map(|(_, value)| value.as_str())
    {
        Some("none") => RelayClaim::Nothing,
        // A value that is neither "none" nor a port is not something a current
        // build sends, so it is read the way a missing header is.
        Some(value) => value.parse().map_or(RelayClaim::Stale, RelayClaim::Holds),
        None => RelayClaim::Stale,
    };
    Some(claim)
}

/// The relay port a running forwarder of ours is holding, when it is the one
/// relay tool configs name - which is the question "should the engine's relay
/// bind behind the forwarder?" and "does a relay config survive this process?"
/// both come down to.
///
/// `wait` covers the one case worth waiting for: a forwarder that holds
/// nothing while the relay port is free. It retries for the port on a one
/// second tick, so right after this process released it (a parked engine
/// stopped for a re-enable) the answer is about to change. A port somebody
/// else is live on is not about to change, and is answered at once.
///
/// That shortcut would misread one case, and does not reach it: a forwarder
/// socket-activated by launchd whose relay socket launchd already holds would
/// say "nothing" while the port is live. The forwarder collects launchd's
/// sockets before it serves anything, so it never answers in that state.
pub(crate) fn fronted_relay_port(wait: Duration) -> Option<u16> {
    // The port first, so a machine that never ran a forwarder - Linux, and
    // every status read there - does not mint a token as a side effect.
    let port = persisted_port()?;
    let token = load_or_create_token().ok()?;
    let deadline = std::time::Instant::now() + wait;
    loop {
        match probe(port, &token)? {
            RelayClaim::Holds(held) => {
                // Only the port configs name counts. The file has several writers
                // - the forwarder once it holds a port, the launch agent's
                // install choosing one for launchd, an enable or a headless
                // `proxy relay` that bound the public port itself - so a
                // mismatch means the configs name some other listener, and the
                // forwarder is not fronting them. It notices the same mismatch
                // and gives its port up.
                return (Some(held) == super::relay::load_persisted_port()).then_some(held);
            }
            RelayClaim::Stale => return None,
            RelayClaim::Nothing => {
                let taken = super::relay::load_persisted_port()
                    .is_some_and(gate_connect_paths::port_is_live);
                if taken || std::time::Instant::now() >= deadline {
                    return None;
                }
                std::thread::sleep(Duration::from_millis(100));
            }
        }
    }
}

/// Retire a forwarder too old to front the relay, so the ensure after this
/// starts a current one.
///
/// The marker is how every forwarder is asked to go, and it polls it every two
/// seconds, so this waits a little past that for the port to stop answering.
/// On macOS the launch agent goes too: launchd would otherwise start the old
/// binary again on the next connection, and a bootstrap over a loaded label is
/// refused, so the new plist would never take.
///
/// Costs the exported variables and the PAC a few seconds with nothing on
/// their port, once, on the first enable after the update. Everything holding
/// them goes direct in that window only if its client retries, which is the
/// price of not leaving the relay in the GUI indefinitely.
///
/// At most once per process. If the binary on disk is itself the old one - a
/// dev build, a partial update - the replacement is stale too, and retiring it
/// on every supervisory pass would take the forwarder's port away every thirty
/// seconds. After one attempt the stale forwarder is kept, doing its old job.
fn retire_stale(port: u16) -> bool {
    static RETIRED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    if RETIRED.swap(true, std::sync::atomic::Ordering::SeqCst) {
        return false;
    }
    if let Ok(marker) = marker_path() {
        let _ = std::fs::remove_file(marker);
    }
    #[cfg(target_os = "macos")]
    launch_agent::remove();
    // Waits for the port itself to go quiet: the old build cannot answer the
    // current proof, so "no longer proves itself" would be true at once.
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while std::time::Instant::now() < deadline && gate_connect_paths::port_is_live(port) {
        std::thread::sleep(Duration::from_millis(100));
    }
    true
}

/// Where the sidecar lives: beside the executable asking for it.
///
/// The bundler installs it next to the app binary (`externalBin`), and a dev
/// build puts both in the same `target/` directory, so one rule covers both.
fn forwarder_binary() -> Result<PathBuf> {
    let exe = std::env::current_exe().context("resolving current exe")?;
    let dir = exe
        .parent()
        .context("current exe has no parent directory")?;
    let candidate = dir.join(FORWARDER_BIN);
    if !candidate.exists() {
        anyhow::bail!(
            "the forwarder binary is not installed beside {} (looked for {})",
            exe.display(),
            candidate.display()
        );
    }
    Ok(candidate)
}

/// Spawn the forwarder detached, so it outlives this process.
///
/// `setsid` on Unix leaves the GUI's session and controlling terminal (still in
/// the login session, so it goes at logout - the lifetime we want);
/// `DETACHED_PROCESS` is the Windows equivalent.
fn spawn_detached() -> Result<()> {
    use std::process::{Command, Stdio};
    let mut cmd = Command::new(forwarder_binary()?);
    cmd.stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        // SAFETY: setsid is async-signal-safe; its only failure is "already a
        // session leader", which is harmless here.
        unsafe {
            cmd.pre_exec(|| {
                libc::setsid();
                Ok(())
            });
        }
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const DETACHED_PROCESS: u32 = 0x0000_0008;
        cmd.creation_flags(DETACHED_PROCESS);
    }
    cmd.spawn().context("spawning the environment forwarder")?;
    Ok(())
}

/// Socket activation, macOS only.
///
/// With a `Sockets` entry in a LaunchAgent plist, launchd binds and listens on
/// the port **itself, at login**, and starts the forwarder on the first
/// connection, handing over the already-listening descriptor. Three things
/// follow, and together they are why this is preferred over spawning:
///
/// - the address answers from login onward even with no Gate process in
///   existence, so there is no window in which a tool holding our exported
///   variables is stranded - not even between boot and the app launching;
/// - the forwarder need not be resident until something connects: launchd
///   starts it on the first connection, and starts another if it ever exits
///   with the agent still installed (it runs until the marker goes, so in
///   practice it stays up from the first connection to [`stop`]);
/// - the port cannot be squatted, because launchd took it before any other
///   process could. That removes the case the health token exists to catch,
///   rather than merely detecting it.
///
/// Everything here is best-effort and self-correcting: if the agent does not
/// produce a healthy forwarder, [`ensure_running`] removes it and falls back to
/// spawning one directly, which is the path the other platforms use anyway.
/// Compiled on every unix, called only on macOS. `launchctl` is not there to
/// answer on Linux, but a module that a Linux build never type-checks is one
/// whose errors surface for the first time on a Mac - and this is the piece
/// least often exercised.
#[cfg(unix)]
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
mod launch_agent {
    use super::*;

    const LABEL: &str = "ai.constellation.gate-connect.forwarder";
    /// Must match the socket name the forwarder asks `launch_activate_socket`
    /// for.
    const SOCKET_NAME: &str = "Forwarder";
    /// Must match the name the forwarder's relay listener asks for.
    const RELAY_SOCKET_NAME: &str = "Relay";

    fn plist_path() -> Result<PathBuf> {
        // Through the test seam, so the tests below never touch the real
        // `~/Library/LaunchAgents`; `$HOME` otherwise, as before.
        let home = crate::env::home().context("resolving the home directory")?;
        Ok(home
            .join("Library")
            .join("LaunchAgents")
            .join(format!("{LABEL}.plist")))
    }

    /// Pick the port launchd will hold. Reuses the persisted one so the
    /// variables exported by a previous session stay valid; otherwise takes a
    /// free one from the band and releases it for launchd to claim.
    fn choose_port() -> Result<u16> {
        if let Some(port) = persisted_port() {
            return Ok(port);
        }
        // Skip what this install already remembers, or launchd would hold the
        // engine's or relay's port for good and those listeners would move
        // every run - stranding exactly the baked configs they exist to keep.
        let taken = gate_connect_paths::remembered_ports_except("forwarder-port");
        let listener =
            gate_connect_paths::bind_fresh(&taken).context("choosing a forwarder port")?;
        let port = listener.local_addr()?.port();
        // Released so launchd can bind it. A third party could take it in the
        // gap, which is why `install` verifies with a health check afterwards
        // rather than assuming.
        drop(listener);
        Ok(port)
    }

    /// The relay port to hand launchd, if it can have one.
    ///
    /// The persisted one, when nothing else is live on it - or when the
    /// listener on it is this agent's own, because leaving it out would then
    /// rewrite the plist and bounce the agent on every ensure. Ours means the
    /// agent on disk declares it, or `fronting` names it: the forwarder has
    /// said it holds that port, which is how a plist written without it gets
    /// it back ([`needs_relay_socket`]). A port something else holds is left
    /// out: launchd cannot bind it, and the forwarder retries for it on its
    /// own once it is free. On a first run there is none, so one is chosen the
    /// way [`choose_port`] chooses and persisted here, as the forwarder's own
    /// port is.
    pub(super) fn choose_relay_port(
        forwarder_port: u16,
        fronting: Option<u16>,
    ) -> Result<Option<u16>> {
        match super::super::relay::load_persisted_port() {
            Some(port) => {
                let ours = declared_on_disk() == Some(port) || fronting == Some(port);
                Ok((ours || !gate_connect_paths::port_is_live(port)).then_some(port))
            }
            None => {
                let mut taken = gate_connect_paths::remembered_ports_except(
                    gate_connect_paths::RELAY_PORT_NAME,
                );
                taken.push(forwarder_port);
                let listener =
                    gate_connect_paths::bind_fresh(&taken).context("choosing a relay port")?;
                let port = listener.local_addr()?.port();
                drop(listener);
                super::super::relay::save_persisted_port(port)?;
                Ok(Some(port))
            }
        }
    }

    /// The relay port the agent on disk declares, if there is one.
    fn declared_on_disk() -> Option<u16> {
        let path = plist_path().ok()?;
        declared_relay_port(&std::fs::read_to_string(path).ok()?)
    }

    /// Whether an installed agent should be rewritten to hold `held`, the
    /// relay port its forwarder has just said it holds.
    ///
    /// The agent leaves the relay socket out when something else is live on
    /// the port at install time - normally this app's own engine, on a session
    /// that started before the forwarder did. The forwarder then binds the
    /// port itself once the engine lets go, and fronts it from then on, but
    /// only for as long as that process lives: after a logout launchd holds
    /// the forwarder's own socket and not this one, so relay tools find
    /// nothing on the port until something wakes the forwarder or Gate starts.
    /// Rewriting the plist once the port is the forwarder's is what closes
    /// that. Only for an agent that exists (a forwarder spawned directly after
    /// the agent failed has no plist to fix), only for the port configs name,
    /// and at most once per process, because the rewrite bounces the agent.
    pub(super) fn needs_relay_socket(held: u16) -> bool {
        static ASKED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
        let wanted = plist_path().is_ok_and(|p| p.exists())
            && super::super::relay::load_persisted_port() == Some(held)
            && declared_on_disk() != Some(held);
        wanted && !ASKED.swap(true, std::sync::atomic::Ordering::SeqCst)
    }

    /// The port an agent plist of ours declares for its relay socket, read
    /// out of the `Relay` dict itself rather than matched anywhere in the file.
    pub(super) fn declared_relay_port(plist: &str) -> Option<u16> {
        let dict = plist
            .split(&format!("<key>{RELAY_SOCKET_NAME}</key>"))
            .nth(1)?;
        let dict = dict.split("</dict>").next()?;
        let value = dict.split("<key>SockServiceName</key>").nth(1)?;
        value
            .trim_start()
            .strip_prefix("<string>")?
            .split("</string>")
            .next()?
            .parse()
            .ok()
    }

    pub(super) fn plist(binary: &std::path::Path, port: u16, relay_port: Option<u16>) -> String {
        // The relay socket is optional; see `choose_relay_port`.
        let relay = relay_port
            .map(|relay| {
                format!(
                    r#"
    <key>{RELAY_SOCKET_NAME}</key>
    <dict>
      <key>SockNodeName</key><string>127.0.0.1</string>
      <key>SockServiceName</key><string>{relay}</string>
      <key>SockType</key><string>stream</string>
      <key>SockFamily</key><string>IPv4</string>
    </dict>"#
                )
            })
            .unwrap_or_default();
        // Hand-written rather than via a plist crate: it is eleven keys, and a
        // dependency that can emit XML is not worth adding for a file this
        // shape. Values are a path we resolved and a number we chose, so there
        // is nothing here that needs escaping.
        format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key><string>{LABEL}</string>
  <key>ProgramArguments</key>
  <array><string>{}</string></array>
  <key>Sockets</key>
  <dict>
    <key>{SOCKET_NAME}</key>
    <dict>
      <key>SockNodeName</key><string>127.0.0.1</string>
      <key>SockServiceName</key><string>{port}</string>
      <key>SockType</key><string>stream</string>
      <key>SockFamily</key><string>IPv4</string>
    </dict>{relay}
  </dict>
  <key>ProcessType</key><string>Background</string>
</dict>
</plist>
"#,
            binary.display()
        )
    }

    fn launchctl(args: &[&str]) -> bool {
        std::process::Command::new("/bin/launchctl")
            .args(args)
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }

    fn domain() -> String {
        // SAFETY: `getuid` takes no arguments and cannot fail.
        format!("gui/{}", unsafe { libc::getuid() })
    }

    /// Install (or refresh) the agent and return the port launchd is holding.
    /// `fronting` is a relay port the running forwarder holds; see
    /// [`choose_relay_port`].
    pub(super) fn install(fronting: Option<u16>) -> Result<u16> {
        let binary = forwarder_binary()?;
        let port = choose_port()?;
        let relay_port = choose_relay_port(port, fronting)?;
        let path = plist_path()?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating {}", parent.display()))?;
        }
        let body = plist(&binary, port, relay_port);
        let refresh = std::fs::read_to_string(&path)
            .map(|old| old != body)
            .unwrap_or(true);
        if refresh {
            std::fs::write(&path, &body).with_context(|| format!("writing {}", path.display()))?;
            // Bootout first so a changed plist (new port, or a new binary path
            // after an app update) actually takes effect; a bootstrap over a
            // loaded label is refused.
            let target = format!("{}/{LABEL}", domain());
            launchctl(&["bootout", &target]);
        }
        let path_str = path.to_string_lossy().to_string();
        if !launchctl(&["bootstrap", &domain(), &path_str]) {
            // Already loaded is the common "failure" here, and is fine: the
            // health check below is what decides whether this worked.
        }
        // Record the port only once launchd has it, so the file the app reads
        // never names a port nothing is holding.
        crate::proxy::port_persist::save("forwarder-port", port)
            .context("recording the forwarder port")?;
        Ok(port)
    }

    /// Remove the agent's plist and leave the loaded job alone, so launchd
    /// keeps holding the sockets and starting the forwarder on a connection
    /// until the login session ends, and loads nothing at the next login.
    /// Checked on a real launchd by `tests/launchd_plist_removed.rs`.
    pub(super) fn forget() {
        if let Ok(path) = plist_path() {
            let _ = std::fs::remove_file(path);
        }
    }

    /// Whether launchd has the job loaded, plist or not.
    fn loaded() -> bool {
        launchctl(&["print", &format!("{}/{LABEL}", domain())])
    }

    /// Write back the plist [`forget`] deleted, without touching the loaded
    /// job. Only when the job is still loaded and the file is gone: a
    /// forwarder spawned directly after the agent failed has no agent to
    /// restore, and writing one here would create an agent that never worked.
    pub(super) fn restore_file(fronting: Option<u16>) {
        let Ok(path) = plist_path() else { return };
        if path.exists() || !loaded() {
            return;
        }
        let written = forwarder_binary().and_then(|binary| {
            let port = choose_port()?;
            let relay_port = choose_relay_port(port, fronting)?;
            std::fs::write(&path, plist(&binary, port, relay_port))
                .with_context(|| format!("writing {}", path.display()))
        });
        if let Err(e) = written {
            eprintln!("gate proxy: could not restore the forwarder launch agent ({e:#})");
        }
    }

    /// Remove the agent entirely. Used when it did not work, and when the user
    /// asks Gate to let go of the machine.
    pub(super) fn remove() {
        launchctl(&["bootout", &format!("{}/{LABEL}", domain())]);
        if let Ok(path) = plist_path() {
            let _ = std::fs::remove_file(path);
        }
    }
}

/// Ensure a forwarder is running and return the port the machine-wide variables
/// should name.
///
/// Idempotent, and cheap in the common case: an already-running forwarder costs
/// one loopback request. The marker is written first so a forwarder that starts
/// fast cannot read it before it exists and exit immediately.
pub(crate) fn ensure_running() -> Result<u16> {
    // A poisoned lock only means an earlier caller panicked mid-ensure; the
    // state it guards is on disk and self-correcting, so carry on.
    let _serial = ENSURE_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    ensure_running_locked()
}

/// [`ensure_running`] for a supervisor rather than for a caller who is about
/// to write the port somewhere.
///
/// Two differences, both about not getting in the way of the user. It gives up
/// rather than queueing when another caller is already inside an ensure, so a
/// periodic pass can never be the reason an enable waits. And it reads the
/// marker *under* [`ENSURE_LOCK`], together with the write that follows, so a
/// [`stop`] cannot land between the two: without that, a pass that had already
/// asked "is one wanted?" went on to recreate the marker, the process and (on
/// macOS) the launch agent after the user had asked Gate to let go of this
/// machine.
pub(crate) fn ensure_running_supervised() -> Supervision {
    let _serial = match ENSURE_LOCK.try_lock() {
        Ok(guard) => guard,
        Err(std::sync::TryLockError::WouldBlock) => return Supervision::Busy,
        Err(std::sync::TryLockError::Poisoned(e)) => e.into_inner(),
    };
    if !wanted() {
        return Supervision::NotWanted;
    }
    match ensure_running_locked() {
        Ok(port) => Supervision::Running(port),
        Err(e) => Supervision::Failed(e),
    }
}

/// The body of an ensure. Callers hold [`ENSURE_LOCK`].
fn ensure_running_locked() -> Result<u16> {
    // Read before the write below, which is what cancels a drain.
    let was_draining = draining();
    let marker = marker_path()?;
    if let Some(parent) = marker.parent() {
        std::fs::create_dir_all(parent).ok();
    }
    crate::primitives::write_file(&marker, b"", 0o600)
        .with_context(|| format!("writing {}", marker.display()))?;
    let token = load_or_create_token()?;

    // A drain deleted the agent's plist and left the job loaded, so a
    // forwarder answering below is still launchd's. Put the file back, or the
    // probe returns early and the next login loads nothing: relay tools would
    // find no one on their port until Gate ran again. The file only, because
    // the loaded job already holds these sockets and a bootstrap would bounce
    // it.
    #[cfg(target_os = "macos")]
    if was_draining {
        let fronting = persisted_port().and_then(|port| match probe(port, &token) {
            Some(RelayClaim::Holds(held)) => Some(held),
            _ => None,
        });
        launch_agent::restore_file(fronting);
    }
    #[cfg(not(target_os = "macos"))]
    let _ = was_draining;

    if let Some(port) = persisted_port() {
        match probe(port, &token) {
            Some(RelayClaim::Stale) => {
                if !retire_stale(port) {
                    eprintln!(
                        "gate proxy: the forwarder is a build that predates fronting the \
                         relay, and replacing it did not help (the installed binary is the \
                         old one); keeping it, so the relay stays in this process"
                    );
                    return Ok(port);
                }
                // The retire removed the marker, and a forwarder started
                // without one exits on its first poll.
                crate::primitives::write_file(&marker, b"", 0o600)
                    .with_context(|| format!("writing {}", marker.display()))?;
            }
            #[cfg(target_os = "macos")]
            Some(RelayClaim::Holds(held)) if launch_agent::needs_relay_socket(held) => {
                match launch_agent::install(Some(held)) {
                    Ok(port) if await_health(port, &token) => return Ok(port),
                    Ok(_) => {
                        eprintln!(
                            "gate proxy: the forwarder launch agent did not answer after adding \
                             the relay socket; removing it and starting the forwarder directly"
                        );
                        launch_agent::remove();
                    }
                    Err(e) => eprintln!(
                        "gate proxy: could not add the relay socket to the forwarder launch \
                         agent ({e:#})"
                    ),
                }
            }
            Some(_) => return Ok(port),
            None => {}
        }
    }

    // Never start one from a test run. `GATE_CONNECT_TEST_HOME` is the seam
    // that makes every per-user path hermetic, and `audit::` already reads it
    // as "this is a test, do not reach outside the sandbox"; a forwarder is
    // the strongest reason yet to do the same, because it is a detached
    // process that deliberately outlives the run that started it. On Windows
    // it stays inside the CI runner's job object, so the step waits for a
    // process built never to exit: a `cargo test` that normally takes three
    // minutes ran for an hour before this guard existed. Reusing a forwarder
    // that is genuinely up is checked above and still allowed - this refuses
    // only to create one - and the callers' fallback is the engine's own port,
    // which is what they did before there was a forwarder at all.
    if crate::env::test_seam("GATE_CONNECT_TEST_HOME").is_some_and(|v| !v.is_empty()) {
        anyhow::bail!(
            "refusing to spawn the environment forwarder under GATE_CONNECT_TEST_HOME; \
             callers fall back to the engine's own port"
        );
    }

    // macOS: let launchd own the socket if it will. Verified rather than
    // assumed - if the agent does not produce a forwarder that answers, it is
    // removed and we fall back to spawning one, which is what the other
    // platforms do anyway.
    #[cfg(target_os = "macos")]
    {
        match launch_agent::install(None) {
            Ok(port) => {
                if await_health(port, &token) {
                    return Ok(port);
                }
                eprintln!(
                    "gate proxy: the forwarder launch agent did not answer; removing it and \
                     starting the forwarder directly"
                );
                launch_agent::remove();
            }
            Err(e) => eprintln!("gate proxy: could not install the forwarder launch agent ({e:#})"),
        }
    }

    spawn_detached()?;

    let deadline = std::time::Instant::now() + SPAWN_TIMEOUT;
    while std::time::Instant::now() < deadline {
        if let Some(port) = persisted_port() {
            if health_ok(port, &token) {
                return Ok(port);
            }
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    anyhow::bail!("the environment forwarder did not answer within {SPAWN_TIMEOUT:?}")
}

/// Poll `port` until it answers as ours, or the spawn budget runs out.
///
/// macOS only, like its one caller: the launch-agent branch is the only place
/// that hands back a port it has not yet watched come up. Everywhere else
/// `ensure_running` polls `persisted_port` itself after spawning.
///
/// Gated rather than left to the module's `allow(dead_code)` at the top of the
/// file, because that allow deliberately exempts macOS and Windows - so on
/// Windows this was a hard error under `-D warnings`, which is what CI reported.
#[cfg(target_os = "macos")]
fn await_health(port: u16, token: &str) -> bool {
    let deadline = std::time::Instant::now() + SPAWN_TIMEOUT;
    while std::time::Instant::now() < deadline {
        if health_ok(port, token) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    false
}

/// Ask a running forwarder to exit, by removing the marker it polls.
///
/// Best-effort and promptless. Deliberately *not* called from `disable`: a
/// forwarder that went away when routing was switched off would strand exactly
/// the processes it exists to protect. The callers are the explicit "Gate
/// should let go of this machine" paths - signing out and untrusting the CA.
/// The quit that disconnects the tools used to be one; it [`drain`]s instead.
///
/// Takes [`ENSURE_LOCK`] so it cannot interleave with an ensure, which would
/// otherwise write the marker back moments after this removed it. That means
/// waiting out an ensure that is mid-spawn; see the lock's own documentation.
pub fn stop() {
    let _serial = ENSURE_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Ok(path) = marker_path() {
        let _ = std::fs::remove_file(path);
    }
    // And take the agent down, or launchd would start a fresh forwarder on the
    // next connection - the marker only retires the process, not the thing that
    // keeps re-creating it.
    #[cfg(target_os = "macos")]
    launch_agent::remove();
}

/// Let go of the machine without breaking what already holds our address.
///
/// [`stop`] ends the forwarder at once, so every tool still running with its
/// address in memory - a proxy URL, a relay base URL, an exported
/// `HTTPS_PROXY` - gets connection refused until it is reopened, and reopened
/// again once Gate reconnects it. This leaves the forwarder serving them,
/// direct as it does after any quit, until the login session ends: every such
/// process was started in this session and ends with it, so logout is the one
/// moment nothing can still need it. Nothing starts another at the next login.
///
/// - The marker stays, so the running forwarder does not exit, and is marked
///   [`DRAINING`], so a supervisory pass declines to respawn one. The next
///   foreground [`ensure_running`] writes it back to wanted.
/// - macOS: the agent's plist goes and the job stays loaded, so launchd keeps
///   the sockets and still starts the forwarder on a connection until logout,
///   and has no file to load at the next login.
/// - Windows: nothing else to do. The forwarder is a detached process nothing
///   but Gate starts, and it ends with the session.
///
/// Only the quit that disconnects the tools uses this. Signing out, untrusting
/// the CA and the uninstall still [`stop`].
pub fn drain() {
    let _serial = ENSURE_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let Ok(path) = marker_path() else { return };
    // Only a forwarder that was wanted is drained. With no marker there is
    // nothing running to keep, and writing one would leave a marker behind
    // that no forwarder asked for.
    if !path.exists() {
        return;
    }
    if let Err(e) = crate::primitives::write_file(&path, DRAINING, 0o600) {
        // A marker that cannot be marked would read as wanted, and a
        // supervisor would keep the forwarder and its agent alive. Stopping
        // is the old behaviour and the safe direction.
        eprintln!(
            "gate proxy: marking the forwarder as draining failed ({e:#}); stopping it instead"
        );
        let _ = std::fs::remove_file(path);
        #[cfg(target_os = "macos")]
        launch_agent::remove();
    } else {
        #[cfg(target_os = "macos")]
        launch_agent::forget();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Per-test hermetic home, serialized process-wide because the path seam is
    /// an environment variable.
    struct TestHome {
        dir: PathBuf,
        _guard: std::sync::MutexGuard<'static, ()>,
    }

    impl TestHome {
        fn set(tag: &str) -> Self {
            let guard = crate::env::path_env_lock();
            let dir =
                std::env::temp_dir().join(format!("gate-connect-fwd-{}-{tag}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            std::env::set_var("GATE_CONNECT_TEST_HOME", &dir);
            TestHome { dir, _guard: guard }
        }
    }

    impl Drop for TestHome {
        fn drop(&mut self) {
            std::env::remove_var("GATE_CONNECT_TEST_HOME");
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    #[test]
    fn a_minted_token_is_stable_and_not_guessable() {
        let _home = TestHome::set("token");
        let first = load_or_create_token().expect("mint");
        assert_eq!(first.len(), 64, "32 bytes, hex encoded");
        assert_ne!(first, "0".repeat(64));
        // Stable across calls: the forwarder reads the file once at startup, so
        // re-minting on each probe would make every health check fail.
        assert_eq!(first, load_or_create_token().expect("reuse"));
    }

    /// The point of the challenge. A listener that merely answers - including
    /// one that answers `204` to everything, which is exactly what a squatter
    /// would do - must not be adopted, because adopting it exports a stranger's
    /// port as the machine's `HTTPS_PROXY`.
    #[test]
    fn a_listener_that_cannot_prove_the_token_is_not_ours() {
        let _home = TestHome::set("health");

        for reply in [
            &b"HTTP/1.1 204 No Content\r\nConnection: close\r\n\r\n"[..],
            &b"HTTP/1.1 204 No Content\r\nx-gate-forwarder-proof: nope\r\n\r\n"[..],
        ] {
            let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
            let port = listener.local_addr().unwrap().port();
            std::thread::spawn(move || {
                use std::io::{Read, Write};
                if let Ok((mut sock, _)) = listener.accept() {
                    let mut buf = [0u8; 1024];
                    let _ = sock.read(&mut buf);
                    let _ = sock.write_all(reply);
                }
            });
            assert!(
                !health_ok(port, "the-token"),
                "answering without the proof must not be adopted"
            );
        }

        // And a port with nothing on it at all is not ours either.
        let free = {
            let l = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
            l.local_addr().unwrap().port()
        };
        assert!(!health_ok(free, "the-token"));
    }

    /// How a fake forwarder answers.
    #[derive(Clone, Copy)]
    enum Build {
        /// A current build, sending this relay header (`None` for none at all).
        Current(Option<&'static str>),
        /// A build from before the proof bound its path: the old proof, and no
        /// relay header.
        Legacy,
    }

    /// A forwarder of ours on `port`, answering every probe as `build` would.
    fn fake_forwarder(token: String, build: Build) -> u16 {
        let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            use std::io::{Read, Write};
            for sock in listener.incoming() {
                let Ok(mut sock) = sock else { continue };
                let mut buf = [0u8; 1024];
                let n = sock.read(&mut buf).unwrap_or(0);
                let req = String::from_utf8_lossy(&buf[..n]).to_string();
                let challenge = req
                    .lines()
                    .find_map(|l| {
                        let (name, value) = l.split_once(':')?;
                        name.trim()
                            .eq_ignore_ascii_case(gate_connect_paths::FORWARDER_CHALLENGE_HEADER)
                            .then(|| value.trim().to_string())
                    })
                    .unwrap_or_default();
                let (proof, relay) = match build {
                    Build::Current(relay) => (
                        gate_connect_paths::forwarder_proof(
                            &token,
                            gate_connect_paths::FORWARDER_HEALTH_PATH,
                            &challenge,
                        ),
                        relay,
                    ),
                    Build::Legacy => (
                        gate_connect_paths::legacy_forwarder_proof(&token, &challenge),
                        None,
                    ),
                };
                let relay = relay
                    .map(|r| format!("{}: {r}\r\n", gate_connect_paths::FORWARDER_RELAY_HEADER))
                    .unwrap_or_default();
                let _ = sock.write_all(
                    format!(
                        "HTTP/1.1 204 No Content\r\n{}: {proof}\r\n{relay}\
                         Connection: close\r\n\r\n",
                        gate_connect_paths::FORWARDER_PROOF_HEADER
                    )
                    .as_bytes(),
                );
            }
        });
        port
    }

    /// A forwarder left running across an update answers only the old proof,
    /// and reading it as "not ours" would leave it in place and the relay in
    /// the GUI until the next reboot. A current build's relay header is read
    /// three ways, and anything it would not send reads as stale too.
    #[test]
    fn a_forwarder_says_whether_it_holds_the_relay_and_an_old_one_is_stale() {
        let _home = TestHome::set("relay-claim");
        let token = load_or_create_token().unwrap();
        let old = fake_forwarder(token.clone(), Build::Legacy);
        let headerless = fake_forwarder(token.clone(), Build::Current(None));
        let garbled = fake_forwarder(token.clone(), Build::Current(Some("yes")));
        let idle = fake_forwarder(token.clone(), Build::Current(Some("none")));
        let holding = fake_forwarder(token.clone(), Build::Current(Some("47101")));
        assert_eq!(probe(old, &token), Some(RelayClaim::Stale));
        assert_eq!(probe(headerless, &token), Some(RelayClaim::Stale));
        assert_eq!(probe(garbled, &token), Some(RelayClaim::Stale));
        assert_eq!(probe(idle, &token), Some(RelayClaim::Nothing));
        assert_eq!(probe(holding, &token), Some(RelayClaim::Holds(47101)));
        assert_eq!(probe(holding, "another-token"), None);
        assert_eq!(probe(old, "another-token"), None);
    }

    /// The launch agent reads back the relay port it declared from the
    /// `Relay` dict itself, so a matching number elsewhere in the file (the
    /// forwarder's own socket) is not mistaken for it.
    #[cfg(unix)]
    #[test]
    fn the_declared_relay_port_is_read_from_the_relay_dict() {
        let body = launch_agent::plist(std::path::Path::new("/bin/fwd"), 47101, Some(47102));
        assert_eq!(launch_agent::declared_relay_port(&body), Some(47102));
        let body = launch_agent::plist(std::path::Path::new("/bin/fwd"), 47101, None);
        assert_eq!(launch_agent::declared_relay_port(&body), None);
    }

    /// An agent written while something else held the relay port leaves the
    /// socket out. Once its own forwarder holds that port, the next ensure
    /// puts it back - once - and the port counts as the agent's own rather
    /// than as somebody else's live listener.
    #[cfg(unix)]
    #[test]
    fn a_relay_socket_left_out_is_added_once_the_forwarder_holds_it() {
        let home = TestHome::set("relay-socket");
        let held = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let relay = held.local_addr().unwrap().port();
        super::super::relay::save_persisted_port(relay).unwrap();

        assert!(
            !launch_agent::needs_relay_socket(relay),
            "no agent installed, nothing to fix"
        );
        let agents = home.dir.join("Library").join("LaunchAgents");
        std::fs::create_dir_all(&agents).unwrap();
        let plist = agents.join("ai.constellation.gate-connect.forwarder.plist");
        std::fs::write(
            &plist,
            launch_agent::plist(std::path::Path::new("/bin/fwd"), 47101, None),
        )
        .unwrap();

        assert_eq!(launch_agent::choose_relay_port(47101, None).unwrap(), None);
        assert_eq!(
            launch_agent::choose_relay_port(47101, Some(relay)).unwrap(),
            Some(relay)
        );
        assert!(
            !launch_agent::needs_relay_socket(relay + 1),
            "only the port configs name"
        );
        assert!(launch_agent::needs_relay_socket(relay));
        assert!(
            !launch_agent::needs_relay_socket(relay),
            "at most once per process: the rewrite bounces the agent"
        );
        drop(held);
    }

    /// Fronted means holding the port relay configs name - not merely holding
    /// some relay port.
    #[test]
    fn only_the_port_configs_name_counts_as_fronted() {
        let _home = TestHome::set("fronted");
        let token = load_or_create_token().unwrap();
        let port = fake_forwarder(token.clone(), Build::Current(Some("47101")));
        super::super::port_persist::save("forwarder-port", port).unwrap();

        super::super::relay::save_persisted_port(47101).unwrap();
        assert_eq!(fronted_relay_port(Duration::ZERO), Some(47101));

        super::super::relay::save_persisted_port(47102).unwrap();
        assert_eq!(fronted_relay_port(Duration::ZERO), None);
    }

    /// What a quit reads with a live forwarder holding the relay port: a
    /// relay config survives an ordinary quit, and dies on an exit the
    /// forwarder does not outlive either.
    #[test]
    fn a_fronted_relay_survives_a_quit_but_not_the_forwarder() {
        let _home = TestHome::set("quit-fronted");
        let token = load_or_create_token().unwrap();
        let port = fake_forwarder(token.clone(), Build::Current(Some("47101")));
        super::super::port_persist::save("forwarder-port", port).unwrap();
        super::super::relay::save_persisted_port(47101).unwrap();
        let config = "http://127.0.0.1:47101/__gate/t/codex/openai/v1";

        assert!(!crate::proxy::QuitAddresses::current().dies(config));
        assert!(crate::proxy::QuitAddresses::relay_unfronted().dies(config));

        // With the forwarder holding some other port, an ordinary quit
        // reverts too, as it did before the forwarder fronted anything.
        super::super::relay::save_persisted_port(47102).unwrap();
        assert!(crate::proxy::QuitAddresses::current()
            .dies("http://127.0.0.1:47102/__gate/t/codex/openai/v1"));
    }

    /// A stale forwarder is never "fronting", however long the caller waits.
    #[test]
    fn a_stale_forwarder_is_not_waited_for() {
        let _home = TestHome::set("stale-wait");
        let token = load_or_create_token().unwrap();
        let port = fake_forwarder(token, Build::Legacy);
        super::super::port_persist::save("forwarder-port", port).unwrap();
        super::super::relay::save_persisted_port(47101).unwrap();
        let started = std::time::Instant::now();
        assert_eq!(fronted_relay_port(Duration::from_secs(5)), None);
        // Well under the five seconds asked for; generous for a loaded runner.
        assert!(started.elapsed() < Duration::from_secs(3));
    }

    /// The drain keeps the marker - the forwarder reads only whether it
    /// exists, so a running one keeps serving - while a supervisory pass reads
    /// it as not wanted and declines to respawn anything.
    #[test]
    fn a_drained_forwarder_keeps_its_marker_and_is_not_supervised() {
        let _home = TestHome::set("drain");
        let marker = marker_path().unwrap();
        std::fs::create_dir_all(marker.parent().unwrap()).unwrap();
        crate::primitives::write_file(&marker, b"", 0o600).unwrap();
        assert!(wanted());

        drain();

        assert!(
            marker.exists(),
            "a drain that removed the marker would stop the forwarder"
        );
        assert!(draining());
        assert!(!wanted());
        assert!(matches!(
            ensure_running_supervised(),
            Supervision::NotWanted
        ));
    }

    /// Nothing to drain means nothing is written: a marker no forwarder asked
    /// for would be left behind for good.
    #[test]
    fn draining_with_no_forwarder_writes_nothing() {
        let _home = TestHome::set("drain-none");
        drain();
        assert!(!marker_path().unwrap().exists());
        assert!(!draining());
    }

    /// The next start cancels a drain: a foreground ensure finding the drained
    /// forwarder still answering adopts it and marks it wanted again.
    #[test]
    fn an_ensure_after_a_drain_wants_the_forwarder_again() {
        let _home = TestHome::set("drain-ensure");
        let token = load_or_create_token().unwrap();
        let port = fake_forwarder(token, Build::Current(Some("none")));
        super::super::port_persist::save("forwarder-port", port).unwrap();
        let marker = marker_path().unwrap();
        crate::primitives::write_file(&marker, b"", 0o600).unwrap();
        drain();
        assert!(draining());

        assert_eq!(ensure_running().unwrap(), port);
        assert!(wanted());
        assert!(!draining());
    }

    /// A stop after a drain still ends it: the marker goes either way.
    #[test]
    fn a_stop_after_a_drain_removes_the_marker() {
        let _home = TestHome::set("drain-stop");
        let marker = marker_path().unwrap();
        std::fs::create_dir_all(marker.parent().unwrap()).unwrap();
        crate::primitives::write_file(&marker, b"", 0o600).unwrap();
        drain();
        stop();
        assert!(!marker.exists());
    }

    /// The other half: a listener that *can* prove it is adopted.
    #[test]
    fn a_listener_that_proves_the_token_is_ours() {
        let _home = TestHome::set("health-ok");
        let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            use std::io::{Read, Write};
            let (mut sock, _) = listener.accept().unwrap();
            let mut buf = [0u8; 1024];
            let n = sock.read(&mut buf).unwrap_or(0);
            let req = String::from_utf8_lossy(&buf[..n]).to_string();
            let challenge = req
                .lines()
                .find_map(|l| {
                    let (name, value) = l.split_once(':')?;
                    name.trim()
                        .eq_ignore_ascii_case(gate_connect_paths::FORWARDER_CHALLENGE_HEADER)
                        .then(|| value.trim().to_string())
                })
                .unwrap_or_default();
            let proof = gate_connect_paths::forwarder_proof(
                "the-token",
                gate_connect_paths::FORWARDER_HEALTH_PATH,
                &challenge,
            );
            let _ = sock.write_all(
                format!(
                    "HTTP/1.1 204 No Content\r\n{}: {proof}\r\nConnection: close\r\n\r\n",
                    gate_connect_paths::FORWARDER_PROOF_HEADER
                )
                .as_bytes(),
            );
        });
        assert!(health_ok(port, "the-token"));
    }
}
