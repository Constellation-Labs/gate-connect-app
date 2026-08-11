//! Orchestrates the proxy subsystem on Windows: composes the CA, system-proxy,
//! engine, and domain-config modules behind a process-global singleton the
//! Tauri commands call. Windows counterpart of the macOS [`super::manager`].
//!
//! Privilege model: changing the WinINET proxy is a per-user (`HKCU`) registry
//! write, so enable/disable/restore/reconcile run promptless. The only step
//! that needs confirmation is trusting the CA (Windows' native root-store
//! dialog), which happens once on enable. Critically, the proxy revert never
//! depends on that dialog - so it can't be cancelled and leave HTTPS routed at
//! a dead port. The CA is left trusted across disable so re-enabling is
//! promptless; removing it is a separate explicit action
//! ([`ProxyManager::untrust_ca`]).
//!
//! Unlike macOS - where each network service carries its own proxy slots -
//! Windows has a single per-user WinINET proxy, so there is no service
//! enumeration here; enable/disable act on that one global setting.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use anyhow::{Context, Result};

use super::{ca, config, engine, system_proxy, ProxyDomain, ProxyState};
use crate::account;

pub struct ProxyManager {
    engine: Mutex<Option<engine::RunningEngine>>,
}

static MANAGER: OnceLock<ProxyManager> = OnceLock::new();

/// Whether a domain watcher is already running, so repeated enables don't
/// stack them.
static WATCHER_ALIVE: AtomicBool = AtomicBool::new(false);

/// How often the watcher stats the domains file. A toggle from another process
/// is a human action, so a second's latency is imperceptible; the cost is one
/// `stat` per tick against a file in the app-support dir.
const WATCH_INTERVAL: Duration = Duration::from_millis(1000);

/// Keep a running engine in step with the domains config after another process
/// writes it.
///
/// Windows, like macOS, hosts the engine inside whichever process enabled it,
/// with no daemon to forward changes to - so [`ProxyManager::set_domain`]
/// updates the engine held by *its own* process and nothing else. From a second
/// process (`gate-connect proxy domain <slug> on` while the menubar app is
/// routing) that handle is `None`: the file was written, `proxy domains`
/// reported the new set, and the engine went on intercepting the old one. The
/// symptom is silence rather than an error - the engine blind-tunnels the host
/// to its real upstream, so the app works and nothing reaches Gate.
///
/// Config and engine disagreeing is precisely the failure this subsystem is
/// meant not to have. Linux fixed its version in #120 by forwarding to the
/// daemon, macOS in #132 with this watcher; Windows was left with neither, which
/// is why this is a port rather than a new mechanism - keep the two in step.
///
/// Watching the file rather than adding a control socket, because the file is
/// already the contract between processes here, and reloading it is safe by
/// construction: [`config::load_domains`] starts from the built-in catalog and
/// applies only per-slug enabled flags, forcing unsupported entries off. So a
/// reload can flip a catalog entry and can never point the MITM at a host the
/// build does not ship - the same guarantee the Linux daemon enforces by
/// validating requests against the catalog.
///
/// Retires on its own when the engine stops, so a disable/enable cycle does not
/// accumulate threads.
fn spawn_domain_watcher() {
    // `swap` rather than load-then-store: two enables racing here would
    // otherwise both see `false` and start a watcher each.
    if WATCHER_ALIVE.swap(true, Ordering::SeqCst) {
        return;
    }
    std::thread::spawn(|| {
        let mgr = manager();
        let mut seen = config::domains_file_mtime();
        loop {
            std::thread::sleep(WATCH_INTERVAL);
            // Never hold the lock across the sleep above: enable/disable take
            // it for whole sequences, and this thread must not be what makes
            // a user-facing toggle wait.
            let engine_gone = mgr
                .engine
                .lock()
                .expect("proxy engine mutex poisoned")
                .is_none();
            if engine_gone {
                WATCHER_ALIVE.store(false, Ordering::SeqCst);
                return;
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
                    eprintln!("gate proxy: could not reload proxy domains ({e}); keeping current");
                    continue;
                }
            };
            if let Some(running) = mgr
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

pub fn manager() -> &'static ProxyManager {
    MANAGER.get_or_init(|| ProxyManager {
        engine: Mutex::new(None),
    })
}

impl ProxyManager {
    /// Current subsystem snapshot for the UI.
    pub fn status(&self) -> Result<ProxyState> {
        let (port, pac_port) = self
            .engine
            .lock()
            .expect("proxy engine mutex poisoned")
            .as_ref()
            .map(|e| (Some(e.port()), Some(e.pac_port())))
            .unwrap_or((None, None));
        Ok(ProxyState {
            running: port.is_some(),
            port,
            pac_port,
            ca_trusted: ca::is_trusted()?,
            env_export_opted_in: crate::proxy::env_export_opted_in(),
            env_export_separable: crate::proxy::env_export_is_separable(),
            domains: config::load_domains()?,
        })
    }

    /// No-op here: this platform runs the engine in-process, so there is no
    /// daemon whose lifetime could outlive the caller and nothing to detach
    /// from. Present so callers need no `cfg` around it.
    pub fn set_detached(&self, _detached: bool) {}

    pub fn list_domains(&self) -> Result<Vec<ProxyDomain>> {
        config::load_domains()
    }

    /// Start the engine, trust the CA, and route the system proxy through it.
    /// The CA trust is the only step that prompts; the proxy change is
    /// promptless. Idempotent: a no-op if already running.
    pub fn enable(&self) -> Result<ProxyState> {
        // Hold the lock for the whole sequence: a concurrent enable must not
        // snapshot the system proxy after this one has already pointed it at
        // our engine - that snapshot would later "restore" a dead port.
        // (handle_engine_crash uses try_lock, so it can't deadlock on this.)
        let mut guard = self.engine.lock().expect("proxy engine mutex poisoned");
        if guard.is_some() {
            drop(guard);
            return self.status();
        }

        let account = account::load()?
            .context("no Gate account configured - sign in before enabling the proxy")?;
        let domains = config::load_domains()?;
        // No enabled-domains guard here: the master switch owns whether the
        // engine runs, while providers/domains own what it intercepts. Starting
        // with zero enabled domains is valid (per-provider toggles can reach that
        // state at runtime too) and lets `provider::restore_all()` re-enable the
        // snapshotted domains immediately after start on master-on.

        let ca = ca::load_or_create()?;

        // Trust the CA so the engine's minted leaf certs validate. Only step
        // that prompts; shows the native dialog once, and only if not already
        // trusted.
        ca::ensure_trusted()?;

        // Snapshot the current WinINET proxy state *before* touching it, so
        // disable can restore it exactly.
        let snapshot = system_proxy::snapshot()?;
        system_proxy::save_snapshot(&snapshot)?;

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
                ca_cert_pem: ca.cert_pem().to_string(),
                ca_key_pem: ca.key_pem().to_string(),
                // Reuse the port we bound last time: WinINET re-resolves after
                // our settings-changed poke, but clients that resolved the
                // proxy once at their own launch keep dialing the old port
                // across our restarts - an app upgrade must come back on the
                // same address or those clients break until relaunched.
                preferred_port: system_proxy::load_port().unwrap_or(None),
                // Same for the PAC port: the AutoConfigURL a client captured
                // at its own launch must keep serving a fresh PAC, or its
                // fetch fails and it falls back to DIRECT, bypassing Gate.
                preferred_pac_port: system_proxy::load_pac_port().unwrap_or(None),
                // Reuse the persisted relay port so CLI tool configs (which bake
                // http://127.0.0.1:<port>) stay valid across restarts.
                preferred_relay_port: crate::proxy::relay::load_persisted_port(),
                // Per-user UID gating is a Linux concern; unused on Windows.
                owner_uid: None,
                // Keep any pre-existing proxy as the PAC fallback so non-Gate
                // traffic still flows through it while routing is on.
                upstream_proxy: system_proxy::upstream_proxy(&snapshot),
            },
            // Fail-safe: if the engine dies unexpectedly, revert the system
            // proxy so traffic is never stranded at a dead listener.
            || manager().handle_engine_crash(),
        )?;

        // Remember the ports for next time (best-effort).
        let _ = system_proxy::save_port(running.port());
        let _ = system_proxy::save_pac_port(running.pac_port());
        // Remember the relay port so the next run rebinds it and baked CLI
        // configs stay valid (best-effort).
        let _ = crate::proxy::relay::save_persisted_port(running.relay_port());

        let pac_url = format!("http://127.0.0.1:{}/proxy.pac", running.pac_port());

        // Point WinINET at the engine's loopback PAC. Promptless.
        if let Err(e) = system_proxy::enable_pac(&pac_url) {
            running.stop();
            let _ = system_proxy::clear_snapshot();
            return Err(e).context("enabling system proxy");
        }

        // Export the proxy variables too, unless the user declined. The PAC
        // above only reaches clients that go through WinINET; the CLI AI tools
        // read `HTTPS_PROXY` instead, and OpenCode has no proxy setting of its
        // own at all. Owned by the `env-proxy` integration, which is why this is
        // a choice and not unconditional - the variables are machine-wide and
        // survive a reboot here, so a user who turned them off must not get them
        // back. Deliberately best-effort: a registry failure must not take
        // routing down for everything that *does* follow the PAC, so it degrades
        // to "GUI apps routed, CLI tools not" rather than to "enable failed".
        if crate::proxy::env_export_opted_in() {
            if let Err(e) = system_proxy::enable_env(running.port()) {
                eprintln!(
                    "gate proxy: could not export proxy environment variables ({e}); GUI apps \
                     still route through Gate, but CLI tools that read HTTPS_PROXY will not"
                );
            }
        }

        *guard = Some(running);
        // The engine now has whatever the config said at startup. Keep it in
        // step with writes made by *other* processes for as long as it runs.
        spawn_domain_watcher();

        // The crash fail-safe defers while we hold the lock; if the engine
        // died somewhere in this sequence, revert here instead of leaving
        // HTTPS routed at a dead port with the snapshot already cleared.
        if guard.as_ref().is_some_and(|r| r.is_finished()) {
            if let Some(dead) = guard.take() {
                dead.stop();
            }
            let snapshot = system_proxy::load_snapshot().unwrap_or_else(|e| {
                eprintln!("gate proxy: unreadable system-proxy snapshot ({e}); forcing proxy off");
                None
            });
            match snapshot {
                Some(snapshot) => system_proxy::restore(&snapshot)?,
                None => system_proxy::force_off()?,
            }
            let _ = system_proxy::clear_snapshot();
            anyhow::bail!("proxy engine exited unexpectedly while enabling");
        }
        drop(guard);
        self.status()
    }

    /// Stop the engine and restore the prior system proxy. Promptless and
    /// unconditional - the revert happens first and never depends on a dialog,
    /// so it can't be cancelled and strand traffic. The CA is left trusted.
    pub fn disable(&self) -> Result<ProxyState> {
        self.disable_inner()?;
        self.status()
    }

    /// Like [`disable`](Self::disable), but returns nothing instead of the
    /// resulting [`ProxyState`]. Used on app exit: `status()` calls
    /// `ca::is_trusted()`, which shells out to `certutil` on Windows, so
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
        let running = self
            .engine
            .lock()
            .expect("proxy engine mutex poisoned")
            .take();

        // Revert the exported variables first, and unconditionally: the PAC
        // restore below can fail with `?`, and of the two channels this is the
        // one where a stale value breaks tools outright rather than merely
        // failing open. A PAC left pointing at a dead port makes clients fall
        // back to DIRECT; an `HTTPS_PROXY` left pointing at a dead port makes
        // every request from a CLI tool fail to connect - and on Windows it
        // would survive the reboot.
        if let Err(e) = system_proxy::disable_env() {
            eprintln!("gate proxy: {e}");
        }

        // An unreadable snapshot must not strand HTTPS at the dead engine
        // port - treat it like a missing one and force the proxy off.
        let snapshot = system_proxy::load_snapshot().unwrap_or_else(|e| {
            eprintln!("gate proxy: unreadable system-proxy snapshot ({e}); forcing proxy off");
            None
        });
        match snapshot {
            Some(snapshot) => system_proxy::restore(&snapshot)?,
            None => system_proxy::force_off()?,
        }
        if let Some(running) = running {
            running.stop();
        }
        let _ = system_proxy::clear_snapshot();
        Ok(())
    }

    /// Toggle a domain. If the engine is running, the new rules are pushed live
    /// - no restart, no prompt.
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

    /// Trust the CA without enabling the proxy (standalone command).
    pub fn trust_ca(&self) -> Result<ProxyState> {
        ca::load_or_create()?; // ensure the cert file exists to trust
        ca::ensure_trusted()?;
        self.status()
    }

    /// Trust the CA machine-wide with no prompt at all, for hosts where nobody
    /// can answer one. CLI-only (`proxy trust-ca --system-trust`) and never
    /// wired to a Tauri command: the prompt is deliberate product behaviour on a
    /// desktop, and this widens the trust to every user on the machine.
    pub fn trust_ca_system(&self) -> Result<ProxyState> {
        ca::load_or_create()?; // ensure the cert file exists to trust
        ca::ensure_trusted_system()?;
        self.status()
    }

    /// Untrust the CA. Refuses while the engine is running, since the engine
    /// mints leaf certs the OS would then reject. This is the explicit way to
    /// remove the standing trusted root (disable alone leaves it trusted).
    pub fn untrust_ca(&self) -> Result<ProxyState> {
        self.refuse_untrust_while_running()?;
        ca::untrust()?;
        self.status()
    }

    /// Remove a machine-wide trust install with no prompt. The counterpart of
    /// [`ProxyManager::trust_ca_system`], and refuses while running for the same
    /// reason [`ProxyManager::untrust_ca`] does.
    pub fn untrust_ca_system(&self) -> Result<ProxyState> {
        self.refuse_untrust_while_running()?;
        ca::untrust_system()?;
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
        // enable still holds it after that, defer - enable re-checks the
        // engine before returning and runs this same revert itself, whereas
        // restoring + clearing the snapshot from here mid-enable would erase
        // the state enable relies on. (A deliberate stop sets `stopping`
        // before signaling, so this isn't reached on that path.)
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
        let _ = guard.take();
        let _ = system_proxy::disable_env();
        let _ = match system_proxy::load_snapshot() {
            Ok(Some(snapshot)) => system_proxy::restore(&snapshot),
            _ => system_proxy::force_off(),
        };
        let _ = system_proxy::clear_snapshot();
    }

    /// Called once at app startup. A leftover snapshot means a previous session
    /// left the system proxy pointed at an engine that no longer exists (unclean
    /// quit / crash) - restore it. Promptless, so it always succeeds; a clean
    /// disable clears the snapshot, making this a no-op.
    pub fn reconcile_on_startup(&self) -> Result<()> {
        // Before anything else, and above the early return below: unlike the
        // WinINET snapshot, the exported variables live in the persistent
        // per-user environment and survive a reboot, so a crashed session
        // leaves them aimed at a port nothing listens on. Clearing them is
        // unconditional - the system-proxy snapshot says nothing about whether
        // they are set. If routing is meant to be on, `enable` runs straight
        // after this and re-exports them at the new engine port.
        if let Err(e) = system_proxy::disable_env() {
            eprintln!("gate proxy: {e}");
        }
        // As in disable: an unreadable snapshot still means an unclean prior
        // session, so force the proxy off rather than bailing and leaving
        // HTTPS routed at a port nothing listens on.
        let snapshot = match system_proxy::load_snapshot() {
            Ok(None) => return Ok(()),
            Ok(Some(s)) => Some(s),
            Err(e) => {
                eprintln!("gate proxy: unreadable system-proxy snapshot ({e}); forcing proxy off");
                None
            }
        };
        match snapshot {
            Some(snapshot) => system_proxy::restore(&snapshot)?,
            None => system_proxy::force_off()?,
        }
        system_proxy::clear_snapshot()?;
        Ok(())
    }
}
