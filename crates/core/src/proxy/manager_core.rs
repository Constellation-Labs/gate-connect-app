//! The desktop proxy-manager sequencing, shared by macOS and Windows.
//!
//! One implementation, generic over [`DesktopOps`] - the seam holding every
//! call that touches the OS (system-proxy settings, trust store, persisted
//! ports). The two platforms used to carry ~600-line near-identical copies of
//! this file whose only real differences were those calls: every sequencing
//! fix had to land twice and could be reasoned about only per-platform, and
//! nothing drove the sequencing in a test because the real calls mutate the
//! host's proxy settings. Now the OS wiring lives in `manager_desktop.rs`
//! (`OsOps`), Linux keeps its structurally different daemon manager
//! (`manager_linux.rs`), and the tests below run the real engine against a
//! fake platform on every OS.
//!
//! Privilege model: changing the system proxy does *not* require admin, so
//! enable/disable/restore/reconcile run it unprivileged and promptless. The
//! only step that needs admin is trusting the CA, which happens once on
//! enable. Critically, the system-proxy revert never depends on an admin
//! prompt - so it can't be canceled and leave HTTPS routed at a dead port.
//! The CA is left trusted across disable so re-enabling is promptless;
//! removing it is a separate explicit action ([`DesktopManager::untrust_ca`]).

use std::sync::atomic::{AtomicBool, AtomicU16, AtomicU8, Ordering};
use std::sync::Mutex;
use std::time::Duration;

use anyhow::{Context, Result};

use super::forwarder::Supervision;
use super::{config, engine, ProxyDomain, ProxyState};
use crate::account;
use crate::audit;

/// The local CA the engine mints leaf certificates from, as PEM material.
/// Returned by [`DesktopOps::ca_load_or_create`] so the sequencing never sees
/// a platform trust-store type.
pub struct CaMaterial {
    pub cert_pem: String,
    pub key_pem: String,
}

/// The platform half of the desktop manager: everything that touches the OS,
/// behind one seam. Implemented by `OsOps` (macOS/Windows wiring, in
/// `manager_desktop.rs`) and by the test fake below. Method-level rationale
/// that is platform-specific (why macOS needs a service list, why Windows
/// env vars survive a reboot) lives on the `OsOps` impl.
pub trait DesktopOps: Send + Sync + 'static {
    /// Opaque saved system-proxy state. The sequencing only carries it
    /// between [`snapshot`](Self::snapshot), the save/load pair, and
    /// [`restore`](Self::restore).
    type Snapshot;

    /// Cheap, promptless preconditions checked before the CA prompt, so a
    /// machine that can't route refuses without bothering the user.
    fn preflight_enable(&self) -> Result<()>;
    /// Read the current system-proxy state (does not modify anything).
    fn snapshot(&self) -> Result<Self::Snapshot>;
    fn save_snapshot(&self, snapshot: &Self::Snapshot) -> Result<()>;
    /// `Ok(None)` when no snapshot is persisted; `Err` when one exists but
    /// cannot be read - callers treat that as an unclean prior session.
    fn load_snapshot(&self) -> Result<Option<Self::Snapshot>>;
    fn clear_snapshot(&self) -> Result<()>;
    /// Put the system proxy back exactly as the snapshot recorded it.
    fn restore(&self, snapshot: &Self::Snapshot) -> Result<()>;
    /// Turn the system proxy off without a snapshot to restore from.
    fn force_off(&self) -> Result<()>;
    /// The user's pre-existing upstream proxy from the snapshot, kept as the
    /// PAC fallback so non-Gate traffic keeps flowing through it.
    fn upstream_proxy(&self, snapshot: &Self::Snapshot) -> Option<String>;
    /// Point the system proxy at the running engine's loopback PAC.
    /// Promptless.
    fn point_system_proxy_at(&self, running: &engine::RunningEngine) -> Result<()>;
    /// Best-effort: remember the engine/PAC ports for the next run.
    fn persist_ports(&self, running: &engine::RunningEngine);
    /// The persisted engine port to rebind, if any.
    fn preferred_engine_port(&self) -> Option<u16>;
    /// The persisted PAC port to rebind (and to report in a cross-process
    /// status), if any.
    fn preferred_pac_port(&self) -> Option<u16>;
    /// The running engine's PAC port. Behind the seam because the engine
    /// only carries a PAC listener on PAC-driven platforms.
    fn engine_pac_port(&self, running: &engine::RunningEngine) -> Option<u16>;
    fn enable_env(&self, port: u16) -> Result<()>;
    fn disable_env(&self) -> Result<()>;
    /// Start (or find) the environment forwarder and return the port the
    /// machine-wide variables should name. Behind the seam because it spawns a
    /// process, which no unit test should do.
    fn ensure_env_forwarder(&self) -> Result<u16>;
    /// One supervisory pass at the forwarder: ensure one is running, unless
    /// another caller is already inside an ensure, or nobody wants one any
    /// more. Behind the seam for the same reason as `ensure_env_forwarder`,
    /// and separate from it because this is the only one allowed to decline.
    fn supervise_env_forwarder(&self) -> Supervision;
    /// Ask a running forwarder to exit. Only for the explicit "Gate should let
    /// go of this machine" paths - never for a plain disable, which is exactly
    /// when the processes it protects still need it.
    fn stop_env_forwarder(&self);
    /// The relay port the forwarder is holding, if it holds the one relay tool
    /// configs name, waiting up to `wait` for it to take a port that has just
    /// come free. Behind the seam because it probes a live forwarder.
    fn fronted_relay_port(&self, wait: Duration) -> Option<u16>;
    /// Startup sweep: clear any proxy slot still pointed at a dead loopback
    /// listener. Returns what it cleared, for the log line.
    fn clear_stranded_loopback(&self) -> Result<Vec<String>>;
    /// The port of an engine another *process* is serving, if the persisted
    /// snapshot + port + a live listener say so.
    fn engine_hosted_elsewhere(&self) -> Option<u16>;
    fn ca_load_or_create(&self) -> Result<CaMaterial>;
    fn ca_ensure_trusted(&self) -> Result<()>;
    fn ca_ensure_trusted_system(&self) -> Result<()>;
    fn ca_untrust(&self) -> Result<()>;
    fn ca_untrust_system(&self) -> Result<()>;
    fn ca_is_trusted(&self) -> Result<bool>;
}

/// How often the watcher stats the domains file. A toggle from another process
/// is a human action, so a second's latency is imperceptible; the cost is one
/// `stat` per tick against a file in the app-support dir.
const WATCH_INTERVAL: Duration = Duration::from_millis(1000);

/// How often the forwarder watcher asks whether the forwarder still answers.
/// Its port is what the PAC, the exported variables and three tool configs
/// name, so a forwarder that has died leaves every one of them falling back to
/// direct until something starts it again. A tick on a healthy forwarder costs
/// one marker-file write, one read of the forwarder's token *file* and one
/// loopback probe (`forwarder::ensure_running`); no secret is read, so it is
/// safe on a timer (see `keychain::get_cached` for why that matters).
const FORWARDER_CHECK_INTERVAL: Duration = Duration::from_secs(30);

/// How long an enable waits for the forwarder to take the relay port once it
/// is free. The forwarder retries on a one second tick, so this is one tick
/// and a margin; it is only ever spent when the port is free and the forwarder
/// holds nothing, which is the first enable after a session that was not
/// fronted.
const RELAY_CLAIM_WAIT: Duration = Duration::from_millis(1500);

/// [`DesktopManager::forwarder_answering`] values: nothing has asked yet.
const FORWARDER_UNKNOWN: u8 = 0;
/// The last ensure found or started a forwarder that answered.
const FORWARDER_ANSWERING: u8 = 1;
/// The last ensure could neither find nor start one.
const FORWARDER_SILENT: u8 = 2;

/// What a teardown does with the engine this process is hosting.
///
/// The distinction exists because `launchctl unsetenv` (and the Windows
/// registry write beside it) only changes what processes started *afterwards*
/// inherit. Every shell, editor and CLI already running keeps the
/// `HTTPS_PROXY` we exported, so releasing the port under them turns "routing
/// off" into "no provider is reachable" - for tools that were never switched
/// on, and for software Gate does not manage, since the variables are
/// machine-wide.
enum Teardown {
    /// Keep the listeners bound and drop them to their credential-free
    /// fallbacks, so a process still holding our variables reaches its
    /// provider by the path it would have taken with Gate not installed.
    ///
    /// This is what Linux has always done - `manager_linux::disable_inner`
    /// ends in `set_passthrough()` rather than a stop - and the divergence is
    /// the whole of the bug. `helper::set_passthrough` is the same pair of
    /// calls against the daemon.
    Dormant,
    /// Join the engine and release its ports. For the paths where a parked
    /// listener would be wrong rather than merely idle: app exit (the process
    /// is going away, so there is nothing to park), and a gateway switch,
    /// whose whole point is that the engine must not outlive the account it
    /// was started with.
    Stop,
}

pub struct DesktopManager<O: DesktopOps> {
    ops: O,
    engine: Mutex<Option<engine::RunningEngine>>,
    /// An engine kept bound after routing was turned off, intercepting
    /// nothing. Separate from `engine` rather than a flag on it, because every
    /// reader of that field means "is this process routing?" - `status`,
    /// `hosts_live_engine`, the enable's idempotence check and the domain
    /// watcher's retirement all answer wrongly if a parked engine sits there.
    ///
    /// Lock order where both are taken: `engine`, then this.
    dormant: Mutex<Option<engine::RunningEngine>>,
    /// Whether a domain watcher is already running, so repeated enables don't
    /// stack them. Per-instance (not a process static) so tests can build
    /// managers side by side.
    watcher_alive: AtomicBool,
    /// Same, for the forwarder watcher ([`spawn_forwarder_watcher`]).
    ///
    /// [`spawn_forwarder_watcher`]: Self::spawn_forwarder_watcher
    forwarder_watcher_alive: AtomicBool,
    /// What the last `ensure_env_forwarder` found, one of the `FORWARDER_*`
    /// constants. Written by `enable` and [`forwarder_tick`], read by `status`
    /// so the UI can show routing degraded - the crash fail-safe makes an
    /// engine death visible, and this is the forwarder's only equivalent.
    ///
    /// [`forwarder_tick`]: Self::forwarder_tick
    forwarder_answering: AtomicU8,
    /// The port *this manager* last told the machine-wide variables to name,
    /// or 0 when it has not exported them this session. The watcher compares
    /// the forwarder's port against it so a forwarder that came back on a
    /// fresh port (its old one squatted, or the persisted file lost) reaches
    /// the variables and not only the PAC.
    ///
    /// Deliberately not "what the variables say": the Settings switch exports
    /// through `proxy::set_env_export`, which has no manager handle, so after
    /// an opt-in there this reads 0 and the next pass re-exports once
    /// redundantly. That costs one `launchctl setenv` (or one registry write
    /// and a settings broadcast) and cannot lose the user's prior values,
    /// because `proxy_env::snapshot_prior` refuses to re-record over a
    /// snapshot that already exists.
    exported_port: AtomicU16,
}

impl<O: DesktopOps> DesktopManager<O> {
    pub fn new(ops: O) -> Self {
        Self {
            ops,
            engine: Mutex::new(None),
            dormant: Mutex::new(None),
            watcher_alive: AtomicBool::new(false),
            forwarder_watcher_alive: AtomicBool::new(false),
            forwarder_answering: AtomicU8::new(FORWARDER_UNKNOWN),
            exported_port: AtomicU16::new(0),
        }
    }

    /// Park a no-longer-routing engine with its ports still bound.
    ///
    /// `set_intercept(false)` is what makes it a plain proxy: every CONNECT is
    /// blind-tunnelled instead of decrypted, and the relay forwards to the
    /// tool's real upstream under the tool's own credential. No leaf cert is
    /// minted, nothing is rewritten to the gateway, and the user's Gate
    /// credential is never attached.
    ///
    /// Clearing the domains is *not* what does that, and on its own would do
    /// the opposite: `route_rules` force-enables Claude Code's own entry
    /// exactly when the live set does not claim the host, so an empty set makes
    /// the selector path fire. It is cleared anyway so the engine's own
    /// `intercepting()` count reads zero, which is what `status` and the
    /// debug log report.
    fn park_dormant(&self, running: engine::RunningEngine) {
        running.set_intercept(false);
        running.update_domains(&[]);
        // Not spendable while parked, but it was still resident for the rest
        // of the session. A re-enable builds a fresh engine, so nothing needs
        // this copy back.
        running.clear_credentials();
        let mut slot = self.dormant.lock().expect("dormant engine mutex poisoned");
        // Take the old handle out before joining it: `stop()` blocks, and
        // holding this lock across it would make a second caller wait on a
        // thread join for no reason. Unreachable today - a routing engine
        // implies an empty `dormant` - but the ordering is free.
        let previous = slot.replace(running);
        drop(slot);
        if let Some(previous) = previous {
            previous.stop();
        }
    }

    /// Release a parked engine's ports, joining it so the address is free
    /// before a caller rebinds it. No-op when nothing is parked.
    fn stop_dormant(&self) {
        let parked = self
            .dormant
            .lock()
            .expect("dormant engine mutex poisoned")
            .take();
        if let Some(parked) = parked {
            parked.stop();
        }
    }

    /// Whether *this* process is hosting an engine that is still serving.
    ///
    /// The free half of [`crate::proxy::engine_listening`]: the host holds the
    /// handle, so it can answer without dialing its own port. Only a process
    /// that holds no handle has to probe, which keeps the cost off the menubar
    /// app - the one process that both hosts the engine and polls tool status
    /// on a timer.
    ///
    /// `try_lock`, never `lock`: this is called from status paths, and the
    /// holder it would otherwise wait on is an enable/disable running the whole
    /// sequence under the mutex. A missed lock is not "not running", it is "ask
    /// the port instead", which is what the caller does with `false`.
    ///
    /// Allowed to be dead code off the desktop platforms: this module also
    /// compiles under `test` on Linux so its own tests run everywhere, but
    /// there `manager()` is `manager_linux`, which carries its own version of
    /// this method, so nothing calls this one.
    #[cfg_attr(not(any(target_os = "macos", target_os = "windows")), allow(dead_code))]
    pub(crate) fn hosts_live_engine(&self) -> bool {
        self.engine
            .try_lock()
            .ok()
            .and_then(|g| g.as_ref().map(|e| !e.is_finished()))
            .unwrap_or(false)
    }

    /// Current subsystem snapshot for the UI.
    pub fn status(&self) -> Result<ProxyState> {
        let (port, pac_port) = self
            .engine
            .lock()
            .expect("proxy engine mutex poisoned")
            .as_ref()
            .map(|e| (Some(e.port()), self.ops.engine_pac_port(e)))
            .unwrap_or((None, None));
        // Holding no engine handle is not the same as nothing running: on
        // these platforms the engine lives in whichever process enabled it, so
        // a CLI invocation beside a routing menubar app has `None` here while
        // the machine is fully routed. Reporting "stopped" then is not a
        // partial truth, it is simply the wrong answer, and it read as one
        // during a real triage - `Proxy: stopped` printed directly above a
        // domain table read from disk that correctly said `anthropic on`.
        //
        // The ports come from the same files the hosting process wrote, so a
        // cross-process status names the engine that is actually serving
        // rather than the one this process would have started.
        // Only the hosting process watches the forwarder, so only it can say.
        let forwarder_answering = if port.is_some() {
            match self.forwarder_answering.load(Ordering::SeqCst) {
                FORWARDER_ANSWERING => Some(true),
                FORWARDER_SILENT => Some(false),
                _ => None,
            }
        } else {
            None
        };
        let (port, pac_port) = match port {
            Some(_) => (port, pac_port),
            None => match self.ops.engine_hosted_elsewhere() {
                Some(p) => (Some(p), self.ops.preferred_pac_port()),
                None => (None, None),
            },
        };
        Ok(ProxyState {
            running: port.is_some(),
            port,
            pac_port,
            ca_trusted: self.ops.ca_is_trusted()?,
            // Browsers here read the OS store, so there is no second one.
            ca_nss_trusted: None,
            env_export_opted_in: crate::proxy::env_export_opted_in(),
            env_export_separable: crate::proxy::env_export_is_separable(),
            forwarder_answering,
            domains: config::load_domains()?,
        })
    }

    /// No-op here: these platforms run the engine in-process, so there is no
    /// daemon whose lifetime could outlive the caller and nothing to detach
    /// from. Present so callers need no `cfg` around it.
    pub fn set_detached(&self, _detached: bool) {}

    pub fn list_domains(&self) -> Result<Vec<ProxyDomain>> {
        config::load_domains()
    }

    /// Start the engine, trust the CA, and route the system proxy through it.
    /// The CA trust is the only step that prompts for admin (first time); the
    /// proxy change is promptless. Idempotent: effectively a no-op if already
    /// running.
    ///
    /// `&'static self`: the domain watcher and the engine's crash fail-safe
    /// both outlive this call and need the manager. In production the manager
    /// is the process-global singleton, so every caller already has `'static`.
    pub fn enable(&'static self) -> Result<ProxyState> {
        // Hold the lock for the whole sequence: a concurrent enable must not
        // snapshot the system proxy after this one has already pointed it at
        // our engine - that snapshot would later "restore" a dead port.
        // (handle_engine_crash uses try_lock, so it can't deadlock on this.)
        let mut guard = self.engine.lock().expect("proxy engine mutex poisoned");
        if guard.is_some() {
            drop(guard);
            return self.status();
        }
        // The lock above only orders concurrent enables *within* this process.
        // Across processes there is nothing to hold, so this is where a second
        // one has to be refused: the comment above is exactly what happens
        // otherwise, and the snapshot it would take is of a machine already
        // pointed at the first engine. Restoring that later hands the user back
        // a PAC aimed at a port nothing answers - the one outcome this
        // subsystem is written to make impossible.
        //
        // Refusing rather than adopting, because there is nothing to adopt: no
        // daemon, no control socket, and the running engine belongs to another
        // process's memory. Linux adopts instead (`manager_linux`), and
        // `relay::serve` already refuses on the same ground.
        if let Some(other) = self.ops.engine_hosted_elsewhere() {
            drop(guard);
            anyhow::bail!(
                "the Gate proxy is already enabled, hosted by another process on \
                 127.0.0.1:{other}. Starting a second engine would take the system proxy \
                 over from it and record Gate's own settings as the ones to restore. Stop \
                 that one first: quit the Gate Connect app, or press Ctrl-C in the terminal \
                 running `gate-connect proxy enable`."
            );
        }

        let account = account::load()?
            .context("no Gate account configured - sign in before enabling the proxy")?;

        let domains = config::load_domains()?;
        // No enabled-domains guard here: the master switch owns whether the
        // engine runs, while providers/domains own what it intercepts. Starting
        // with zero enabled domains is valid (per-provider toggles can reach that
        // state at runtime too) and lets `provider::restore_all()` re-enable the
        // snapshotted domains immediately after start on master-on.
        self.ops.preflight_enable()?;

        let ca = self.ops.ca_load_or_create()?;

        // Trust the CA so the engine's minted leaf certs validate. Only step
        // that needs admin; prompts once, and only if not already trusted.
        self.ops.ca_ensure_trusted()?;

        // Snapshot the current system-proxy state *before* touching it, so
        // disable can restore it exactly.
        let snapshot = self.ops.snapshot()?;
        self.ops.save_snapshot(&snapshot)?;

        // Start the forwarder before anything can fetch the PAC, and hand its
        // port to both channels that outlive this process: the PAC and,
        // unless the user declined, the machine-wide variables. Browsers cache
        // the PAC they fetched, and they fetch it exactly when the system
        // proxy setting changes below - so the first body served has to be the
        // one they should keep. Every shell already running keeps the exported
        // variable for its whole life too; `launchctl unsetenv` and the
        // registry write beside it only reach processes started afterwards.
        // So the address either one carries has to keep answering after the
        // engine goes away, or routing off (or a crash) becomes "no provider
        // is reachable" for every already-running tool and every open browser.
        // The forwarder does: it hands connections to the engine while there
        // is one and goes direct when there is not. See `proxy::forwarder`.
        //
        // It needs nothing from the system proxy, and it reads the engine's
        // port files per connection rather than at startup, so it can run
        // before the engine exists - and has to, now that it also holds the
        // relay port the engine's relay binds behind (below). Falling back to
        // the engine's own port if it will not start: that is exactly what
        // shipped before there was a forwarder, so a forwarder problem costs
        // the fail-open property and nothing else.
        //
        // Before the park is released, not after: an ensure can take seconds
        // (a spawn, or retiring a forwarder left over from an older build), and
        // the tools the park is serving should not spend them with nothing on
        // their ports. And before the "another Gate Connect" refusal below, so
        // an enable that refuses has already started or adopted a forwarder.
        // That is intended: the forwarder is per-user and shared - another Gate
        // on this machine uses the same marker and port files - so what this
        // adopts is the one that Gate already runs, and a forwarder is wanted
        // for as long as any Gate wants routing.
        let forwarder_port = match self.ops.ensure_env_forwarder() {
            Ok(port) => Some(port),
            Err(e) => {
                eprintln!(
                    "gate proxy: could not start the environment forwarder ({e}); the PAC and \
                     the exported variables name the engine port instead, so browsers and \
                     tools will lose connectivity when routing is switched off until they \
                     are restarted"
                );
                None
            }
        };
        self.forwarder_answering.store(
            if forwarder_port.is_some() {
                FORWARDER_ANSWERING
            } else {
                FORWARDER_SILENT
            },
            Ordering::SeqCst,
        );
        // Release a parked engine now and not earlier. It has to go before the
        // bind, because it holds exactly the address `preferred_engine_port`
        // names and `bind_preferred` refuses to shadow a live listener - the
        // new engine would land on a fresh band port and rewrite the persisted
        // files under every tool config. And it has to go *after* the exits
        // above: a failed enable used to release the park on its way to
        // refusing, so "turn routing on, cancel the admin prompt" left the
        // already-running tools with neither routing nor passthrough. The one
        // exit between here and the bind is the "another Gate Connect" refusal
        // below, which fires only when another Gate holds these ports - so
        // there was no park of ours holding them to lose.
        self.stop_dormant();

        // Whether the forwarder holds the relay port every relay config names.
        // When it does, the engine's relay binds behind it and the forwarder
        // hands connections through; when the engine is gone - this process
        // quit or crashed - the forwarder serves them itself, straight to the
        // provider. That is what lets a relay config outlive this process, so
        // a quit no longer has to rewrite Codex's and OpenCode's configs (see
        // `proxy::address_dies_with_gui`). When it does not - no forwarder, a
        // stale one, or something else on the port - the engine binds the
        // public port itself, exactly as before the forwarder fronted it.
        //
        // After `stop_dormant`, because a park from a session that was not
        // fronted holds the public port, and the forwarder can only take it
        // once that is released; `RELAY_CLAIM_WAIT` covers its retry tick.
        let mut fronted =
            forwarder_port.and_then(|_| self.ops.fronted_relay_port(RELAY_CLAIM_WAIT));

        // Our own park is gone by now, so a relay still answering belongs to
        // somebody else - and it answers a challenge only Gate can, so this is
        // "another Gate Connect is on this machine's ports", not "something is
        // on that port".
        //
        // The wider of the two questions, and why there are two. The refusal
        // above asks `engine_hosted_elsewhere`, which counts only an instance
        // that is *routing*, because that is what `status` has to report. An
        // instance that is merely parked holds the same ports and routes
        // nothing, so it belongs here and not there. This one cannot move up
        // beside the other: until `stop_dormant` above, the relay answering
        // might be our own.
        //
        // `engine_hosted_elsewhere` above cannot see it: it reads the
        // system-proxy snapshot, and a parked instance cleared that on its way
        // to parking. Without this check the enable proceeded, found the
        // persisted ports held, fell back to fresh ones and rewrote the port
        // files - silently repointing every tool config on the machine at
        // addresses nothing names any more, and taking them out of reach of the
        // quit revert, which decides what to put back by comparing against
        // exactly those files.
        //
        // Deliberately not a refusal when a *stranger* holds the port: that is
        // recoverable on its own, because the fallback port is persisted, the
        // configs then read as drifted and the reconcile passes repair them.
        // Refusing there would let any local process keep Gate from starting.
        //
        // Behind the forwarder the question moves with the relay: the public
        // port answering is the forwarder itself, so it is the engine's own
        // relay port that has to be free - asked on the engine-only path,
        // because the forwarder answers the other two for anybody and a
        // squatter could relay one of those answers to pass as another Gate.
        //
        // And the forwarder holding the public port is not another Gate even
        // when its own health check said otherwise - it took the port since it
        // was asked, or its proxy listener would not answer while its relay
        // listener does. Its relay answer says which it is, so that decides
        // before anything is refused.
        if fronted.is_none() {
            if let Some(report) = crate::proxy::relay_report() {
                if report.fronted_by_forwarder {
                    fronted = crate::proxy::relay::load_persisted_port();
                }
            }
        }
        let relay_taken = match fronted {
            Some(_) => crate::proxy::relay::load_engine_port().is_some_and(|port| {
                crate::proxy::relay_report_at(port, gate_connect_paths::RELAY_ENGINE_HEALTH_PATH)
                    .is_some()
            }),
            None => crate::proxy::relay_listening(),
        };
        if relay_taken {
            anyhow::bail!(
                "another Gate Connect process is already using this machine's proxy ports; \
                 use that one, or quit it before enabling routing here"
            );
        }

        let running = engine::start(
            engine::EngineConfig {
                gateway_base_url: account.gateway_base_url.clone(),
                api_key: account.api_key.clone(),
                // Cognito access token to inject instead of the API key, when
                // a valid one is stored. Empty means fall back to the API
                // key; a later refresh pushes updates via `refresh_token`.
                oauth_token: crate::oauth::access_token_for_injection(),
                // Selected org, injected as X-Gate-Org-Id alongside the token.
                org_id: crate::account::org_id_for_injection(),
                domains: domains.clone(),
                ca_cert_pem: ca.cert_pem,
                ca_key_pem: ca.key_pem,
                // Reuse the port we bound last time: clients that resolved the
                // proxy once at their own launch (e.g. Claude Desktop) keep
                // dialing the old port across our restarts - an app upgrade
                // must come back on the same address or those clients break
                // until relaunched.
                preferred_port: self.ops.preferred_engine_port(),
                // Same for the PAC port: the AutoConfigURL a client captured
                // at its own launch must keep serving a fresh PAC, or its
                // fetch fails and it falls back to DIRECT, bypassing Gate.
                preferred_pac_port: self.ops.preferred_pac_port(),
                // Reuse the persisted relay port so CLI tool configs (which bake
                // http://127.0.0.1:<port>) stay valid across restarts. Behind
                // the forwarder that port is the forwarder's, and the engine
                // rebinds its own; nothing but the forwarder names that one.
                preferred_relay_port: if fronted.is_some() {
                    crate::proxy::relay::load_engine_port()
                } else {
                    crate::proxy::relay::load_persisted_port()
                },
                // Per-user UID gating is a Linux concern (the daemon's shared
                // loopback proxy); unresolvable for TCP peers here.
                owner_uid: None,
                // Keep any pre-existing proxy as the PAC fallback so non-Gate
                // traffic still flows through it while routing is on.
                upstream_proxy: self.ops.upstream_proxy(&snapshot),
            },
            // Fail-safe: if the engine dies unexpectedly, revert the system
            // proxy so traffic is never stranded at a dead listener.
            || self.handle_engine_crash(),
        )?;

        // Remember the ports for next time (best-effort).
        self.ops.persist_ports(&running);
        // Remember the relay port so the next run rebinds it and baked CLI
        // configs stay valid (best-effort). Behind the forwarder it goes in
        // `relay-engine-port`, for the forwarder to find.
        //
        // Asked again in exactly one case: the engine was meant to take the
        // public port and landed somewhere else, because something took the
        // port between the question and the bind. If that was the forwarder,
        // writing the engine's fallback into `relay-port` would repoint every
        // tool config away from the listener that outlives this process. Not
        // asked when the first answer was yes: a probe that failed on a busy
        // moment would then do exactly that repointing for no reason.
        let behind = fronted.is_some()
            || (forwarder_port.is_some()
                && Some(running.relay_port()) != crate::proxy::relay::load_persisted_port()
                && self.ops.fronted_relay_port(Duration::ZERO).is_some());
        if behind {
            let _ = crate::proxy::relay::save_engine_port(running.relay_port());
        } else {
            let _ = crate::proxy::relay::save_persisted_port(running.relay_port());
        }

        #[cfg(any(target_os = "macos", target_os = "windows"))]
        if let Some(port) = forwarder_port {
            running.set_pac_target(port);
        }

        // Point the system proxy at the engine's loopback PAC. Promptless. A
        // failure here leaves the forwarder running, which is fine: it is
        // built to outlive every process that started it.
        if let Err(e) = self.ops.point_system_proxy_at(&running) {
            running.stop();
            let _ = self.ops.clear_snapshot();
            return Err(e).context("enabling system proxy");
        }

        // Export the proxy variables too, unless the user declined. The PAC
        // above only reaches clients that consult the OS proxy setting; the CLI
        // AI tools read `HTTPS_PROXY` instead, and OpenCode has no proxy setting
        // of its own at all. Owned by the `env-proxy` integration, which is why
        // this is a choice and not unconditional - the variables are
        // machine-wide, so a user who turned them off must not get them back
        // here. Deliberately best-effort: an export failure must not take
        // routing down for everything that *does* follow the PAC, so it degrades
        // to "GUI apps routed, CLI tools not" rather than to "enable failed".
        self.exported_port.store(0, Ordering::SeqCst);
        if crate::proxy::env_export_opted_in() {
            let env_port = forwarder_port.unwrap_or_else(|| running.port());
            match self.ops.enable_env(env_port) {
                Ok(()) => self.exported_port.store(env_port, Ordering::SeqCst),
                Err(e) => eprintln!(
                    "gate proxy: could not export proxy environment variables ({e}); GUI apps \
                     still route through Gate, but CLI tools that read HTTPS_PROXY will not"
                ),
            }
        }

        *guard = Some(running);
        // The engine now has whatever the config said at startup. Keep it in
        // step with writes made by *other* processes for as long as it runs.
        self.spawn_domain_watcher();
        // And keep the forwarder answering for as long as it runs: every
        // address written above depends on that.
        self.spawn_forwarder_watcher();

        // The crash fail-safe defers while we hold the lock; if the engine
        // died somewhere in this sequence, revert here instead of leaving
        // HTTPS routed at a dead port with the snapshot already cleared.
        if guard.as_ref().is_some_and(|r| r.is_finished()) {
            if let Some(dead) = guard.take() {
                dead.stop();
            }
            let snapshot = self.ops.load_snapshot().unwrap_or_else(|e| {
                eprintln!("gate proxy: unreadable system-proxy snapshot ({e}); forcing proxy off");
                None
            });
            match snapshot {
                Some(snapshot) => self.ops.restore(&snapshot)?,
                None => self.ops.force_off()?,
            }
            let _ = self.ops.clear_snapshot();
            anyhow::bail!("proxy engine exited unexpectedly while enabling");
        }
        drop(guard);

        // Best-effort audit. The account is already loaded here, so its key is
        // the in-hand credential for ApiKey mode. `port` stays an Option: when
        // `status()` fails, the record says `null` rather than inventing a 0 that
        // a reader could not tell from a real port.
        let port = self.status().ok().and_then(|s| s.port);
        audit::proxy_enabled(&account.gateway_base_url, Some(&account.api_key), port);

        self.status()
    }

    /// Restore the prior system proxy and stop intercepting. Promptless and
    /// unconditional - the revert happens first and never depends on admin,
    /// so it can't be canceled and strand traffic. The CA is left trusted.
    ///
    /// The engine's ports stay bound, intercepting nothing, until something
    /// explicitly releases them - a quit, a gateway switch, a re-enable, or an
    /// untrust. See [`Teardown::Dormant`] for why releasing them *here* is
    /// what breaks every already-running tool.
    pub fn disable(&self) -> Result<ProxyState> {
        self.disable_inner(Teardown::Dormant)?;

        // Best-effort audit, deliberately here rather than in `disable_inner`:
        // `disable_quiet` shares that body and runs at app exit, which is not an
        // operator action, and a network call with a 5s ceiling on the quit path
        // is exactly the hang that function exists to avoid.
        //
        // `load_base_url` rather than `load`, because the URL is all this path
        // needs; `audit::credential` reaches for the key itself when the mode
        // calls for one, so passing `None` costs no coverage.
        if let Ok(Some(base_url)) = account::load_base_url() {
            audit::proxy_disabled(&base_url, None);
        }

        self.status()
    }

    /// Like [`disable`](Self::disable), but returns nothing instead of the
    /// resulting [`ProxyState`]. Used on app exit: `status()` calls
    /// `ca_is_trusted()`, which shells out to `certutil` on Windows, so
    /// computing a status the exiting process only discards spawns that probe
    /// on the shutdown path - where the child can be torn down mid-read and
    /// hang the quit. Reverting the proxy and stopping the engine never needs
    /// certutil.
    ///
    /// Stops rather than parks: the listeners live in this process, so they go
    /// when it does whatever we ask for here. Tools still holding our
    /// variables are stranded by the quit itself, which is the residual this
    /// change does not reach - closing it needs a listener that outlives the
    /// GUI, as Linux's daemon already is.
    pub fn disable_quiet(&self) -> Result<()> {
        self.disable_inner(Teardown::Stop)
    }

    /// Stop the engine so the next [`enable`](Self::enable) builds a fresh one
    /// from the current account.
    ///
    /// For a gateway switch. The engine takes `gateway_base_url` at start and
    /// keeps it - unlike the key, token, org, and domains, there is no live
    /// update for it - so a surviving engine would go on rewriting to the *old*
    /// environment's gateway while the refresh loop pushes the *new*
    /// environment's token into it, and that gateway rejects the bearer: a 401
    /// on every proxied call, with control-plane calls (which go direct) still
    /// working. Here that is exactly what a disable already does, since the
    /// engine lives in this process; the Linux manager has to go further and
    /// replace the daemon that outlives the GUI.
    pub fn shutdown_engine(&self) -> Result<()> {
        self.disable_inner(Teardown::Stop)
    }

    /// Shared body of [`disable`](Self::disable) /
    /// [`disable_quiet`](Self::disable_quiet): revert the system proxy and put
    /// the engine down the way `teardown` says, without computing status.
    fn disable_inner(&self, teardown: Teardown) -> Result<()> {
        // Hold the lock for the whole teardown, mirroring `enable`. Taking the
        // handle and releasing early left two windows for a concurrent enable:
        // before `stop()` it was falsely refused as "hosted by another
        // process" (the old engine still accepted and the snapshot still
        // existed), and between `stop()` and `clear_snapshot()` it proceeded
        // and then had its fresh snapshot deleted - routing on with no
        // snapshot, so cross-process status read "stopped" and the exit-time
        // disable fell to force_off. A crash callback that fires meanwhile
        // gives up its try_lock and defers to us - correct, since this IS the
        // revert it wanted to run.
        let mut guard = self.engine.lock().expect("proxy engine mutex poisoned");
        let running = guard.take();

        // Revert the exported variables first, and unconditionally: the PAC
        // restore below can fail with `?`, and of the two channels this is the
        // one where a stale value breaks tools outright rather than merely
        // failing open. A PAC left pointing at a dead port makes clients fall
        // back to DIRECT; an `HTTPS_PROXY` left pointing at a dead port makes
        // every request from a CLI tool fail to connect - and on Windows it
        // would survive the reboot.
        if let Err(e) = self.ops.disable_env() {
            eprintln!("gate proxy: {e}");
        }

        // An unreadable snapshot must not strand HTTPS at the dead engine
        // port - treat it like a missing one and force the proxy off.
        let snapshot = self.ops.load_snapshot().unwrap_or_else(|e| {
            eprintln!("gate proxy: unreadable system-proxy snapshot ({e}); forcing proxy off");
            None
        });
        // Revert first, put the engine down second - a live engine behind a
        // reverted proxy is harmless, the reverse strands HTTPS at a dead
        // port. But the engine has already been taken out of the guard, so
        // returning early on a failed revert would drop it, and
        // `RunningEngine::drop` does not join: the listeners would stay up
        // with no handle left to stop them, while our own state says nothing
        // is running. Keep the result, put it down, then report.
        let reverted = match snapshot {
            Some(snapshot) => self.ops.restore(&snapshot),
            None => self.ops.force_off(),
        };
        // Park only when the revert worked. A failed one returns early below
        // with the snapshot still on disk, so the next launch's reconcile can
        // retry it - and a live snapshot plus a live listener is exactly what
        // `engine_hosted_elsewhere` reads as "another process is routing",
        // which would make `status` report running with routing off. The
        // failure path keeps the old behaviour and releases the ports.
        let park = matches!(teardown, Teardown::Dormant) && reverted.is_ok();
        if let Some(running) = running {
            if park {
                self.park_dormant(running);
            } else {
                running.stop();
            }
        } else if !park {
            // Nothing routing, but a previous disable may have parked one -
            // an exit or a gateway switch has to take that down too.
            self.stop_dormant();
        }
        reverted?;
        let _ = self.ops.clear_snapshot();

        Ok(())
    }

    /// Toggle a domain. If the engine is running, the new rules are pushed
    /// live - no restart, no prompt.
    pub fn set_domain(&self, slug: &str, enabled: bool) -> Result<ProxyState> {
        let domains = config::set_enabled(slug, enabled)?;
        if let Some(running) = self
            .engine
            .lock()
            .expect("proxy engine mutex poisoned")
            .as_ref()
        {
            running.update_domains(&domains);
        }
        self.status()
    }

    /// Push a rotated Gate API key into the running engine, if any - the
    /// engine otherwise keeps injecting the key it was started with.
    pub fn refresh_api_key(&self, api_key: &str) {
        if let Some(running) = self
            .engine
            .lock()
            .expect("proxy engine mutex poisoned")
            .as_ref()
        {
            running.update_api_key(api_key);
        }
    }

    /// Push a refreshed OAuth access token into the running engine, if any.
    /// Empty string reverts to the API key. Used by the silent-refresh loop
    /// so a renewed token reaches in-flight routing without a restart.
    pub fn refresh_token(&self, oauth_token: &str) {
        if let Some(running) = self
            .engine
            .lock()
            .expect("proxy engine mutex poisoned")
            .as_ref()
        {
            running.update_token(oauth_token);
        }
    }

    /// Push a newly-selected org UUID into the running engine, if any. Empty
    /// string clears it. Used by the org switcher so the new `X-Gate-Org-Id`
    /// reaches in-flight routing without a restart.
    pub fn refresh_org(&self, org_id: &str) {
        if let Some(running) = self
            .engine
            .lock()
            .expect("proxy engine mutex poisoned")
            .as_ref()
        {
            running.update_org(org_id);
        }
    }

    /// Push a captured chatgpt.com `cf_clearance` cookie into the running
    /// engine, if any. Empty string clears it. Used by the challenge-solve
    /// webview so a freshly minted cookie reaches in-flight app turns without
    /// a restart.
    pub fn refresh_cf_clearance(&self, cf_clearance: &str) {
        // Recorded here rather than in the engine, so a cookie captured while
        // the engine is down (or restarted afterwards) is not lost with it;
        // see `proxy::LAST_CAPTURED_CF_CLEARANCE`.
        super::record_captured_cf_clearance(cf_clearance);
        if let Some(running) = self
            .engine
            .lock()
            .expect("proxy engine mutex poisoned")
            .as_ref()
        {
            running.update_cf_clearance(cf_clearance);
        }
    }

    /// Trust the CA without enabling the proxy (standalone command).
    pub fn trust_ca(&self) -> Result<ProxyState> {
        self.ops.ca_load_or_create()?; // ensure the cert file exists to trust
        self.ops.ca_ensure_trusted()?;
        self.status()
    }

    /// Trust the CA machine-wide with no prompt at all, for hosts where nobody
    /// can answer one. CLI-only (`proxy trust-ca --system-trust`) and never
    /// wired to a Tauri command: the prompt is deliberate product behaviour on a
    /// desktop, and this widens the trust to every user on the machine.
    pub fn trust_ca_system(&self) -> Result<ProxyState> {
        self.ops.ca_load_or_create()?; // ensure the cert file exists to trust
        self.ops.ca_ensure_trusted_system()?;
        self.status()
    }

    /// Untrust the CA. Refuses while the engine is running, since the engine
    /// mints leaf certs the OS would then reject. This is the explicit way to
    /// remove the standing trusted root (disable alone leaves it trusted).
    pub fn untrust_ca(&self) -> Result<ProxyState> {
        self.prepare_untrust()?;
        self.ops.ca_untrust()?;
        self.status()
    }

    /// Remove a machine-wide trust install with no prompt. The counterpart of
    /// [`trust_ca_system`](Self::trust_ca_system), and refuses while running
    /// for the same reason [`untrust_ca`](Self::untrust_ca) does.
    pub fn untrust_ca_system(&self) -> Result<ProxyState> {
        self.prepare_untrust()?;
        self.ops.ca_untrust_system()?;
        self.status()
    }

    fn prepare_untrust(&self) -> Result<()> {
        if self
            .engine
            .lock()
            .expect("proxy engine mutex poisoned")
            .is_some()
        {
            anyhow::bail!("turn the proxy off before untrusting the CA");
        }
        // Untrusting the CA is the explicit "Gate should let go of this
        // machine" action (it is what Reset runs), so both things Gate leaves
        // bound on this machine go with it.
        //
        // Not a refusal for a *parked* engine, which mints no leaf certs and so
        // cannot be invalidated by this - but leaving a listener bound
        // afterwards would be Gate still holding a port the user just asked it
        // to drop.
        self.stop_dormant();
        // The forwarder is the other one, and it is retired here for the same
        // reason. Not the only path: forgetting the workspace runs
        // `clear_account`, which retires it too, and that is the common one
        // (the Reset button). A plain disable must leave it running - that is
        // exactly when the processes holding our exported variables still need
        // it.
        self.ops.stop_env_forwarder();
        Ok(())
    }

    /// Fail-safe invoked from the engine thread if the engine exits without a
    /// deliberate stop. Drops the dead handle and reverts the system proxy so
    /// HTTPS isn't stranded. Promptless and best-effort.
    pub(crate) fn handle_engine_crash(&self) {
        // Deliberately not logged until we know *which* engine died: a parked
        // pass-through engine reaches this callback too, and there the line
        // below would claim a revert that neither happens nor is wanted.
        // Briefly retry the lock: short holders (status) clear in ms. If
        // enable or disable still holds it after that, defer - enable
        // re-checks the engine before returning and runs this same revert
        // itself, disable IS this revert, and restoring + clearing the
        // snapshot from here mid-sequence would erase the state they rely
        // on. (A deliberate stop sets `stopping` before signaling, so this
        // isn't reached on that path.)
        let mut guard = None;
        for _ in 0..20 {
            match self.engine.try_lock() {
                Ok(g) => {
                    guard = Some(g);
                    break;
                }
                Err(_) => std::thread::sleep(std::time::Duration::from_millis(50)),
            }
        }
        let Some(mut guard) = guard else {
            eprintln!("gate proxy: engine lock busy; deferring revert to the operation holding it");
            return;
        };
        // A parked engine carries this same callback, so its death lands here
        // too - and there is nothing to revert, because the disable that parked
        // it already did all of it. Falling through would be actively harmful:
        // the snapshot is gone by now, so the `force_off` below would fire and
        // switch off a system proxy the user owns, which the disable had
        // faithfully restored. Say nothing; routing is already off and the
        // shell is already drawing it that way.
        //
        // Gated on a parked engine actually being held, not merely on "no
        // routing engine": a crash *during* `enable` also arrives with an empty
        // guard, and that one still wants the revert below.
        //
        // The dead handle is deliberately not reaped here. `on_unexpected_exit`
        // runs on the engine's own thread, so `is_finished()` is still false at
        // this point and any check for it would never match; the next
        // `stop_dormant` (enable, exit, or untrust) joins it instead.
        if guard.is_none()
            && self
                .dormant
                .lock()
                .expect("dormant engine mutex poisoned")
                .is_some()
        {
            return;
        }
        eprintln!("gate proxy engine exited unexpectedly; reverting system proxy");
        // Join the engine instead of dropping it. `RunningEngine::drop` only
        // signals shutdown and returns - deliberately, to avoid blocking - so a
        // dropped engine's listeners stay bound for an unbounded moment after
        // we return here. Because the next enable prefers the persisted ports,
        // that is exactly the window in which it finds its own address still
        // live and moves off it. A genuinely crashed engine joins instantly,
        // its thread having already exited. The dead-engine reap in `enable`
        // calls `stop()` for the same reason.
        if let Some(running) = guard.take() {
            running.stop();
        }
        let _ = self.ops.disable_env();
        let _ = match self.ops.load_snapshot() {
            Ok(Some(snapshot)) => self.ops.restore(&snapshot),
            _ => self.ops.force_off(),
        };
        let _ = self.ops.clear_snapshot();
        drop(guard);
        // Traffic is safe again; now let the shell repaint. After the lock
        // drops, so the observer's own status read can't deadlock here.
        crate::proxy::notify_engine_crash_observer();
    }

    /// Called once at app startup to undo a system proxy left pointing at an
    /// engine that no longer exists (unclean quit / crash / OS shutdown).
    ///
    /// Two layers, because the graceful-disable path is bypassed by a hard
    /// kill: (1) a leftover snapshot restores the exact pre-Gate state; (2) a
    /// belt-and-suspenders sweep turns off any slot still pointed at a dead
    /// loopback listener even when no (or a partial) snapshot survives - in
    /// that case the PAC fetch fails and traffic silently falls back to
    /// DIRECT, bypassing Gate while it shows "off". Both are promptless, so
    /// this always succeeds; a clean disable makes it a near no-op.
    pub fn reconcile_on_startup(&self) -> Result<()> {
        // Clear any exported proxy variables left over from the prior session
        // before touching the PAC. If routing is meant to be on, `enable` runs
        // straight after this and re-exports them at the new engine port.
        if let Err(e) = self.ops.disable_env() {
            eprintln!("gate proxy: {e}");
        }
        // As in disable: an unreadable snapshot still means an unclean prior
        // session, so force the proxy off rather than bailing and leaving
        // HTTPS routed at a port nothing listens on.
        match self.ops.load_snapshot() {
            Ok(Some(snapshot)) => {
                self.ops.restore(&snapshot)?;
                self.ops.clear_snapshot()?;
            }
            Ok(None) => {}
            Err(e) => {
                eprintln!("gate proxy: unreadable system-proxy snapshot ({e}); forcing proxy off");
                self.ops.force_off()?;
                self.ops.clear_snapshot()?;
            }
        }
        let cleared = self.ops.clear_stranded_loopback()?;
        if !cleared.is_empty() {
            eprintln!(
                "[gate-proxy] startup: cleared stranded loopback proxy ({})",
                cleared.join(", ")
            );
        }
        Ok(())
    }

    /// Keep a running engine in step with the domains config after another
    /// process writes it.
    ///
    /// These platforms host the engine inside whichever process enabled it,
    /// and there is no daemon to forward changes to - so [`Self::set_domain`]
    /// updates the engine held by *its own* process and nothing else. From a
    /// second process (`gate-connect proxy domain <slug> on` while the menubar
    /// app is routing) that handle is `None`: the file was written, `proxy
    /// domains` reported the new set, and the engine went on intercepting the
    /// old one. Config and engine disagreeing is precisely the failure this
    /// subsystem is meant not to have, and Linux fixed its version of it in
    /// #120 by forwarding to the daemon.
    ///
    /// Watching the file rather than adding a control socket, because the file
    /// is already the contract between processes here, and reloading it is
    /// safe by construction: [`config::load_domains`] starts from the built-in
    /// catalog and applies only per-slug enabled flags, forcing unsupported
    /// entries off. So a reload can flip a catalog entry and can never point
    /// the MITM at a host the build does not ship - the same guarantee the
    /// Linux daemon enforces by validating requests against the catalog.
    ///
    /// Retires on its own when the engine stops, so a disable/enable cycle
    /// does not accumulate threads.
    fn spawn_domain_watcher(&'static self) {
        // `swap` rather than load-then-store: two enables racing here would
        // otherwise both see `false` and start a watcher each.
        if self.watcher_alive.swap(true, Ordering::SeqCst) {
            return;
        }
        std::thread::spawn(move || {
            let mut seen = config::domains_file_mtime();
            loop {
                std::thread::sleep(WATCH_INTERVAL);
                // Never hold the lock across the sleep above: enable/disable take
                // it for whole sequences, and this thread must not be what makes
                // a user-facing toggle wait.
                {
                    let guard = self.engine.lock().expect("proxy engine mutex poisoned");
                    if guard.is_none() {
                        // Retire *under the same lock acquisition* that observed
                        // the engine gone. `enable` installs the new engine and
                        // calls `spawn_domain_watcher` while holding this lock, so
                        // storing the flag here cannot interleave with a fresh
                        // enable's `swap(true)` - a load-then-store outside the
                        // lock could, leaving a running engine with no watcher
                        // when a disable/enable flip landed inside one tick.
                        self.watcher_alive.store(false, Ordering::SeqCst);
                        return;
                    }
                }
                let current = config::domains_file_mtime();
                if current == seen {
                    continue;
                }
                seen = current;
                let domains = match config::load_domains() {
                    Ok(d) => d,
                    // A torn or unreadable read is not worth acting on: the engine
                    // keeps the rules it has, and the next tick tries again.
                    Err(e) => {
                        eprintln!(
                            "gate proxy: could not reload proxy domains ({e}); keeping current"
                        );
                        continue;
                    }
                };
                if let Some(running) = self
                    .engine
                    .lock()
                    .expect("proxy engine mutex poisoned")
                    .as_ref()
                {
                    running.update_domains(&domains);
                }
            }
        });
    }

    /// Keep the forwarder answering for as long as this process hosts a
    /// routing engine.
    ///
    /// Nothing else supervises it. It is spawned detached so that it outlives
    /// the app, which also means no OS facility restarts it, and until this
    /// existed the only things that started one were an enable and a tool
    /// config write. A forwarder killed in Task Manager, or taken down with
    /// the app by an "End task" on the process tree, stayed dead until the
    /// next relaunch while the tray said Connected - and every address that
    /// named it fell back to direct with nothing on screen saying so.
    ///
    /// Same shape as [`spawn_domain_watcher`](Self::spawn_domain_watcher): one
    /// per manager, spawned under the engine lock, retiring under the lock
    /// acquisition that sees the engine gone. While routing is off the
    /// forwarder is not watched; the next enable or launch starts it again.
    /// The work is in [`forwarder_tick`](Self::forwarder_tick) so a test can
    /// drive it without the interval.
    fn spawn_forwarder_watcher(&'static self) {
        if self.forwarder_watcher_alive.swap(true, Ordering::SeqCst) {
            return;
        }
        std::thread::spawn(move || loop {
            std::thread::sleep(FORWARDER_CHECK_INTERVAL);
            if !self.forwarder_tick() {
                return;
            }
        });
    }

    /// One pass of the forwarder watcher. Returns `false` once the watcher
    /// should stop, which it does under the same lock acquisition that saw the
    /// engine gone (the reasoning is in `spawn_domain_watcher`).
    ///
    /// The pass itself is `forwarder::ensure_running_supervised`, which
    /// differs from the ensure `enable` runs in the two ways a supervisor
    /// needs. It declines when another caller is already inside an ensure, so
    /// a timer can never be why a user-facing toggle waits. And it decides
    /// "is one still wanted?" under the same lock as the write that follows,
    /// so a pass cannot resurrect a forwarder that `forwarder::stop` retired
    /// while an engine was still hosted, which the quit-with-disconnect path
    /// does.
    ///
    /// The ensure is idempotent: a live forwarder answers its health check and
    /// is reused, a dead one is replaced. The replacement normally rebinds the
    /// persisted port, so the addresses everything holds keep working
    /// unchanged. When it cannot (the port squatted, the file lost) the PAC is
    /// repointed here and the variables are re-exported; the three tool
    /// configs that name the forwarder read as drifted against the new port
    /// file and are repaired by the shell's next reconcile pass
    /// (`provider::reconcile_enabled`, on focus and at startup).
    ///
    /// Logs on transitions only, and records the outcome for `status`, which
    /// is how the UI learns that routing is on but nothing is being routed.
    fn forwarder_tick(&self) -> bool {
        {
            let guard = self.engine.lock().expect("proxy engine mutex poisoned");
            if guard.is_none() {
                self.forwarder_watcher_alive.store(false, Ordering::SeqCst);
                return false;
            }
        }
        let was_answering = self.forwarder_answering.load(Ordering::SeqCst) == FORWARDER_ANSWERING;
        // Not under the engine lock: on a dead forwarder this spawns a process
        // and waits for it to answer, and a user-facing toggle must not queue
        // behind that.
        match self.ops.supervise_env_forwarder() {
            // Someone else is doing this work right now. Saying anything about
            // the forwarder's health from here would be a guess about a state
            // that is mid-change.
            Supervision::Busy => {}
            // Retired on purpose. Back to unknown rather than silent: "Gate was
            // asked to let go of this machine" is not the same claim as "the
            // forwarder died", and the UI's fallback count is the honest
            // answer once it is true.
            Supervision::NotWanted => {
                self.forwarder_answering
                    .store(FORWARDER_UNKNOWN, Ordering::SeqCst);
            }
            Supervision::Running(port) => {
                if !was_answering {
                    eprintln!(
                        "gate proxy: the environment forwarder is answering again on \
                         127.0.0.1:{port}"
                    );
                }
                self.forwarder_answering
                    .store(FORWARDER_ANSWERING, Ordering::SeqCst);
                // Both repoints behind one re-check of the engine, because a
                // `disable` can land inside the ensure above: it withdraws the
                // variables on its way out, and putting them back afterwards
                // would leave `HTTPS_PROXY` set with routing off. The lock is
                // dropped before the export, which shells out.
                let still_hosted = {
                    let guard = self.engine.lock().expect("proxy engine mutex poisoned");
                    #[cfg(any(target_os = "macos", target_os = "windows"))]
                    if let Some(running) = guard.as_ref() {
                        running.set_pac_target(port);
                    }
                    guard.is_some()
                };
                if still_hosted
                    && crate::proxy::env_export_opted_in()
                    && self.exported_port.load(Ordering::SeqCst) != port
                {
                    match self.ops.enable_env(port) {
                        Ok(()) => self.exported_port.store(port, Ordering::SeqCst),
                        Err(e) => eprintln!(
                            "gate proxy: could not re-export proxy environment variables \
                             ({e}); they still name the forwarder's previous port"
                        ),
                    }
                }
            }
            Supervision::Failed(e) => {
                if was_answering {
                    eprintln!(
                        "gate proxy: the environment forwarder stopped answering and could \
                         not be restarted ({e}); browsers and tools that name it are going \
                         direct until it is back"
                    );
                }
                self.forwarder_answering
                    .store(FORWARDER_SILENT, Ordering::SeqCst);
            }
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};
    use std::sync::{Mutex as StdMutex, MutexGuard, OnceLock};

    use rcgen::{BasicConstraints, CertificateParams, DnType, IsCa, KeyPair, KeyUsagePurpose};

    use super::*;

    /// Everything the fake platform records and the tests steer. One mutex
    /// per fake; the tests themselves are serialized by [`TestHome`].
    #[derive(Default)]
    struct FakeState {
        /// Call log, coarse-grained: the tests assert ordering invariants
        /// (snapshot saved before the proxy is pointed, env reverted before
        /// the restore), not exact traces.
        calls: Vec<String>,
        /// The "persisted" snapshot file. `Some` while a snapshot is saved.
        persisted_snapshot: Option<String>,
        /// Make `load_snapshot` fail, modeling a torn file on disk.
        snapshot_unreadable: bool,
        /// What `engine_hosted_elsewhere` reports.
        hosted_elsewhere: Option<u16>,
        /// Make `point_system_proxy_at` fail, modeling a system-proxy write
        /// error after the engine is already up.
        point_fails: bool,
        /// The "persisted" engine port file.
        persisted_port: Option<u16>,
        /// Make `ensure_env_forwarder` fail, modeling a forwarder that will
        /// not start.
        forwarder_fails: bool,
        /// The port `ensure_env_forwarder` handed out, if it was asked.
        forwarder_port: Option<u16>,
        /// The port the next `ensure_env_forwarder` hands out; 0 means the
        /// usual 47_321. Set to model a forwarder that came back elsewhere.
        forwarder_binds: u16,
        /// Model `forwarder::stop` having run: the marker is gone.
        forwarder_stopped: bool,
        /// Model another caller already being inside an ensure.
        forwarder_busy: bool,
        /// The port `enable_env` was actually told to export.
        exported_port: Option<u16>,
        /// What `fronted_relay_port` reports: the relay port the forwarder
        /// holds. `relay_front_answers` is consumed first, one answer per ask,
        /// to model a forwarder whose claim changes between two questions.
        relay_front: Option<u16>,
        relay_front_answers: std::collections::VecDeque<Option<u16>>,
    }

    struct FakeOps(StdMutex<FakeState>);

    impl FakeOps {
        fn new() -> Self {
            Self(StdMutex::new(FakeState::default()))
        }

        fn with(state: FakeState) -> Self {
            Self(StdMutex::new(state))
        }

        fn record(&self, call: &str) {
            self.0.lock().unwrap().calls.push(call.to_string());
        }

        fn calls(&self) -> Vec<String> {
            self.0.lock().unwrap().calls.clone()
        }

        fn count(&self, call: &str) -> usize {
            self.calls().iter().filter(|c| c.as_str() == call).count()
        }

        fn index_of(&self, call: &str) -> usize {
            self.calls()
                .iter()
                .position(|c| c == call)
                .unwrap_or_else(|| panic!("{call} was never called: {:?}", self.calls()))
        }
    }

    /// A throwaway CA minted once per test binary: the engine parses real PEM
    /// material, and the manager must never care what is inside it.
    fn test_ca() -> &'static (String, String) {
        static CA: OnceLock<(String, String)> = OnceLock::new();
        CA.get_or_init(|| {
            let mut params = CertificateParams::new(Vec::<String>::new())
                .expect("building CA certificate params");
            params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
            params.key_usages = vec![
                KeyUsagePurpose::KeyCertSign,
                KeyUsagePurpose::CrlSign,
                KeyUsagePurpose::DigitalSignature,
            ];
            params
                .distinguished_name
                .push(DnType::CommonName, "Gate Connect Test CA");
            let key = KeyPair::generate().expect("generating CA key pair");
            let cert = params.self_signed(&key).expect("self-signing CA cert");
            (cert.pem(), key.serialize_pem())
        })
    }

    impl DesktopOps for FakeOps {
        type Snapshot = String;

        fn preflight_enable(&self) -> Result<()> {
            self.record("preflight");
            Ok(())
        }

        fn snapshot(&self) -> Result<String> {
            self.record("snapshot");
            Ok("user-proxy-state".to_string())
        }

        fn save_snapshot(&self, snapshot: &String) -> Result<()> {
            let mut s = self.0.lock().unwrap();
            s.calls.push("save_snapshot".to_string());
            s.persisted_snapshot = Some(snapshot.clone());
            Ok(())
        }

        fn load_snapshot(&self) -> Result<Option<String>> {
            let s = self.0.lock().unwrap();
            if s.snapshot_unreadable {
                anyhow::bail!("torn snapshot file");
            }
            Ok(s.persisted_snapshot.clone())
        }

        fn clear_snapshot(&self) -> Result<()> {
            let mut s = self.0.lock().unwrap();
            s.calls.push("clear_snapshot".to_string());
            s.persisted_snapshot = None;
            Ok(())
        }

        fn restore(&self, snapshot: &String) -> Result<()> {
            self.record(&format!("restore:{snapshot}"));
            Ok(())
        }

        fn force_off(&self) -> Result<()> {
            self.record("force_off");
            Ok(())
        }

        fn upstream_proxy(&self, _snapshot: &String) -> Option<String> {
            None
        }

        fn point_system_proxy_at(&self, _running: &engine::RunningEngine) -> Result<()> {
            self.record("point_system_proxy");
            if self.0.lock().unwrap().point_fails {
                anyhow::bail!("system proxy write refused");
            }
            Ok(())
        }

        fn persist_ports(&self, running: &engine::RunningEngine) {
            self.0.lock().unwrap().persisted_port = Some(running.port());
        }

        fn preferred_engine_port(&self) -> Option<u16> {
            self.0.lock().unwrap().persisted_port
        }

        fn preferred_pac_port(&self) -> Option<u16> {
            None
        }

        fn engine_pac_port(&self, _running: &engine::RunningEngine) -> Option<u16> {
            None
        }

        fn enable_env(&self, port: u16) -> Result<()> {
            let mut s = self.0.lock().unwrap();
            s.calls.push("enable_env".to_string());
            // Recorded, not discarded: the test that matters here is *which*
            // port was exported, and a fake that drops it passes just as
            // happily when production exports the engine's.
            s.exported_port = Some(port);
            Ok(())
        }

        fn disable_env(&self) -> Result<()> {
            self.record("disable_env");
            Ok(())
        }

        fn ensure_env_forwarder(&self) -> Result<u16> {
            self.record("ensure_env_forwarder");
            let mut s = self.0.lock().unwrap();
            if s.forwarder_fails {
                anyhow::bail!("forwarder refused to start");
            }
            let port = if s.forwarder_binds == 0 {
                47_321
            } else {
                s.forwarder_binds
            };
            s.forwarder_port = Some(port);
            Ok(port)
        }

        fn supervise_env_forwarder(&self) -> Supervision {
            {
                let s = self.0.lock().unwrap();
                if s.forwarder_busy {
                    return Supervision::Busy;
                }
                if s.forwarder_stopped {
                    return Supervision::NotWanted;
                }
            }
            match self.ensure_env_forwarder() {
                Ok(port) => Supervision::Running(port),
                Err(e) => Supervision::Failed(e),
            }
        }

        fn stop_env_forwarder(&self) {
            self.0
                .lock()
                .unwrap()
                .calls
                .push("stop_env_forwarder".into());
        }

        fn fronted_relay_port(&self, _wait: Duration) -> Option<u16> {
            self.record("fronted_relay_port");
            let mut s = self.0.lock().unwrap();
            let fallback = s.relay_front;
            s.relay_front_answers.pop_front().unwrap_or(fallback)
        }

        fn clear_stranded_loopback(&self) -> Result<Vec<String>> {
            self.record("clear_stranded");
            Ok(Vec::new())
        }

        fn engine_hosted_elsewhere(&self) -> Option<u16> {
            self.0.lock().unwrap().hosted_elsewhere
        }

        fn ca_load_or_create(&self) -> Result<CaMaterial> {
            let (cert_pem, key_pem) = test_ca().clone();
            Ok(CaMaterial { cert_pem, key_pem })
        }

        fn ca_ensure_trusted(&self) -> Result<()> {
            self.record("ensure_trusted");
            Ok(())
        }

        fn ca_ensure_trusted_system(&self) -> Result<()> {
            Ok(())
        }

        fn ca_untrust(&self) -> Result<()> {
            self.record("untrust");
            Ok(())
        }

        fn ca_untrust_system(&self) -> Result<()> {
            Ok(())
        }

        fn ca_is_trusted(&self) -> Result<bool> {
            Ok(true)
        }
    }

    /// Per-test hermetic home + secrets dir, serialized process-wide: the
    /// engine reads the account and domain config through the path seams,
    /// which are env vars, so tests that redirect them cannot overlap (see
    /// `crate::env::path_env_lock`). Restores the prior env and removes the
    /// dir on drop.
    struct TestHome {
        dir: PathBuf,
        prev_home: Option<OsString>,
        prev_secrets: Option<OsString>,
        _guard: MutexGuard<'static, ()>,
    }

    impl TestHome {
        fn set() -> Self {
            let guard = crate::env::path_env_lock();
            static N: AtomicUsize = AtomicUsize::new(0);
            let dir = std::env::temp_dir().join(format!(
                "gate-connect-manager-core-test-{}-{}",
                std::process::id(),
                N.fetch_add(1, AtomicOrdering::Relaxed)
            ));
            std::fs::create_dir_all(dir.join("secrets")).unwrap();
            let prev_home = std::env::var_os("GATE_CONNECT_TEST_HOME");
            let prev_secrets = std::env::var_os("GATE_CONNECT_TEST_SECRETS");
            std::env::set_var("GATE_CONNECT_TEST_HOME", &dir);
            std::env::set_var("GATE_CONNECT_TEST_SECRETS", dir.join("secrets"));
            account::save("https://gw.example.com", Some("sk-gw-testkey123")).unwrap();
            TestHome {
                dir,
                prev_home,
                prev_secrets,
                _guard: guard,
            }
        }
    }

    impl Drop for TestHome {
        fn drop(&mut self) {
            fn restore(key: &str, prev: &Option<OsString>) {
                match prev {
                    Some(v) => std::env::set_var(key, v),
                    None => std::env::remove_var(key),
                }
            }
            restore("GATE_CONNECT_TEST_HOME", &self.prev_home);
            restore("GATE_CONNECT_TEST_SECRETS", &self.prev_secrets);
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    /// The manager wants `&'static self` (watcher + crash fail-safe outlive
    /// the call); production has the singleton, tests leak one per test.
    fn leak(ops: FakeOps) -> &'static DesktopManager<FakeOps> {
        Box::leak(Box::new(DesktopManager::new(ops)))
    }

    /// Crash notifications seen so far, installing the observer on first use.
    ///
    /// One counter shared by every test that asserts on it, because the observer is
    /// a process-global `OnceLock` where the first set wins: a second test
    /// installing its own closure would silently get a counter that never
    /// moves, and would fail or pass depending on test order.
    fn notifications() -> usize {
        static NOTIFIED: AtomicUsize = AtomicUsize::new(0);
        crate::proxy::set_engine_crash_observer(|| {
            NOTIFIED.fetch_add(1, AtomicOrdering::SeqCst);
        });
        NOTIFIED.load(AtomicOrdering::SeqCst)
    }

    #[test]
    fn enable_snapshots_before_pointing_and_disable_restores_exactly() {
        let _home = TestHome::set();
        let mgr = leak(FakeOps::new());

        let state = mgr.enable().expect("enable should succeed");
        assert!(state.running);
        assert!(state.port.is_some());
        // The snapshot of the *user's* state must exist before the system
        // proxy is pointed at us - the whole revert story hangs on it.
        assert!(mgr.ops.index_of("save_snapshot") < mgr.ops.index_of("point_system_proxy"));
        // And the CA prompt comes after the promptless preflight.
        assert!(mgr.ops.index_of("preflight") < mgr.ops.index_of("ensure_trusted"));

        let state = mgr.disable().expect("disable should succeed");
        assert!(!state.running);
        // Env vars are the fail-hard channel: reverted before the PAC restore.
        assert!(mgr.ops.index_of("disable_env") < mgr.ops.index_of("restore:user-proxy-state"));
        // The exact snapshot came back, and nothing is left persisted.
        assert_eq!(mgr.ops.count("restore:user-proxy-state"), 1);
        assert!(mgr.ops.0.lock().unwrap().persisted_snapshot.is_none());
        assert_eq!(
            mgr.ops.count("force_off"),
            0,
            "restore path must not force off"
        );
        mgr.stop_dormant(); // a disable parks; release it so the port band is free for the next test
    }

    #[test]
    fn second_enable_is_idempotent() {
        let _home = TestHome::set();
        let mgr = leak(FakeOps::new());

        mgr.enable().expect("first enable");
        let state = mgr.enable().expect("second enable");
        assert!(state.running);
        // The second call returned status without redoing the sequence: one
        // snapshot, one pointing of the system proxy.
        assert_eq!(mgr.ops.count("snapshot"), 1);
        assert_eq!(mgr.ops.count("point_system_proxy"), 1);

        mgr.disable().expect("disable");
        mgr.stop_dormant(); // release the park for the next test
    }

    #[test]
    fn enable_refuses_when_hosted_by_another_process() {
        let _home = TestHome::set();
        let mgr = leak(FakeOps::with(FakeState {
            hosted_elsewhere: Some(47123),
            ..FakeState::default()
        }));

        let err = mgr.enable().expect_err("enable must refuse");
        assert!(
            err.to_string().contains("hosted by another process"),
            "refusal must name the cause: {err:#}"
        );
        // Refused before anything was touched: no snapshot, no CA prompt.
        assert_eq!(mgr.ops.count("snapshot"), 0);
        assert_eq!(mgr.ops.count("ensure_trusted"), 0);
    }

    /// Answer the relay identity challenge the way a second Gate Connect's
    /// parked relay would, so `enable` can tell it from a stranger.
    fn parked_relay_of_another_instance() -> crate::proxy::test_relay::TestRelay {
        let relay = crate::proxy::test_relay::TestRelay::with_interception(0, false);
        relay.persist_port();
        relay
    }

    /// A second process must not enable while another Gate Connect holds the
    /// ports, even when that one is *parked*: parking clears the system-proxy
    /// snapshot, so `engine_hosted_elsewhere` cannot see it. Without this the
    /// enable bound fallback ports and rewrote the persisted port files,
    /// repointing every tool config on the machine at addresses nothing names.
    #[test]
    fn enable_refuses_when_another_instances_relay_is_parked() {
        let _home = TestHome::set();
        let _relay = parked_relay_of_another_instance();
        let mgr = leak(FakeOps::new());

        let err = mgr.enable().expect_err("enable must refuse");
        assert!(
            err.to_string().contains("another Gate Connect process"),
            "refusal must name the cause: {err:#}"
        );
        // Nothing was bound and nothing was persisted, so the tool configs that
        // name the running instance's ports still name something live.
        assert_eq!(mgr.ops.count("persist_ports"), 0);
    }

    /// A stranger on the relay port is not another Gate, and must not keep Gate
    /// from starting: that case recovers on its own, because the fallback port
    /// is persisted and the reconcile passes repair the configs that drift.
    #[test]
    fn enable_proceeds_when_the_relay_port_holds_a_stranger() {
        let _home = TestHome::set();
        let squatter = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = squatter.local_addr().unwrap().port();
        std::thread::spawn(move || {
            for stream in squatter.incoming() {
                drop(stream);
            }
        });
        let dir = crate::env::app_support_dir().unwrap().join("proxy");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("relay-port"), port.to_string()).unwrap();
        let mgr = leak(FakeOps::new());

        mgr.enable().expect("a stranger must not block enabling");
        mgr.disable_quiet().expect("release for the next test");
    }

    #[test]
    fn untrust_is_refused_while_running_and_allowed_after_disable() {
        let _home = TestHome::set();
        let mgr = leak(FakeOps::new());

        mgr.enable().expect("enable");
        let err = mgr
            .untrust_ca()
            .expect_err("untrust must refuse while running");
        assert!(err.to_string().contains("turn the proxy off"));
        assert_eq!(mgr.ops.count("untrust"), 0);

        mgr.disable().expect("disable");
        mgr.untrust_ca().expect("untrust after disable");
        assert_eq!(mgr.ops.count("untrust"), 1);
    }

    #[test]
    fn crash_handler_reverts_clears_and_notifies() {
        let _home = TestHome::set();
        let mgr = leak(FakeOps::new());

        mgr.enable().expect("enable");
        let before = notifications();
        mgr.handle_engine_crash();

        // Traffic made safe: env reverted, exact snapshot restored, nothing
        // left persisted - and the shell was told, after the fact.
        assert_eq!(mgr.ops.count("restore:user-proxy-state"), 1);
        assert!(mgr.ops.0.lock().unwrap().persisted_snapshot.is_none());
        assert!(notifications() > before);
        let state = mgr.status().expect("status");
        assert!(!state.running, "the dead handle must be dropped");
    }

    #[test]
    fn failed_system_proxy_write_rolls_back_the_enable() {
        let _home = TestHome::set();
        let mgr = leak(FakeOps::with(FakeState {
            point_fails: true,
            ..FakeState::default()
        }));

        let err = mgr.enable().expect_err("enable must fail");
        assert!(err.to_string().contains("enabling system proxy"));
        // The engine was stopped and the snapshot cleared: nothing persisted
        // says routing is on, so a later reconcile has nothing to undo.
        assert!(mgr.ops.0.lock().unwrap().persisted_snapshot.is_none());
        let state = mgr.status().expect("status");
        assert!(!state.running);
    }

    #[test]
    fn unreadable_snapshot_on_disable_forces_the_proxy_off() {
        let _home = TestHome::set();
        let mgr = leak(FakeOps::new());

        mgr.enable().expect("enable");
        mgr.ops.0.lock().unwrap().snapshot_unreadable = true;
        mgr.disable().expect("disable must still succeed");
        // Fail-open would strand HTTPS at the dead engine port; the contract
        // is force-off when the exact restore is impossible.
        assert_eq!(mgr.ops.count("force_off"), 1);
        mgr.stop_dormant(); // a disable parks; release it so the port band is free for the next test
    }

    #[test]
    fn engine_port_persists_across_an_enable_cycle() {
        let _home = TestHome::set();
        let mgr = leak(FakeOps::new());

        let first = mgr.enable().expect("first enable").port.expect("port");
        mgr.disable().expect("disable");
        let second = mgr.enable().expect("second enable").port.expect("port");
        // Clients that resolved the proxy at their own launch keep dialing the
        // old port; a restart must come back on the same address.
        assert_eq!(first, second, "the persisted port must be rebound");
        mgr.disable().expect("disable");
        mgr.stop_dormant(); // release the park for the next test
    }

    #[test]
    fn reconcile_restores_a_leftover_snapshot_then_sweeps() {
        let _home = TestHome::set();
        let mgr = leak(FakeOps::with(FakeState {
            persisted_snapshot: Some("user-proxy-state".to_string()),
            ..FakeState::default()
        }));

        mgr.reconcile_on_startup().expect("reconcile");
        // An unclean prior session: the exact state comes back, the snapshot
        // is consumed, and the stranded-loopback sweep runs regardless.
        assert_eq!(mgr.ops.count("restore:user-proxy-state"), 1);
        assert!(mgr.ops.0.lock().unwrap().persisted_snapshot.is_none());
        assert!(mgr.ops.index_of("disable_env") < mgr.ops.index_of("restore:user-proxy-state"));
        assert_eq!(mgr.ops.count("clear_stranded"), 1);

        // A clean prior session (no snapshot) still sweeps - the sweep is the
        // layer that repairs what the snapshot cannot (it was lost).
        mgr.reconcile_on_startup().expect("reconcile again");
        assert_eq!(mgr.ops.count("clear_stranded"), 2);
        assert_eq!(mgr.ops.count("force_off"), 0);
    }

    #[test]
    fn reconcile_forces_off_when_the_snapshot_is_unreadable() {
        let _home = TestHome::set();
        let mgr = leak(FakeOps::with(FakeState {
            persisted_snapshot: Some("user-proxy-state".to_string()),
            snapshot_unreadable: true,
            ..FakeState::default()
        }));

        mgr.reconcile_on_startup().expect("reconcile");
        // Unreadable still means unclean: fail closed to off, never leave
        // HTTPS routed at a port nothing listens on.
        assert_eq!(mgr.ops.count("force_off"), 1);
        assert!(mgr.ops.0.lock().unwrap().persisted_snapshot.is_none());
    }

    /// The remaining surface, exercised once so its wiring can't silently rot:
    /// these paths are thin (lock, delegate, status) and their platform side
    /// is covered by the OS wiring, but nothing else on a non-desktop test
    /// build would even call them.
    #[test]
    fn thin_surface_smoke() {
        let _home = TestHome::set();
        let mgr = leak(FakeOps::new());

        mgr.set_detached(true); // no-op by contract
        assert!(!mgr.list_domains().expect("domains").is_empty());

        mgr.trust_ca().expect("trust");
        mgr.trust_ca_system().expect("system trust");

        mgr.enable().expect("enable");
        // Live updates against a running engine: push-only, must not error
        // or take the engine down.
        mgr.set_domain("anthropic", true).expect("set_domain");
        mgr.refresh_api_key("sk-gw-rotated");
        mgr.refresh_token("fresh-token");
        mgr.refresh_org("org-uuid-2");
        mgr.refresh_cf_clearance("cf-clearance-cookie");
        assert!(mgr.status().expect("status").running);

        mgr.shutdown_engine().expect("shutdown");
        assert!(!mgr.status().expect("status").running);

        mgr.enable().expect("re-enable");
        mgr.disable_quiet().expect("quiet disable");
        mgr.untrust_ca_system()
            .expect("system untrust after disable");
    }

    /// Behind a forwarder holding the public relay port, the engine's relay
    /// binds a port of its own and says so in `relay-engine-port`. The public
    /// port file - the one every relay config bakes - is not touched: writing
    /// the engine's port there would take every tool off the listener that
    /// outlives this process.
    #[test]
    fn the_relay_binds_behind_a_forwarder_that_holds_the_public_port() {
        let _home = TestHome::set();
        // The forwarder's listener on the public port, which the engine must
        // leave alone.
        let public = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let public_port = public.local_addr().unwrap().port();
        crate::proxy::relay::save_persisted_port(public_port).unwrap();
        let mgr = leak(FakeOps::with(FakeState {
            relay_front: Some(public_port),
            ..FakeState::default()
        }));

        mgr.enable().expect("enable");

        let engine_relay = crate::proxy::relay::load_engine_port().expect("engine relay recorded");
        assert_ne!(engine_relay, public_port);
        assert_eq!(
            crate::proxy::relay::load_persisted_port(),
            Some(public_port),
            "the port configs name must stay the forwarder's"
        );
        assert_eq!(
            mgr.ops.count("fronted_relay_port"),
            1,
            "a yes is not asked again: a failed second probe would repoint every config"
        );
        assert!(
            mgr.ops.index_of("fronted_relay_port") > mgr.ops.index_of("ensure_env_forwarder"),
            "fronting is only asked once a forwarder is known to be up"
        );
        mgr.shutdown_engine().expect("shutdown");
        drop(public);
    }

    /// The forwarder takes the public port between the enable's question and
    /// the engine's bind. The engine lands on a fallback port, and the second
    /// question sends that to `relay-engine-port` rather than into the file
    /// every relay config names.
    #[test]
    fn a_claim_landing_between_question_and_bind_keeps_the_configs_on_the_forwarder() {
        let _home = TestHome::set();
        let public = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let public_port = public.local_addr().unwrap().port();
        crate::proxy::relay::save_persisted_port(public_port).unwrap();
        let mgr = leak(FakeOps::with(FakeState {
            relay_front_answers: [None, Some(public_port)].into(),
            ..FakeState::default()
        }));

        mgr.enable().expect("enable");

        assert_eq!(mgr.ops.count("fronted_relay_port"), 2);
        assert_eq!(
            crate::proxy::relay::load_persisted_port(),
            Some(public_port)
        );
        let engine_relay = crate::proxy::relay::load_engine_port().expect("recorded behind");
        assert_ne!(engine_relay, public_port);
        mgr.shutdown_engine().expect("shutdown");
        drop(public);
    }

    /// Behind the forwarder, an engine relay of another Gate on the engine's
    /// port is refused as another Gate - found on the engine-only path.
    #[test]
    fn another_gate_behind_the_forwarder_is_refused() {
        let _home = TestHome::set();
        let other = crate::proxy::test_relay::TestRelay::start(0);
        let public = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let public_port = public.local_addr().unwrap().port();
        crate::proxy::relay::save_persisted_port(public_port).unwrap();
        crate::proxy::relay::save_engine_port(other.port()).unwrap();
        let mgr = leak(FakeOps::with(FakeState {
            relay_front: Some(public_port),
            ..FakeState::default()
        }));

        let err = mgr
            .enable()
            .expect_err("another Gate holds the engine's relay port");
        assert!(
            format!("{err:#}").contains("another Gate Connect"),
            "{err:#}"
        );
        drop(public);
    }

    /// Without a forwarder on it, the engine keeps the public relay port, as it
    /// always has.
    #[test]
    fn the_relay_keeps_the_public_port_when_nothing_fronts_it() {
        let _home = TestHome::set();
        let mgr = leak(FakeOps::new());

        mgr.enable().expect("enable");

        assert!(crate::proxy::relay::load_persisted_port().is_some());
        assert_eq!(crate::proxy::relay::load_engine_port(), None);
        mgr.shutdown_engine().expect("shutdown");
    }

    /// The exported variables must name the forwarder, not the engine. They
    /// outlive every process that reads them, so the address they carry has to
    /// be one that still answers after the engine goes away.
    #[test]
    fn the_exported_variables_name_the_forwarder() {
        let _home = TestHome::set();
        let mgr = leak(FakeOps::new());

        let state = mgr.enable().expect("enable");
        let engine_port = state.port.expect("port");

        assert_eq!(mgr.ops.count("ensure_env_forwarder"), 1);
        let exported = mgr.ops.0.lock().unwrap().exported_port;
        assert_eq!(
            exported,
            Some(47_321),
            "the forwarder's port, not any other"
        );
        assert_ne!(
            exported,
            Some(engine_port),
            "exporting the engine's own port is the bug"
        );
        assert!(mgr.ops.index_of("ensure_env_forwarder") < mgr.ops.index_of("enable_env"));
        // And before the system proxy is pointed at the PAC: browsers fetch it
        // on that change and cache what they get, so the first body served has
        // to already name the forwarder.
        assert!(mgr.ops.index_of("ensure_env_forwarder") < mgr.ops.index_of("point_system_proxy"));

        mgr.disable().expect("disable");
        mgr.stop_dormant(); // release the park for the next test
    }

    /// The forwarder is not the export's: the PAC names it too, so it has to
    /// exist whenever routing is on, whether or not the user wants the
    /// machine-wide variables.
    #[test]
    fn the_forwarder_is_ensured_even_when_the_export_is_declined() {
        let _home = TestHome::set();
        crate::proxy::set_env_export_opted_in(false).expect("decline the export");
        let mgr = leak(FakeOps::new());

        mgr.enable().expect("enable");
        assert_eq!(mgr.ops.count("ensure_env_forwarder"), 1);
        assert_eq!(mgr.ops.count("enable_env"), 0, "declined means declined");

        mgr.disable().expect("disable");
        mgr.stop_dormant();
    }

    /// Fetch the PAC the way WinINET does: a plain GET on the PAC listener.
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    fn fetch_pac(pac_port: u16) -> String {
        use std::io::{Read, Write};
        let mut s = std::net::TcpStream::connect(("127.0.0.1", pac_port)).expect("PAC listener");
        s.write_all(
            format!("GET /proxy.pac HTTP/1.1\r\nHost: 127.0.0.1:{pac_port}\r\n\r\n").as_bytes(),
        )
        .unwrap();
        let mut body = String::new();
        s.read_to_string(&mut body).unwrap();
        body
    }

    /// The body a browser actually fetches follows `set_pac_target`: the
    /// watch receiver is read per request, not captured at start.
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    #[test]
    fn the_served_pac_follows_the_target() {
        let _home = TestHome::set();
        let mgr = leak(FakeOps::new());

        mgr.enable().expect("enable");
        // From the engine, not from `status`: the fake platform reports no
        // PAC port through the seam, and it is the listener that matters here.
        let pac_port = mgr
            .engine
            .lock()
            .unwrap()
            .as_ref()
            .expect("engine hosted")
            .pac_port();
        assert!(
            fetch_pac(pac_port).contains("PROXY 127.0.0.1:47321; DIRECT"),
            "the first body served names the forwarder"
        );
        mgr.engine
            .lock()
            .unwrap()
            .as_ref()
            .expect("engine hosted")
            .set_pac_target(47_999);
        assert!(fetch_pac(pac_port).contains("PROXY 127.0.0.1:47999; DIRECT"));

        mgr.disable().expect("disable");
        mgr.stop_dormant();
    }

    /// The tick retires under the lock acquisition that sees the engine gone,
    /// exactly like the domain watcher, so a disable/enable flip cannot leave
    /// a routing engine with two watchers or none.
    #[test]
    fn the_forwarder_tick_retires_when_the_engine_is_gone() {
        let _home = TestHome::set();
        let mgr = leak(FakeOps::new());

        mgr.enable().expect("enable");
        assert!(
            mgr.forwarder_tick(),
            "keeps running while the engine is hosted"
        );
        mgr.disable().expect("disable");
        assert!(!mgr.forwarder_tick(), "retires once the engine is parked");
        assert!(!mgr.forwarder_watcher_alive.load(Ordering::SeqCst));
        // The thread `enable` spawned is still in its first sleep, so the flag
        // this just cleared no longer means "exactly one watcher is alive" for
        // this manager. Harmless while the test ends here; a re-enable added
        // below this line would start a second watcher.
        mgr.stop_dormant();
    }

    /// A pass that finds another caller already inside an ensure does nothing
    /// and says nothing. Reporting health from here would be a guess about a
    /// state that is mid-change, and re-running the ensure is what the lock
    /// exists to prevent.
    #[test]
    fn the_forwarder_tick_stands_aside_for_an_ensure_already_running() {
        let _home = TestHome::set();
        let mgr = leak(FakeOps::new());

        mgr.enable().expect("enable");
        mgr.ops.0.lock().unwrap().forwarder_busy = true;
        assert!(mgr.forwarder_tick(), "still watching");
        assert_eq!(
            mgr.ops.count("ensure_env_forwarder"),
            1,
            "only the enable's; the pass declined rather than queueing"
        );
        assert_eq!(
            mgr.status().unwrap().forwarder_answering,
            Some(true),
            "and left the last known state alone"
        );

        mgr.disable().expect("disable");
        mgr.stop_dormant();
    }

    /// A forwarder that came back on a different port reaches everything
    /// that named the old one: the PAC and the exported variables here, the
    /// tool configs through the shell's reconcile pass.
    #[test]
    fn the_forwarder_tick_repoints_the_pac_and_the_variables() {
        let _home = TestHome::set();
        let mgr = leak(FakeOps::new());

        let state = mgr.enable().expect("enable");
        assert_eq!(state.forwarder_answering, Some(true));
        assert_eq!(mgr.ops.count("enable_env"), 1);

        // Same port: nothing to redo.
        assert!(mgr.forwarder_tick());
        assert_eq!(
            mgr.ops.count("enable_env"),
            1,
            "an unchanged port is not re-exported"
        );

        // Came back elsewhere.
        mgr.ops.0.lock().unwrap().forwarder_binds = 47_322;
        assert!(mgr.forwarder_tick());
        assert_eq!(
            mgr.ops.count("enable_env"),
            2,
            "a moved port is re-exported"
        );
        assert_eq!(mgr.ops.0.lock().unwrap().exported_port, Some(47_322));
        #[cfg(any(target_os = "macos", target_os = "windows"))]
        {
            let target = mgr
                .engine
                .lock()
                .unwrap()
                .as_ref()
                .expect("engine hosted")
                .pac_target();
            assert_eq!(target, 47_322, "and the PAC follows it");
        }

        mgr.disable().expect("disable");
        mgr.stop_dormant();
    }

    /// The one thing the tray could not see: routing on, forwarder gone. The
    /// tick records it for `status`, and clears it when the forwarder is back.
    #[test]
    fn the_forwarder_tick_reports_a_forwarder_that_will_not_come_back() {
        let _home = TestHome::set();
        let mgr = leak(FakeOps::new());

        mgr.enable().expect("enable");
        mgr.ops.0.lock().unwrap().forwarder_fails = true;
        assert!(
            mgr.forwarder_tick(),
            "a dead forwarder does not retire the watcher"
        );
        assert_eq!(mgr.status().unwrap().forwarder_answering, Some(false));

        mgr.ops.0.lock().unwrap().forwarder_fails = false;
        assert!(mgr.forwarder_tick());
        assert_eq!(mgr.status().unwrap().forwarder_answering, Some(true));

        mgr.disable().expect("disable");
        assert_eq!(
            mgr.status().unwrap().forwarder_answering,
            None,
            "nothing to say about a forwarder when this process is not routing"
        );
        mgr.stop_dormant();
    }

    /// `forwarder::stop` is an instruction, and the quit-with-disconnect path
    /// gives it while the engine is still hosted. A tick landing in that gap
    /// must not write the marker back and respawn what was just retired.
    #[test]
    fn the_forwarder_tick_leaves_a_stopped_forwarder_alone() {
        let _home = TestHome::set();
        let mgr = leak(FakeOps::new());

        mgr.enable().expect("enable");
        mgr.ops.0.lock().unwrap().forwarder_stopped = true;
        assert!(
            mgr.forwarder_tick(),
            "still watching, in case routing continues"
        );
        assert_eq!(
            mgr.ops.count("ensure_env_forwarder"),
            1,
            "only the enable's; the tick did not resurrect it"
        );
        // And stops claiming one is answering. The quit that retires the
        // forwarder can leave the user back on Home if the quit itself fails,
        // and "8 of 8 routing" over a forwarder nobody intends to restart is
        // the report this field exists to prevent.
        assert_eq!(mgr.status().unwrap().forwarder_answering, None);

        mgr.disable().expect("disable");
        mgr.stop_dormant();
    }

    /// A forwarder that will not start costs the fail-open property and
    /// nothing else: routing still comes up, exporting the engine port exactly
    /// as it did before there was a forwarder.
    #[test]
    fn a_forwarder_that_will_not_start_does_not_fail_the_enable() {
        let _home = TestHome::set();
        let mgr = leak(FakeOps::with(FakeState {
            forwarder_fails: true,
            ..FakeState::default()
        }));

        let state = mgr.enable().expect("enable must still succeed");
        assert!(state.running);
        assert_eq!(mgr.ops.count("enable_env"), 1, "the export still happens");
        assert_eq!(
            mgr.ops.0.lock().unwrap().exported_port,
            state.port,
            "and falls back to the engine's own port, which is what shipped \
             before there was a forwarder"
        );
        assert_eq!(
            state.forwarder_answering,
            Some(false),
            "and says so, because every address it just wrote falls back to \
             direct the moment the engine goes"
        );
        #[cfg(any(target_os = "macos", target_os = "windows"))]
        {
            let target = mgr
                .engine
                .lock()
                .unwrap()
                .as_ref()
                .expect("engine hosted")
                .pac_target();
            assert_eq!(
                Some(target),
                state.port,
                "the PAC falls back to the engine's own port for the same reason"
            );
        }

        mgr.disable().expect("disable");
        mgr.stop_dormant(); // release the park for the next test
    }

    /// The PAC must name the forwarder too. A browser caches the script it
    /// fetched, so with the engine's own port in it every Gate host fails
    /// closed the moment the engine is gone - the failure the PAC channel was
    /// supposed to be immune to.
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    #[test]
    fn the_pac_names_the_forwarder() {
        let _home = TestHome::set();
        let mgr = leak(FakeOps::new());

        let state = mgr.enable().expect("enable");
        let engine_port = state.port.expect("port");
        let target = mgr
            .engine
            .lock()
            .unwrap()
            .as_ref()
            .expect("engine hosted")
            .pac_target();
        assert_eq!(target, 47_321, "the forwarder's port, not any other");
        assert_ne!(
            target, engine_port,
            "naming the engine's own port is the bug"
        );

        mgr.disable().expect("disable");
        mgr.stop_dormant(); // release the park for the next test
    }

    /// A plain disable must leave the forwarder alone - it is exactly then
    /// that the processes holding our variables still need it. Untrusting the
    /// CA is the explicit "let go of this machine" action, and does stop it.
    #[test]
    fn disable_leaves_the_forwarder_running_and_untrust_stops_it() {
        let _home = TestHome::set();
        let mgr = leak(FakeOps::new());

        mgr.enable().expect("enable");
        mgr.disable().expect("disable");
        assert_eq!(
            mgr.ops.count("stop_env_forwarder"),
            0,
            "stopping it on disable would strand the tools it exists to protect"
        );

        mgr.untrust_ca().expect("untrust after disable");
        assert_eq!(mgr.ops.count("stop_env_forwarder"), 1);
        mgr.stop_dormant(); // release the park for the next test
    }

    #[test]
    fn status_reports_an_engine_hosted_by_another_process() {
        let _home = TestHome::set();
        let mgr = leak(FakeOps::with(FakeState {
            hosted_elsewhere: Some(47150),
            ..FakeState::default()
        }));

        // No handle in this process, but the machine is routed: status must
        // say running, on the other host's port - "stopped" would be the
        // wrong answer, not a partial one.
        let state = mgr.status().expect("status");
        assert!(state.running);
        assert_eq!(state.port, Some(47150));
    }

    /// Whether anything is still accepting on a loopback port - the same
    /// question `engine_hosted_elsewhere` asks, and the one a tool holding a
    /// stale `HTTPS_PROXY` asks by dialing it.
    fn answering(port: u16) -> bool {
        std::net::TcpStream::connect_timeout(
            &std::net::SocketAddr::from(([127, 0, 0, 1], port)),
            Duration::from_millis(250),
        )
        .is_ok()
    }

    #[test]
    fn disable_parks_the_engine_so_stale_clients_still_reach_their_provider() {
        let _home = TestHome::set();
        let mgr = leak(FakeOps::new());

        let port = mgr.enable().expect("enable").port.expect("port");
        assert!(answering(port), "the engine should accept while routing");

        let state = mgr.disable().expect("disable");

        // Routing is off by every user-visible measure: the switch reads off,
        // the system proxy is back, and nothing says Gate is serving.
        assert!(!state.running);
        assert_eq!(mgr.ops.count("restore:user-proxy-state"), 1);
        assert_eq!(mgr.ops.count("disable_env"), 1);
        assert!(mgr.ops.0.lock().unwrap().persisted_snapshot.is_none());
        assert!(!mgr.hosts_live_engine(), "a parked engine is not routing");

        // But the port still accepts. `launchctl unsetenv` cannot reach a
        // process that is already running, so every shell, editor and CLI
        // started before the toggle keeps dialing this address - including
        // tools that were never switched on, and software Gate does not
        // manage. Releasing it here is what turned "routing off" into "no
        // provider is reachable".
        assert!(
            answering(port),
            "disable must leave the exported port answering"
        );

        // And it answers as a plain proxy. Clearing the domains is the visible
        // half; `set_intercept(false)` is the half that matters, because
        // `route_rules` force-enables Claude Code's entry exactly when the live
        // set is empty - an engine parked by clearing domains alone would still
        // decrypt and bill a `claude` session that predates the toggle.
        // `a_parked_engine_routes_nothing_even_with_the_selector` in `engine`
        // pins that; here we check the manager asked for both.
        assert_eq!(
            mgr.dormant
                .lock()
                .unwrap()
                .as_ref()
                .map(|e| e.intercepting()),
            Some(0),
            "a parked engine must claim no domains"
        );

        mgr.disable_quiet().expect("release for the next test");
    }

    #[test]
    fn exit_releases_the_ports_and_a_gateway_switch_releases_a_park() {
        let _home = TestHome::set();
        let mgr = leak(FakeOps::new());

        // App exit: the listeners live in this process, so there is nothing to
        // park - they go when it does.
        let port = mgr.enable().expect("enable").port.expect("port");
        mgr.disable_quiet().expect("exit teardown");
        assert!(!answering(port), "exit must release the port");

        // A gateway switch reaches through a park: the engine holds the old
        // environment's base URL, which `engine::start` never updates.
        let port = mgr.enable().expect("re-enable").port.expect("port");
        mgr.disable().expect("disable parks it");
        assert!(answering(port), "the park is what disable does");
        mgr.shutdown_engine().expect("gateway switch");
        assert!(
            !answering(port),
            "a gateway switch must not leave the old account's engine bound"
        );
    }

    #[test]
    fn untrusting_the_ca_releases_a_parked_engine() {
        let _home = TestHome::set();
        let mgr = leak(FakeOps::new());

        let port = mgr.enable().expect("enable").port.expect("port");
        mgr.disable().expect("disable");
        mgr.untrust_ca().expect("untrust after disable");

        assert_eq!(mgr.ops.count("untrust"), 1);
        // Untrusting is the explicit "let go of this machine" action - it is
        // what Reset runs - so Gate must not still be holding a port after it.
        assert!(!answering(port), "untrust must release the parked port");
    }

    #[test]
    fn the_crash_fail_safe_stands_down_while_parked() {
        let _home = TestHome::set();
        let mgr = leak(FakeOps::new());

        let port = mgr.enable().expect("enable").port.expect("port");
        mgr.disable().expect("disable");
        let before = notifications();

        // The parked engine carries the same crash callback the routing one
        // did, so its death lands in the same handler. Calling it directly is
        // the only way to drive that branch from a test: `on_unexpected_exit`
        // runs on the engine's own thread, so a genuinely dying engine could
        // not be observed as finished from here anyway.
        mgr.handle_engine_crash();

        // Nothing to revert, and reverting anyway would be destructive: the
        // snapshot is already cleared, so the force-off fallback would fire
        // and switch off a system proxy the *user* owns - the one the disable
        // had just faithfully restored.
        assert_eq!(
            mgr.ops.count("force_off"),
            0,
            "must not force the proxy off"
        );
        assert_eq!(
            mgr.ops.count("restore:user-proxy-state"),
            1,
            "the disable's restore is the only one"
        );
        assert_eq!(
            notifications(),
            before,
            "routing is already off; a crash banner would be a lie"
        );
        // Still live, so it is still doing its job for stale clients.
        assert!(answering(port));

        mgr.disable_quiet().expect("release for the next test");
    }
}
