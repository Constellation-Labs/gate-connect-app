//! Orchestrates the proxy subsystem on Linux: composes the CA, system-proxy,
//! engine, and domain-config modules behind a process-global singleton the
//! Tauri commands (and the CLI) call. Linux counterpart of the macOS
//! [`super::manager`]; structurally it mirrors the Windows manager, since Linux
//! also has a single global proxy with no per-network-service concept.
//!
//! Privilege model differs from macOS/Windows. There the system-proxy store is
//! per-user, so disable/reconcile are promptless and the revert can never be
//! cancelled. On Linux both the CA install (system trust store) and the proxy
//! wiring (`/etc/environment`) are root-owned, so enable, disable, and the
//! startup reconcile may each prompt for elevation (sudo in a terminal, polkit
//! in a GUI). Cancelling a disable therefore leaves the proxy on rather than
//! silently reverting — the trade-off for a DE-agnostic, command-line-friendly
//! proxy. The CA is left trusted across disable so re-enabling is cheaper;
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
        let port = self
            .engine
            .lock()
            .expect("proxy engine mutex poisoned")
            .as_ref()
            .map(|e| e.port());
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
    /// Both the CA install and the proxy write may prompt for elevation.
    /// Idempotent: a no-op if already running.
    pub fn enable(&self) -> Result<ProxyState> {
        // Hold the lock for the whole sequence: a concurrent enable must not
        // snapshot the system proxy after this one has already pointed it at
        // our engine — that snapshot would later "restore" a dead port.
        // (handle_engine_crash uses try_lock, so it can't deadlock on this.)
        let mut guard = self.engine.lock().expect("proxy engine mutex poisoned");
        if guard.is_some() {
            drop(guard);
            return self.status();
        }

        let account = account::load()?
            .context("no Gate account configured — sign in before enabling the proxy")?;
        let domains = config::load_domains()?;
        if !domains.iter().any(|d| d.enabled) {
            anyhow::bail!("enable at least one provider before turning on the proxy");
        }

        let ca = ca::load_or_create()?;

        // Trust the CA so the engine's minted leaf certs validate. Installs into
        // the system trust store (privileged); only if not already trusted.
        ca::ensure_trusted()?;

        // Snapshot the current proxy state *before* touching it, so disable can
        // restore it exactly.
        let snapshot = system_proxy::snapshot()?;
        system_proxy::save_snapshot(&snapshot)?;

        let running = engine::start(
            engine::EngineConfig {
                gateway_base_url: account.gateway_base_url.clone(),
                api_key: account.api_key.clone(),
                domains: domains.clone(),
                ca_cert_pem: ca.cert_pem().to_string(),
                ca_key_pem: ca.key_pem().to_string(),
            },
            // Fail-safe: if the engine dies unexpectedly, revert the system
            // proxy so new shells aren't stranded at a dead listener.
            || manager().handle_engine_crash(),
        )?;
        let port = running.port();

        // Point the system proxy at the engine. Privileged.
        if let Err(e) = system_proxy::enable(port) {
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

    /// Stop the engine and restore the prior system proxy. The revert is done
    /// first so that, if it succeeds, the engine is then torn down. On Linux the
    /// revert is privileged and can be cancelled; if it is, the proxy stays on.
    /// The CA is left trusted.
    pub fn disable(&self) -> Result<ProxyState> {
        let running = self
            .engine
            .lock()
            .expect("proxy engine mutex poisoned")
            .take();

        // An unreadable snapshot must not strand HTTPS at the dead engine
        // port — treat it like a missing one and force the proxy off.
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
        self.status()
    }

    /// Toggle a domain. If the engine is running, the new rules are pushed live
    /// — no restart, no prompt.
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

    /// Push a rotated Gate API key into the running engine, if any — the
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
    /// new shells aren't stranded. Best-effort; the revert may prompt.
    pub(crate) fn handle_engine_crash(&self) {
        eprintln!("gate proxy engine exited unexpectedly; reverting system proxy");
        // Briefly retry the lock: short holders (status) clear in ms. If
        // enable still holds it after that, defer — enable re-checks the
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
    /// quit / crash) — restore it. On Linux this may prompt for elevation; a
    /// clean disable clears the snapshot, making this a no-op.
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
