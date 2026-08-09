//! Orchestrates the proxy subsystem: composes the CA, system-proxy, engine,
//! and domain-config modules behind a process-global singleton the Tauri
//! commands call. macOS only.
//!
//! Privilege model: changing the system proxy does *not* require admin, so
//! enable/disable/restore/reconcile run it unprivileged and promptless. The
//! only step that needs admin is trusting the CA, which happens once on
//! enable. Critically, the system-proxy revert never depends on an admin
//! prompt - so it can't be canceled and leave HTTPS routed at a dead port.
//! The CA is left trusted across disable so re-enabling is promptless;
//! removing it is a separate explicit action ([`ProxyManager::untrust_ca`]).

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
    /// The CA trust is the only step that prompts for admin ; the
    /// proxy change is promptless. Idempotent: a no-op if already running.
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
        let services = system_proxy::active_services()?;
        if services.is_empty() {
            anyhow::bail!("no active network services found to route through the proxy");
        }

        let ca = ca::load_or_create()?;

        // Trust the CA so the engine's minted leaf certs validate. Only step
        // that needs admin; prompts once, and only if not already trusted.
        ca::ensure_trusted()?;

        // Snapshot the current system-proxy state *before* touching it, so
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
                // Reuse the port we bound last time: macOS reads the system
                // proxy live, but clients that resolved the proxy once at
                // their own launch (e.g. Claude Desktop) keep dialing the old
                // port across our restarts - an app upgrade must come back on
                // the same address or those clients break until relaunched.
                preferred_port: system_proxy::load_port().unwrap_or(None),
                // Same for the PAC port: the AutoConfigURL a client captured
                // at its own launch must keep serving a fresh PAC, or its
                // fetch fails and it falls back to DIRECT, bypassing Gate.
                preferred_pac_port: system_proxy::load_pac_port().unwrap_or(None),
                // Reuse the persisted relay port so CLI tool configs (which bake
                // http://127.0.0.1:<port>) stay valid across restarts.
                preferred_relay_port: crate::proxy::relay::load_persisted_port(),
                // Per-user UID gating is a Linux concern (shared loopback proxy);
                // unused on macOS.
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

        // Point the system proxy at the engine's loopback PAC. Promptless.
        let pac_url = format!("http://127.0.0.1:{}/proxy.pac", running.pac_port());
        if let Err(e) = system_proxy::enable_pac(&pac_url, &services) {
            running.stop();
            let _ = system_proxy::clear_snapshot();
            return Err(e).context("enabling system proxy");
        }

        // Export the proxy variables too, unless the user declined. The PAC
        // above only reaches clients that consult the OS proxy setting; the CLI
        // AI tools read `HTTPS_PROXY` instead, and OpenCode has no proxy setting
        // of its own at all. Owned by the `env-proxy` integration, which is why
        // this is a choice and not unconditional - the variables are
        // machine-wide, so a user who turned them off must not get them back
        // here. Deliberately best-effort: a launchctl failure must not take
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
                None => {
                    let services = system_proxy::active_services()?;
                    system_proxy::force_off(&services)?;
                }
            }
            let _ = system_proxy::clear_snapshot();
            anyhow::bail!("proxy engine exited unexpectedly while enabling");
        }
        drop(guard);
        self.status()
    }

    /// Stop the engine and restore the prior system proxy. Promptless and
    /// unconditional - the revert happens first and never depends on admin,
    /// so it can't be canceled and strand traffic. The CA is left trusted.
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

        // Revert the exported variables first, and unconditionally: the PAC
        // restore below can fail with `?`, and of the two channels this is the
        // one where a stale value breaks tools outright rather than merely
        // failing open. A PAC left pointing at a dead port makes clients fall
        // back to DIRECT; an `HTTPS_PROXY` left pointing at a dead port makes
        // every request from a CLI tool fail to connect.
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
            None => {
                let services = system_proxy::active_services()?;
                system_proxy::force_off(&services)?;
            }
        }
        if let Some(running) = running {
            running.stop();
        }
        let _ = system_proxy::clear_snapshot();
        Ok(())
    }

    /// Toggle a domain. If the engine is running, the new rules are pushed
    /// live - no restart, no prompt .
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
        let _ = system_proxy::disable_env();
        let _ = match system_proxy::load_snapshot() {
            Ok(Some(snapshot)) => system_proxy::restore(&snapshot),
            _ => system_proxy::active_services().and_then(|s| system_proxy::force_off(&s)),
        };
        let _ = system_proxy::clear_snapshot();
    }

    /// Called once at app startup to undo a system proxy left pointing at an
    /// engine that no longer exists (unclean quit / crash / OS shutdown).
    ///
    /// Two layers, because the graceful-disable `Drop` is bypassed by a hard
    /// kill: (1) a leftover snapshot restores the exact pre-Gate state; (2) a
    /// belt-and-suspenders sweep turns off any service still pointed at a dead
    /// loopback listener even when no (or a partial) snapshot survives - in
    /// that case the PAC fetch fails and traffic silently falls back to
    /// DIRECT, bypassing Gate while it shows "off". Both are promptless, so
    /// this always succeeds; a clean disable makes it a near no-op.
    pub fn reconcile_on_startup(&self) -> Result<()> {
        // Clear any exported proxy variables left over from the prior session
        // before touching the PAC. If routing is meant to be on, `enable` runs
        // straight after this and re-exports them at the new engine port.
        if let Err(e) = system_proxy::disable_env() {
            eprintln!("gate proxy: {e}");
        }
        // As in disable: an unreadable snapshot still means an unclean prior
        // session, so force the proxy off rather than bailing and leaving
        // HTTPS routed at a port nothing listens on.
        match system_proxy::load_snapshot() {
            Ok(Some(snapshot)) => {
                system_proxy::restore(&snapshot)?;
                system_proxy::clear_snapshot()?;
            }
            Ok(None) => {}
            Err(e) => {
                eprintln!("gate proxy: unreadable system-proxy snapshot ({e}); forcing proxy off");
                let services = system_proxy::active_services()?;
                system_proxy::force_off(&services)?;
                system_proxy::clear_snapshot()?;
            }
        }
        let cleared = system_proxy::clear_stranded_loopback()?;
        if !cleared.is_empty() {
            eprintln!(
                "[gate-proxy] startup: cleared stranded loopback proxy on {}",
                cleared.join(", ")
            );
        }
        Ok(())
    }
}
