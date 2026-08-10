//! Orchestrates the proxy subsystem on Linux: composes the CA, system-proxy,
//! domain-config, and the long-lived helper daemon behind a process-global
//! singleton the Tauri commands (and the CLI) call. Linux counterpart of the
//! macOS [`super::manager`].
//!
//! Unlike macOS/Windows, the loopback engine here does **not** live in this
//! process - it runs in a detached helper daemon ([`super::helper`]) that owns
//! the port and outlives the GUI. The manager drives it over a control socket
//! ([`super::helper_client`]); the open connection *is* the "proxy on" state,
//! and dropping it (clean disable, or the GUI exiting) makes the daemon fall
//! back to pass-through - the port stays bound, so a session that froze the
//! proxy pointer keeps flowing instead of being stranded.
//!
//! Privilege model. The proxy wiring is a user-scoped systemd `environment.d`
//! drop-in (see [`super::system_proxy`]) and the daemon is unprivileged, so
//! enable, disable, and the startup reconcile are all promptless. Only trusting
//! the CA touches the system trust store and needs root (sudo in a terminal,
//! polkit in a GUI), and only on first enable; once trusted, `ensure_trusted`
//! is a no-op. The CA is left trusted across disable so re-enabling is cheaper;
//! removing it is a separate explicit action ([`ProxyManager::untrust_ca`]).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};

use anyhow::{Context, Result};

use super::flock::FileLock;
use super::helper_client::HelperClient;
use super::{ca, config, system_proxy, ProxyDomain, ProxyState};
use crate::account;

pub struct ProxyManager {
    /// Open control connection to the helper daemon. `Some` exactly while the
    /// proxy is on (intercepting); dropping it reverts the daemon to
    /// pass-through - unless [`ProxyManager::set_detached`] said otherwise.
    client: Mutex<Option<HelperClient>>,
    /// Whether this process's lifetime should NOT bound the routing lifetime.
    /// False by default, which is right for the GUI: it holds the control
    /// connection for exactly as long as routing is meant to be on, so a lost
    /// connection genuinely means "stop routing". A short-lived caller (the
    /// CLI) sets it, or its own exit would silently un-route the machine.
    detached: AtomicBool,
}

static MANAGER: OnceLock<ProxyManager> = OnceLock::new();

pub fn manager() -> &'static ProxyManager {
    MANAGER.get_or_init(|| ProxyManager {
        client: Mutex::new(None),
        detached: AtomicBool::new(false),
    })
}

impl ProxyManager {
    /// Current subsystem snapshot for the UI.
    pub fn status(&self) -> Result<ProxyState> {
        let mut guard = self.client.lock().expect("proxy client mutex poisoned");
        // Adopt a running daemon when this process has none of its own, the
        // same fallback `set_domain` uses. `self.client` is `Some` only for a
        // caller that enabled the proxy and stayed alive holding the handle -
        // the GUI - so every CLI invocation fell through to the `None` arm
        // below and reported the proxy off without ever asking the daemon.
        // Measured: `proxy status` printed `Proxy:    stopped` while the daemon
        // was live and intercepting on 127.0.0.1:45981.
        //
        // Gated on the snapshot because the attempt is not free: connecting to
        // a daemon that won't answer (a different build - the handshake pins
        // BUILD_FINGERPRINT - or one busy serving the GUI's held connection)
        // costs the full CONTROL_TIMEOUT. The snapshot exists for exactly as
        // long as some process has the system proxy routed through a live
        // engine, so this spends those 3s only when there is really an engine
        // to ask, and the proxy-off path stays as cheap as it was.
        if guard.is_none() && crate::proxy::engine_likely_running() {
            if let Ok(client) = HelperClient::connect_existing() {
                *guard = Some(client);
            }
        }
        // A dead/stale control connection means the daemon's gone or we lost
        // it - treat as off and drop the handle so a later enable reconnects.
        let (running, port) = match guard.as_mut() {
            Some(client) => match client.status() {
                Ok((running, port, _intercepting)) => (running, port),
                // A single failed round-trip could be a transient blip while
                // the daemon is still intercepting - don't desync by dropping
                // the handle on the first error. Try one fresh connection to
                // the existing daemon; only declare the proxy off if that also
                // fails (daemon truly gone).
                Err(_) => match HelperClient::connect_existing() {
                    Ok(mut fresh) => match fresh.status() {
                        Ok((running, port, _)) => {
                            *guard = Some(fresh);
                            (running, port)
                        }
                        Err(_) => {
                            *guard = None;
                            (false, None)
                        }
                    },
                    Err(_) => {
                        *guard = None;
                        (false, None)
                    }
                },
            },
            None => (false, None),
        };
        drop(guard);
        Ok(ProxyState {
            running,
            port,
            // Linux wires env-var proxies straight at the engine port; there
            // is no PAC listener.
            pac_port: None,
            ca_trusted: ca::is_trusted()?,
            env_export_opted_in: crate::proxy::env_export_opted_in(),
            env_export_separable: crate::proxy::env_export_is_separable(),
            domains: config::load_domains()?,
        })
    }

    /// Declare that this process's lifetime does not bound the routing
    /// lifetime, so the daemon keeps intercepting after the control connection
    /// closes. For short-lived callers only - the CLI. The GUI must never set
    /// it: its connection IS the signal that someone is still minding the
    /// proxy, and detaching would leave a machine routed with no owner.
    ///
    /// Takes effect on the next `enable`; it travels in `SetIntercept`.
    pub fn set_detached(&self, detached: bool) {
        self.detached.store(detached, Ordering::SeqCst);
    }

    pub fn list_domains(&self) -> Result<Vec<ProxyDomain>> {
        config::load_domains()
    }

    /// Start the engine (in the helper daemon), trust the CA, and route the
    /// system proxy through it. Only the CA install may prompt for elevation
    /// (and only when not already trusted); the proxy write is an unprivileged
    /// user drop-in. Idempotent: a no-op if already on.
    pub fn enable(&self) -> Result<ProxyState> {
        // Cross-process lock: keep the app and the CLI from interleaving the
        // snapshot / drop-in / port writes below. Held for the whole sequence.
        let _op_lock = FileLock::acquire(&system_proxy::op_lock_path()?, true)?;
        // Hold the in-process lock for the whole sequence so a concurrent enable
        // can't race the snapshot/drop-in writes.
        let mut guard = self.client.lock().expect("proxy client mutex poisoned");
        if guard.is_some() {
            drop(guard);
            return self.status();
        }

        let account = account::load()?
            .context("no Gate account configured - sign in before enabling the proxy")?;
        // Start the engine even with zero enabled domains - it then passes
        // everything through, and enabling a provider flips its domain live
        // once the proxy is running. Matches the cross-platform manager
        // (manager.rs) and lets the proxy bootstrap from a clean first run.
        let domains = config::load_domains()?;

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
        let bound = match client.set_intercept(
            &account.gateway_base_url,
            &account.api_key,
            &crate::oauth::access_token_for_injection(),
            &crate::account::org_id_for_injection(),
            ca.cert_pem(),
            ca.key_pem(),
            &domains,
            self.detached.load(Ordering::SeqCst),
            preferred_port,
            crate::proxy::relay::load_persisted_port(),
        ) {
            Ok(bound) => bound,
            Err(e) => {
                let _ = system_proxy::clear_snapshot();
                return Err(e).context("starting proxy interception");
            }
        };
        let port = bound.port;
        // Remember the ports for next time (best-effort). The relay port must be
        // stable too, since CLI tool configs bake http://127.0.0.1:<relay_port>.
        let _ = system_proxy::save_port(port);
        let _ = crate::proxy::relay::save_persisted_port(bound.relay_port);

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
        // Cross-process lock: serialize against a concurrent app/CLI enable.
        let _op_lock = FileLock::acquire(&system_proxy::op_lock_path()?, true)?;
        self.disable_inner()?;
        self.status()
    }

    /// Like [`disable`](Self::disable), but returns nothing instead of the
    /// resulting [`ProxyState`]. Used on app exit: `status()` does a control
    /// round-trip to the daemon and probes CA trust via `ca::is_trusted()` -
    /// work the exiting process only discards. Clearing interception never
    /// needs either.
    pub fn disable_quiet(&self) -> Result<()> {
        // Cross-process lock: serialize against a concurrent app/CLI enable.
        let _op_lock = FileLock::acquire(&system_proxy::op_lock_path()?, true)?;
        self.disable_inner()
    }

    /// Shared body of [`disable`](Self::disable) /
    /// [`disable_quiet`](Self::disable_quiet): revert the system proxy and drop
    /// the daemon to pass-through, without computing status. Assumes the caller
    /// holds the cross-process op lock.
    fn disable_inner(&self) -> Result<()> {
        let mut guard = self.client.lock().expect("proxy client mutex poisoned");
        // Adopt a running daemon when this process has none of its own, like
        // `status` and `set_domain`. Without this the `if let Some` below was
        // dead code for the CLI - every invocation is a fresh process, so the
        // handle was always `None` - and `proxy disable` cleared the snapshot
        // and removed the drop-in without ever telling the daemon to stop
        // intercepting. It kept its rule set and its ports; only *newly*
        // launched processes stopped routing, because the drop-in was gone,
        // which is why it looked like it had worked.
        //
        // Deliberately not gated on the snapshot the way `status` is. Disable
        // is the forceful path - a daemon still intercepting with no snapshot
        // is exactly the state a user runs this to escape - and it is rare and
        // explicit enough to afford the control timeout when there turns out
        // to be nothing to adopt.
        if guard.is_none() {
            if let Ok(client) = HelperClient::connect_existing() {
                *guard = Some(client);
            }
        }
        let client = guard.take();
        // Released before `status`, which takes the same lock.
        drop(guard);

        // An unreadable snapshot must not strand traffic - treat it like a
        // missing one and force the drop-in off.
        let snapshot = system_proxy::load_snapshot().unwrap_or_else(|e| {
            eprintln!("gate proxy: unreadable system-proxy snapshot ({e}); forcing proxy off");
            None
        });
        let off_result = match snapshot {
            Some(snapshot) => system_proxy::restore(&snapshot),
            None => system_proxy::force_off(),
        };
        // Drop the daemon to pass-through and clear the snapshot *even if* the
        // drop-in delete failed - otherwise a leftover snapshot would re-honor
        // (turn the proxy back on) on the next launch, against the user's
        // intent. Surface the delete error afterward.
        if let Some(mut client) = client {
            let _ = client.set_passthrough();
        }
        let _ = system_proxy::clear_snapshot();
        off_result.context("removing the system-proxy drop-in")?;
        Ok(())
    }

    /// Toggle a domain. If the proxy is on, push the new rule set to the daemon
    /// live - no restart, no prompt.
    pub fn set_domain(&self, slug: &str, enabled: bool) -> Result<ProxyState> {
        let domains = config::set_enabled(slug, enabled)?;
        let mut guard = self.client.lock().expect("proxy client mutex poisoned");
        // Adopt a running daemon when this process has no connection of its
        // own. `self.client` is only ever `Some` for a caller that enabled the
        // proxy itself and stayed alive to hold the handle - the GUI. Every CLI
        // invocation is a fresh process, so without this the toggle wrote
        // config and stopped there: the engine kept its old rules, `proxy
        // domains` reported the new state off config, and the provider did not
        // actually route until routing was turned off and on again. Measured -
        // it is why the e2e's OpenRouter phase tunnelled while the CLI called
        // the domain enabled. Same `connect_existing` fallback `status` uses.
        if guard.is_none() {
            if let Ok(client) = HelperClient::connect_existing() {
                *guard = Some(client);
            }
        }
        if let Some(client) = guard.as_mut() {
            // Re-push the full intercept config (cheap; the engine updates its
            // rule set live). Best-effort - a failed live update shouldn't
            // wedge the toggle; the next status reflects reality.
            self.push_intercept(client, &domains);
        }
        // Released before `status`, which takes the same lock.
        drop(guard);
        self.status()
    }

    /// Push a rotated Gate API key into the running daemon, if any - it
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
                    &crate::oauth::access_token_for_injection(),
                    &crate::account::org_id_for_injection(),
                    ca.cert_pem(),
                    ca.key_pem(),
                    &domains,
                    self.detached.load(Ordering::SeqCst),
                    system_proxy::load_port().unwrap_or(None),
                    crate::proxy::relay::load_persisted_port(),
                );
            }
        }
    }

    /// Push a refreshed OAuth access token into the running daemon, if any.
    /// Empty string reverts to the API key. Re-sends the current account/CA
    /// so the live update carries the new token to in-flight routing.
    pub fn refresh_token(&self, oauth_token: &str) {
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
                    &account.api_key,
                    oauth_token,
                    &crate::account::org_id_for_injection(),
                    ca.cert_pem(),
                    ca.key_pem(),
                    &domains,
                    self.detached.load(Ordering::SeqCst),
                    system_proxy::load_port().unwrap_or(None),
                    crate::proxy::relay::load_persisted_port(),
                );
            }
        }
    }

    /// Push a newly-selected org UUID into the running daemon, if any. Empty
    /// string clears it. Re-sends the current account/CA/token so the live
    /// update carries the new org to in-flight routing.
    pub fn refresh_org(&self, org_id: &str) {
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
                    &account.api_key,
                    &crate::oauth::access_token_for_injection(),
                    org_id,
                    ca.cert_pem(),
                    ca.key_pem(),
                    &domains,
                    self.detached.load(Ordering::SeqCst),
                    system_proxy::load_port().unwrap_or(None),
                    crate::proxy::relay::load_persisted_port(),
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
            &crate::oauth::access_token_for_injection(),
            &crate::account::org_id_for_injection(),
            ca.cert_pem(),
            ca.key_pem(),
            domains,
            self.detached.load(Ordering::SeqCst),
            system_proxy::load_port().unwrap_or(None),
            crate::proxy::relay::load_persisted_port(),
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

    /// Trust the CA machine-wide with no prompt at all, for hosts where nobody
    /// can answer one. CLI-only (`proxy trust-ca --system-trust`) and never
    /// wired to a Tauri command: the prompt is deliberate product behaviour on a
    /// desktop, and this widens the trust to every user on the machine.
    pub fn trust_ca_system(&self) -> Result<ProxyState> {
        ca::load_or_create()?; // ensure the cert file exists to trust
        ca::ensure_trusted_system()?;
        self.status()
    }

    /// Untrust the CA. Refuses while the proxy is on, since the engine mints
    /// leaf certs the OS would then reject. This is the explicit way to remove
    /// the standing trusted root (disable alone leaves it trusted).
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
            .client
            .lock()
            .expect("proxy client mutex poisoned")
            .is_some()
        {
            anyhow::bail!("turn the proxy off before untrusting the CA");
        }
        Ok(())
    }

    /// Called once at app startup. A leftover snapshot means the proxy was on
    /// when the previous session ended (crash / unclean quit / reboot) - a clean
    /// disable clears it, making this a no-op.
    ///
    /// Re-honor: rather than tearing the proxy down, bring it back up on the
    /// same stable port, so a session that froze the proxy pointer at login
    /// keeps reaching a live engine (and the user's "on" survives a crash or
    /// reboot). If we can't - no account, no provider enabled, the CA isn't
    /// trusted (so re-honoring would prompt), or the daemon won't start - fall
    /// back to a clean slate so nothing is stranded. Runs off the main thread
    /// (see the setup hook), so the daemon spin-up doesn't block the tray.
    pub fn reconcile_on_startup(&self) -> Result<()> {
        match system_proxy::load_snapshot() {
            // Clean prior exit - nothing to reconcile.
            Ok(None) => return Ok(()),
            // The proxy was on; try to re-honor it below.
            Ok(Some(_)) => {}
            Err(e) => {
                // Can't trust the snapshot - don't re-honor blindly; clean slate.
                eprintln!("gate proxy: unreadable system-proxy snapshot ({e}); forcing proxy off");
                return self.force_clean_slate();
            }
        }

        // Guard on CA trust so re-honoring never triggers a startup elevation
        // prompt: the proxy being on before implies the CA is trusted, so
        // `enable`'s `ensure_trusted` is a no-op; if it somehow isn't trusted,
        // we decline and clean up instead.
        if ca::is_trusted().unwrap_or(false) {
            // Mirror the app/CLI enable flow: restore the providers a prior
            // master-off disabled before re-enabling, so re-honor brings back
            // the user's actual domain selection rather than starting bare.
            let _ = crate::provider::restore_all();
            match self.enable() {
                Ok(_) => {
                    // Second restore pass: domain-only providers have nothing
                    // to configure until the proxy is running, so the
                    // pre-enable pass leaves them in the snapshot.
                    let _ = crate::provider::restore_all();
                    return Ok(());
                }
                Err(e) => {
                    eprintln!("gate proxy: re-honor on startup failed ({e}); forcing proxy off")
                }
            }
        } else {
            eprintln!("gate proxy: CA not trusted at startup; not re-honoring, forcing proxy off");
        }

        // Couldn't re-honor - strip any drop-in and drop the stale snapshot.
        self.force_clean_slate()
    }

    /// Strip our drop-in and clear the snapshot - the safe "off" state when we
    /// can't (or shouldn't) re-honor. Also reverts any half-open client to
    /// pass-through.
    fn force_clean_slate(&self) -> Result<()> {
        if let Some(mut client) = self
            .client
            .lock()
            .expect("proxy client mutex poisoned")
            .take()
        {
            let _ = client.set_passthrough();
        }
        system_proxy::force_off()?;
        system_proxy::clear_snapshot()?;
        Ok(())
    }
}
