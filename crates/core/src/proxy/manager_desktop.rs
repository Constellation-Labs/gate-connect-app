//! OS wiring for the shared desktop-manager sequencing ([`manager_core`]):
//! the [`DesktopOps`] impl that touches the real system proxy, trust store,
//! and persisted ports, plus the process-global singleton. This file is the
//! only manager code that differs between macOS and Windows, and the
//! differences are confined to the `cfg` blocks below - everything else,
//! including every ordering invariant, lives once in `manager_core.rs`.

use std::sync::OnceLock;

use anyhow::Result;

use super::manager_core::{CaMaterial, DesktopManager, DesktopOps};
use super::{ca, engine, system_proxy};

pub type ProxyManager = DesktopManager<OsOps>;

static MANAGER: OnceLock<ProxyManager> = OnceLock::new();

pub fn manager() -> &'static ProxyManager {
    MANAGER.get_or_init(|| DesktopManager::new(OsOps))
}

pub struct OsOps;

impl DesktopOps for OsOps {
    type Snapshot = system_proxy::ProxySnapshot;

    /// macOS: refuse before the CA prompt when there is nothing to route -
    /// `networksetup` needs at least one active service to point at the PAC.
    /// Windows WinINET is per-user registry state and always addressable.
    fn preflight_enable(&self) -> Result<()> {
        #[cfg(target_os = "macos")]
        if system_proxy::active_services()?.is_empty() {
            anyhow::bail!("no active network services found to route through the proxy");
        }
        Ok(())
    }

    fn snapshot(&self) -> Result<Self::Snapshot> {
        system_proxy::snapshot()
    }

    fn save_snapshot(&self, snapshot: &Self::Snapshot) -> Result<()> {
        system_proxy::save_snapshot(snapshot)
    }

    fn load_snapshot(&self) -> Result<Option<Self::Snapshot>> {
        system_proxy::load_snapshot()
    }

    fn clear_snapshot(&self) -> Result<()> {
        system_proxy::clear_snapshot()
    }

    fn restore(&self, snapshot: &Self::Snapshot) -> Result<()> {
        system_proxy::restore(snapshot)
    }

    /// macOS's `networksetup` needs the service list to switch slots off; the
    /// Windows registry write does not. Fetching the list here (rather than
    /// carrying it from `preflight_enable`) costs one extra promptless
    /// subprocess on a path that is already subprocess-bound, and keeps the
    /// seam's methods independent.
    fn force_off(&self) -> Result<()> {
        #[cfg(target_os = "macos")]
        {
            let services = system_proxy::active_services()?;
            system_proxy::force_off(&services)
        }
        #[cfg(target_os = "windows")]
        system_proxy::force_off()
    }

    fn upstream_proxy(&self, snapshot: &Self::Snapshot) -> Option<String> {
        system_proxy::upstream_proxy(snapshot)
    }

    fn point_system_proxy_at(&self, running: &engine::RunningEngine) -> Result<()> {
        let pac_url = format!("http://127.0.0.1:{}/proxy.pac", running.pac_port());
        #[cfg(target_os = "macos")]
        {
            let services = system_proxy::active_services()?;
            system_proxy::enable_pac(&pac_url, &services)
        }
        #[cfg(target_os = "windows")]
        system_proxy::enable_pac(&pac_url)
    }

    fn persist_ports(&self, running: &engine::RunningEngine) {
        let _ = system_proxy::save_port(running.port());
        let _ = system_proxy::save_pac_port(running.pac_port());
    }

    fn preferred_engine_port(&self) -> Option<u16> {
        system_proxy::load_port().unwrap_or(None)
    }

    fn preferred_pac_port(&self) -> Option<u16> {
        system_proxy::load_pac_port().unwrap_or(None)
    }

    fn engine_pac_port(&self, running: &engine::RunningEngine) -> Option<u16> {
        Some(running.pac_port())
    }

    fn enable_env(&self, port: u16) -> Result<()> {
        system_proxy::enable_env(port)
    }

    fn disable_env(&self) -> Result<()> {
        system_proxy::disable_env()
    }

    fn clear_stranded_loopback(&self) -> Result<Vec<String>> {
        system_proxy::clear_stranded_loopback()
    }

    fn engine_hosted_elsewhere(&self) -> Option<u16> {
        crate::proxy::engine_hosted_elsewhere()
    }

    fn ca_load_or_create(&self) -> Result<CaMaterial> {
        let ca = ca::load_or_create()?;
        Ok(CaMaterial {
            cert_pem: ca.cert_pem().to_string(),
            key_pem: ca.key_pem().to_string(),
        })
    }

    fn ca_ensure_trusted(&self) -> Result<()> {
        ca::ensure_trusted()
    }

    fn ca_ensure_trusted_system(&self) -> Result<()> {
        ca::ensure_trusted_system()
    }

    fn ca_untrust(&self) -> Result<()> {
        ca::untrust()
    }

    fn ca_untrust_system(&self) -> Result<()> {
        ca::untrust_system()
    }

    fn ca_is_trusted(&self) -> Result<bool> {
        ca::is_trusted()
    }
}
