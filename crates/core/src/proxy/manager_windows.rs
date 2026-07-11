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

use std::sync::{Mutex, OnceLock};

use anyhow::{Context, Result};

use super::{ca, config, engine, system_proxy, ProxyDomain, ProxyState};
use crate::account;

pub struct ProxyManager {
    engine: Mutex<Option<engine::RunningEngine>>,
}

static MANAGER: OnceLock<ProxyManager> = OnceLock::new();

pub fn manager() -> &'static ProxyManager {
    MANAGER.get_or_init(|| ProxyManager {
        engine: Mutex::new(None),
    })
}

impl ProxyManager {
    /// Current subsystem snapshot for the UI.
    pub fn status(&self) -> Result<ProxyState> {
        let port = self
            .engine
            .lock()
            .expect("proxy engine mutex poisoned")
            .as_ref()
            .map(|e| Some(e.port()))
            .unwrap_or(None);
        Ok(ProxyState {
            running: port.is_some(),
            port,
            ca_trusted: ca::is_trusted()?,
            domains: config::load_domains()?,
        })
    }

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
                // WinINET is read live per-process, so an ephemeral port is fine.
                preferred_port: None,
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

        *guard = Some(running);

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

    /// Shared body of [`disable`](Self::disable) /
    /// [`disable_quiet`](Self::disable_quiet): revert the system proxy and stop
    /// the engine, without computing status.
    fn disable_inner(&self) -> Result<()> {
        let running = self
            .engine
            .lock()
            .expect("proxy engine mutex poisoned")
            .take();

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

    /// Untrust the CA. Refuses while the engine is running, since the engine
    /// mints leaf certs the OS would then reject. This is the explicit way to
    /// remove the standing trusted root (disable alone leaves it trusted).
    pub fn untrust_ca(&self) -> Result<ProxyState> {
        if self
            .engine
            .lock()
            .expect("proxy engine mutex poisoned")
            .is_some()
        {
            anyhow::bail!("turn the proxy off before untrusting the CA");
        }
        ca::untrust()?;
        self.status()
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
