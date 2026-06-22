//! Orchestrates the proxy subsystem on Linux: composes the CA, system-proxy,
//! domain-config, and the long-lived helper daemon behind a process-global
//! singleton the Tauri commands (and the CLI) call. Linux counterpart of the
//! macOS [`super::manager`].
//!
//! Unlike macOS/Windows, the loopback engine here does **not** live in this
//! process — it runs in a detached helper daemon ([`super::helper`]) that owns
//! the port and outlives the GUI. The manager drives it over a control socket
//! ([`super::helper_client`]); the open connection *is* the "proxy on" state,
//! and dropping it (clean disable, or the GUI exiting) makes the daemon fall
//! back to pass-through — the port stays bound, so a session that froze the
//! proxy pointer keeps flowing instead of being stranded.
//!
//! Privilege model. The proxy wiring is a user-scoped systemd `environment.d`
//! drop-in (see [`super::system_proxy`]) and the daemon is unprivileged, so
//! enable, disable, and the startup reconcile are all promptless. Only trusting
//! the CA touches the system trust store and needs root (sudo in a terminal,
//! polkit in a GUI), and only on first enable; once trusted, `ensure_trusted`
//! is a no-op. The CA is left trusted across disable so re-enabling is cheaper;
//! removing it is a separate explicit action ([`ProxyManager::untrust_ca`]).

use std::sync::{Mutex, OnceLock};

use anyhow::{Context, Result};

use super::helper_client::HelperClient;
use super::{ca, config, system_proxy, ProxyDomain, ProxyState};
use crate::account;

pub struct ProxyManager {
    /// Open control connection to the helper daemon. `Some` exactly while the
    /// proxy is on (intercepting); dropping it reverts the daemon to
    /// pass-through.
    client: Mutex<Option<HelperClient>>,
}

static MANAGER: OnceLock<ProxyManager> = OnceLock::new();

pub fn manager() -> &'static ProxyManager {
    MANAGER.get_or_init(|| ProxyManager {
        client: Mutex::new(None),
    })
}

impl ProxyManager {
    /// Current subsystem snapshot for the UI.
    pub fn status(&self) -> Result<ProxyState> {
        let mut guard = self.client.lock().expect("proxy client mutex poisoned");
        // A dead/stale control connection means the daemon's gone or we lost
        // it — treat as off and drop the handle so a later enable reconnects.
        let (running, port) = match guard.as_mut() {
            Some(client) => match client.status() {
                Ok((running, port, _intercepting)) => (running, port),
                Err(_) => {
                    *guard = None;
                    (false, None)
                }
            },
            None => (false, None),
        };
        drop(guard);
        Ok(ProxyState {
            running,
            port,
            ca_trusted: ca::is_trusted()?,
            domains: config::load_domains()?,
        })
    }

    pub fn list_domains(&self) -> Result<Vec<ProxyDomain>> {
        config::load_domains()
    }

    /// Start the engine (in the helper daemon), trust the CA, and route the
    /// system proxy through it. Only the CA install may prompt for elevation
    /// (and only when not already trusted); the proxy write is an unprivileged
    /// user drop-in. Idempotent: a no-op if already on.
    pub fn enable(&self) -> Result<ProxyState> {
        // Hold the lock for the whole sequence so a concurrent enable can't
        // race the snapshot/drop-in writes.
        let mut guard = self.client.lock().expect("proxy client mutex poisoned");
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

        // Record that we left a drop-in behind, so reconcile can clean up after
        // an unclean quit.
        let snapshot = system_proxy::snapshot()?;
        system_proxy::save_snapshot(&snapshot)?;

        // Reuse the port we bound last time so a session that froze the proxy
        // pointer at login keeps reaching a live engine across restarts.
        let preferred_port = system_proxy::load_port().unwrap_or(None);

        // Spawn/connect the daemon and start (or live-update) interception.
        let mut client = HelperClient::connect_or_spawn()?;
        let port = match client.set_intercept(
            &account.gateway_base_url,
            &account.api_key,
            ca.cert_pem(),
            ca.key_pem(),
            &domains,
            preferred_port,
        ) {
            Ok(port) => port,
            Err(e) => {
                let _ = system_proxy::clear_snapshot();
                return Err(e).context("starting proxy interception");
            }
        };
        // Remember the port for next time (best-effort).
        let _ = system_proxy::save_port(port);

        // Point the system proxy at the engine. Unprivileged (user drop-in).
        if let Err(e) = system_proxy::enable(port) {
            let _ = client.set_passthrough();
            drop(client); // closes the connection → daemon stays in pass-through
            let _ = system_proxy::clear_snapshot();
            return Err(e).context("enabling system proxy");
        }

        *guard = Some(client);
        drop(guard);
        self.status()
    }

    /// Stop intercepting and remove the system-proxy drop-in. The daemon is left
    /// running in pass-through (port still bound) so frozen sessions keep
    /// flowing; only its interception is cleared. Unprivileged and promptless.
    pub fn disable(&self) -> Result<ProxyState> {
        let client = self
            .client
            .lock()
            .expect("proxy client mutex poisoned")
            .take();

        // An unreadable snapshot must not strand traffic — treat it like a
        // missing one and force the drop-in off.
        let snapshot = system_proxy::load_snapshot().unwrap_or_else(|e| {
            eprintln!("gate proxy: unreadable system-proxy snapshot ({e}); forcing proxy off");
            None
        });
        match snapshot {
            Some(snapshot) => system_proxy::restore(&snapshot)?,
            None => system_proxy::force_off()?,
        }
        if let Some(mut client) = client {
            // Best-effort: tell the daemon to drop to pass-through. Dropping the
            // connection afterward triggers the same revert as a fail-safe.
            let _ = client.set_passthrough();
        }
        let _ = system_proxy::clear_snapshot();
        self.status()
    }

    /// Toggle a domain. If the proxy is on, push the new rule set to the daemon
    /// live — no restart, no prompt.
    pub fn set_domain(&self, slug: &str, enabled: bool) -> Result<ProxyState> {
        let domains = config::set_enabled(slug, enabled)?;
        if let Some(client) = self
            .client
            .lock()
            .expect("proxy client mutex poisoned")
            .as_mut()
        {
            // Re-push the full intercept config (cheap; the engine updates its
            // rule set live). Best-effort — a failed live update shouldn't
            // wedge the toggle; the next status reflects reality.
            self.push_intercept(client, &domains);
        }
        self.status()
    }

    /// Push a rotated Gate API key into the running daemon, if any — it
    /// otherwise keeps injecting the key it was started with.
    pub fn refresh_api_key(&self, api_key: &str) {
        if let Some(client) = self
            .client
            .lock()
            .expect("proxy client mutex poisoned")
            .as_mut()
        {
            let domains = match config::load_domains() {
                Ok(d) => d,
                Err(_) => return,
            };
            if let (Ok(Some(account)), Ok(ca)) = (account::load(), ca::load_or_create()) {
                let _ = client.set_intercept(
                    &account.gateway_base_url,
                    api_key,
                    ca.cert_pem(),
                    ca.key_pem(),
                    &domains,
                    system_proxy::load_port().unwrap_or(None),
                );
            }
        }
    }

    /// Re-send the current account/CA with `domains` to the daemon as a live
    /// update. Best-effort; errors are logged, not propagated.
    fn push_intercept(&self, client: &mut HelperClient, domains: &[ProxyDomain]) {
        let account = match account::load() {
            Ok(Some(a)) => a,
            _ => return,
        };
        let ca = match ca::load_or_create() {
            Ok(ca) => ca,
            Err(_) => return,
        };
        if let Err(e) = client.set_intercept(
            &account.gateway_base_url,
            &account.api_key,
            ca.cert_pem(),
            ca.key_pem(),
            domains,
            system_proxy::load_port().unwrap_or(None),
        ) {
            eprintln!("gate proxy: live domain update failed: {e}");
        }
    }

    /// Trust the CA without enabling the proxy (standalone command).
    pub fn trust_ca(&self) -> Result<ProxyState> {
        ca::load_or_create()?; // ensure the cert file exists to trust
        ca::ensure_trusted()?;
        self.status()
    }

    /// Untrust the CA. Refuses while the proxy is on, since the engine mints
    /// leaf certs the OS would then reject. This is the explicit way to remove
    /// the standing trusted root (disable alone leaves it trusted).
    pub fn untrust_ca(&self) -> Result<ProxyState> {
        if self
            .client
            .lock()
            .expect("proxy client mutex poisoned")
            .is_some()
        {
            anyhow::bail!("turn the proxy off before untrusting the CA");
        }
        ca::untrust()?;
        self.status()
    }

    /// Called once at app startup. A leftover snapshot means a previous session
    /// left a drop-in behind (unclean quit / crash) — remove it so new sessions
    /// fall through to direct. Any daemon still alive from that session already
    /// reverted to pass-through when its control connection dropped, so live
    /// traffic isn't stranded. Unprivileged; a clean disable clears the
    /// snapshot, making this a no-op.
    pub fn reconcile_on_startup(&self) -> Result<()> {
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
