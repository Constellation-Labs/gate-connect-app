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

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::Duration;

use anyhow::{Context, Result};

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

pub struct DesktopManager<O: DesktopOps> {
    ops: O,
    engine: Mutex<Option<engine::RunningEngine>>,
    /// Whether a domain watcher is already running, so repeated enables don't
    /// stack them. Per-instance (not a process static) so tests can build
    /// managers side by side.
    watcher_alive: AtomicBool,
}

impl<O: DesktopOps> DesktopManager<O> {
    pub fn new(ops: O) -> Self {
        Self {
            ops,
            engine: Mutex::new(None),
            watcher_alive: AtomicBool::new(false),
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
            env_export_opted_in: crate::proxy::env_export_opted_in(),
            env_export_separable: crate::proxy::env_export_is_separable(),
            relay_base_url: crate::proxy::relay_base_url(),
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
                 over from it and record Gate's own settings as the ones to restore. Quit \
                 that process, or run `gate-connect proxy disable` first."
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
                // Who pays: `Payg` drops the upstream hint and the tool's own
                // credential on the rewrite path. A later switch pushes an
                // update via `refresh_mode`.
                billing_mode: account.billing_mode,
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
                // http://127.0.0.1:<port>) stay valid across restarts.
                preferred_relay_port: crate::proxy::relay::load_persisted_port(),
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
        // configs stay valid (best-effort).
        let _ = crate::proxy::relay::save_persisted_port(running.relay_port());

        // Point the system proxy at the engine's loopback PAC. Promptless.
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
        if crate::proxy::env_export_opted_in() {
            if let Err(e) = self.ops.enable_env(running.port()) {
                eprintln!(
                    "gate proxy: could not export proxy environment variables ({e}); GUI apps \
                     still route through Gate, but CLI tools that read HTTPS_PROXY will not"
                );
            }
        }

        *guard = Some(running);
        // The engine now has whatever the config said at startup. Keep it in
        // step with writes made by *other* processes for as long as it runs.
        self.spawn_domain_watcher();

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

    /// Stop the engine and restore the prior system proxy. Promptless and
    /// unconditional - the revert happens first and never depends on admin,
    /// so it can't be canceled and strand traffic. The CA is left trusted.
    pub fn disable(&self) -> Result<ProxyState> {
        self.disable_inner()?;

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
    pub fn disable_quiet(&self) -> Result<()> {
        self.disable_inner()
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
        self.disable_inner()
    }

    /// Shared body of [`disable`](Self::disable) /
    /// [`disable_quiet`](Self::disable_quiet): revert the system proxy and stop
    /// the engine, without computing status.
    fn disable_inner(&self) -> Result<()> {
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
        // Revert first, stop second - a live engine behind a reverted proxy is
        // harmless, the reverse strands HTTPS at a dead port. But the engine has
        // already been taken out of the guard, so returning early on a failed
        // revert would drop it, and `RunningEngine::drop` does not join: the
        // listeners would stay up with no handle left to stop them, while our
        // own state says nothing is running. Keep the result, stop, then report.
        let reverted = match snapshot {
            Some(snapshot) => self.ops.restore(&snapshot),
            None => self.ops.force_off(),
        };
        if let Some(running) = running {
            running.stop();
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

    /// Push a changed billing mode into the running engine (and the relay it
    /// hosts), if any. Used when the user switches BYOK/PAYG so the new request
    /// shape reaches in-flight routing without a restart. Reads the mode from
    /// disk rather than taking it as an argument, so a live engine can never be
    /// routing under a mode the account does not hold.
    pub fn refresh_mode(&self) {
        if let Some(running) = self
            .engine
            .lock()
            .expect("proxy engine mutex poisoned")
            .as_ref()
        {
            running.update_mode(crate::account::billing_mode_for_injection());
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
        self.refuse_untrust_while_running()?;
        self.ops.ca_untrust()?;
        self.status()
    }

    /// Remove a machine-wide trust install with no prompt. The counterpart of
    /// [`trust_ca_system`](Self::trust_ca_system), and refuses while running
    /// for the same reason [`untrust_ca`](Self::untrust_ca) does.
    pub fn untrust_ca_system(&self) -> Result<ProxyState> {
        self.refuse_untrust_while_running()?;
        self.ops.ca_untrust_system()?;
        self.status()
    }

    fn refuse_untrust_while_running(&self) -> Result<()> {
        if self
            .engine
            .lock()
            .expect("proxy engine mutex poisoned")
            .is_some()
        {
            anyhow::bail!("turn the proxy off before untrusting the CA");
        }
        Ok(())
    }

    /// Fail-safe invoked from the engine thread if the engine exits without a
    /// deliberate stop. Drops the dead handle and reverts the system proxy so
    /// HTTPS isn't stranded. Promptless and best-effort.
    pub(crate) fn handle_engine_crash(&self) {
        eprintln!("gate proxy engine exited unexpectedly; reverting system proxy");
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

        fn enable_env(&self, _port: u16) -> Result<()> {
            self.record("enable_env");
            Ok(())
        }

        fn disable_env(&self) -> Result<()> {
            self.record("disable_env");
            Ok(())
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

        // The observer is a process-global OnceLock; first set wins, which is
        // fine here - this is the only test that asserts on it, and it only
        // asserts the count *increased* across the crash.
        static NOTIFIED: AtomicUsize = AtomicUsize::new(0);
        crate::proxy::set_engine_crash_observer(|| {
            NOTIFIED.fetch_add(1, AtomicOrdering::SeqCst);
        });

        mgr.enable().expect("enable");
        let before = NOTIFIED.load(AtomicOrdering::SeqCst);
        mgr.handle_engine_crash();

        // Traffic made safe: env reverted, exact snapshot restored, nothing
        // left persisted - and the shell was told, after the fact.
        assert_eq!(mgr.ops.count("restore:user-proxy-state"), 1);
        assert!(mgr.ops.0.lock().unwrap().persisted_snapshot.is_none());
        assert!(NOTIFIED.load(AtomicOrdering::SeqCst) > before);
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
        mgr.refresh_mode();
        assert!(mgr.status().expect("status").running);

        mgr.shutdown_engine().expect("shutdown");
        assert!(!mgr.status().expect("status").running);

        mgr.enable().expect("re-enable");
        mgr.disable_quiet().expect("quiet disable");
        mgr.untrust_ca_system()
            .expect("system untrust after disable");
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
}
