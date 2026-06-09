//! Orchestrates the proxy subsystem: composes the CA, system-proxy, engine,
//! and domain-config modules behind a process-global singleton the Tauri
//! commands call. macOS only.
//!
//! Privilege model: changing the system proxy does *not* require admin, so
//! enable/disable/restore/reconcile run it unprivileged and promptless. The
//! only step that needs admin is trusting the CA, which happens once on
//! enable. Critically, the system-proxy revert never depends on an admin
//! prompt — so it can't be canceled and leave HTTPS routed at a dead port.
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
    /// The CA trust is the only step that prompts for admin ; the
    /// proxy change is promptless. Idempotent: a no-op if already running.
    pub fn enable(&self) -> Result<ProxyState> {
        {
            let guard = self.engine.lock().expect("proxy engine mutex poisoned");
            if guard.is_some() {
                drop(guard);
                return self.status();
            }
        }

        let account = account::load()?
            .context("no Gate account configured — sign in before enabling the proxy")?;
        let domains = config::load_domains()?;
        if !domains.iter().any(|d| d.enabled) {
            anyhow::bail!("enable at least one provider before turning on the proxy");
        }
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
                domains: domains.clone(),
                ca_cert_pem: ca.cert_pem().to_string(),
                ca_key_pem: ca.key_pem().to_string(),
            },
            // Fail-safe: if the engine dies unexpectedly, revert the system
            // proxy so traffic is never stranded at a dead listener.
            || manager().handle_engine_crash(),
        )?;
        let port = running.port();

        // Point the system proxy at the engine. Promptless.
        if let Err(e) = system_proxy::enable(port, &services) {
            running.stop();
            let _ = system_proxy::clear_snapshot();
            return Err(e).context("enabling system proxy");
        }

        *self.engine.lock().expect("proxy engine mutex poisoned") = Some(running);
        self.status()
    }

    /// Stop the engine and restore the prior system proxy. Promptless and
    /// unconditional — the revert happens first and never depends on admin,
    /// so it can't be canceled and strand traffic. The CA is left trusted.
    pub fn disable(&self) -> Result<ProxyState> {
        let running = self
            .engine
            .lock()
            .expect("proxy engine mutex poisoned")
            .take();

        match system_proxy::load_snapshot()? {
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
        self.status()
    }

    /// Toggle a domain. If the engine is running, the new rules are pushed
    /// live — no restart, no prompt .
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
        // try_lock avoids deadlocking if a deliberate stop already holds the
        // mutex (in which case this isn't a crash).
        if let Ok(mut guard) = self.engine.try_lock() {
            let _ = guard.take();
        }
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
    /// loopback listener even when no (or a partial) snapshot survives — that
    /// case otherwise strands every proxy-honoring app with
    /// ERR_PROXY_CONNECTION_FAILED while Gate shows "off". Both are promptless,
    /// so this always succeeds; a clean disable makes it a near no-op.
    pub fn reconcile_on_startup(&self) -> Result<()> {
        if let Some(snapshot) = system_proxy::load_snapshot()? {
            system_proxy::restore(&snapshot)?;
            system_proxy::clear_snapshot()?;
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
