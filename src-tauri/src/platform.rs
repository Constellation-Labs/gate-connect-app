//! The part of the outside world a command reaches through: the app handle it
//! emits on, the OS it reports, and the store it writes. Behind a trait so the
//! command bodies in `commands.rs` can run in a unit test against
//! [`fake::FakePlatform`] instead of a running app (AG-903).
//!
//! Kept to what the extracted commands use today. A batch that moves another
//! command adds the methods that command needs, and nothing ahead of it.

use gate_connect_core::logging::Level;
use gate_connect_core::Integration;
use serde_json::Value;
use std::path::{Path, PathBuf};

pub(crate) trait Platform {
    /// The OS identifier the UI tailors its copy on ("macos" / "windows" /
    /// "linux").
    fn os(&self) -> &'static str;

    /// The OS marketing name and version ("macOS 15.3 (24D60)").
    fn os_name(&self) -> String;

    /// The backend half of the diagnostics report. Shells out on macOS.
    fn diagnostics(&self) -> gate_connect_core::diagnostics::Diagnostics;

    /// This install's stable id, minted on first read.
    fn install_id(&self) -> anyhow::Result<String>;

    /// Write one line to the shared log, when logging is on.
    fn log(&self, level: Level, message: &str);

    // ---- Tools ----

    /// Every integration the core knows, hidden ones included.
    fn integrations(&self) -> Vec<Box<dyn Integration>>;

    /// Find a tool's binary on this machine.
    fn resolve_binary(&self, well_known: &[&str], names: &[&str]) -> Option<PathBuf>;

    /// Ask a binary for its version. Spawns a process on a cold cache.
    fn version_at(&self, path: &Path) -> Option<String>;

    /// One row per model provider.
    fn providers(&self) -> Vec<gate_connect_core::provider::ProviderState>;

    // ---- Preferences ----

    /// The Settings choices, or the defaults when the file is missing or bad.
    fn preferences(&self) -> gate_connect_core::preferences::Preferences;

    /// What to call this machine.
    fn device_name(&self) -> String;

    /// Persist a name for this machine, or clear it.
    fn set_device_name(&self, name: &str) -> anyhow::Result<()>;

    /// Broadcast an event to every window. Fire and forget, like every emit in
    /// the shell: a window that is not listening is not a failure.
    fn emit(&self, event: &str, payload: Value);

    /// Persist the native-notifications switch.
    fn set_notifications(&self, enabled: bool) -> anyhow::Result<()>;

    /// Persist the sound switch for security notifications.
    fn set_security_notification_sound(&self, enabled: bool) -> anyhow::Result<()>;

    /// Persist the answer to "may Gate Connect send diagnostic data".
    fn set_share_diagnostics(&self, enabled: bool) -> anyhow::Result<()>;

    /// Persist the provider domains Gate switched on for one tool.
    fn record_auto_enabled_domains(&self, tool: &str, domains: Vec<String>) -> anyhow::Result<()>;

    /// The provider domains Gate switched on for one tool.
    fn read_auto_enabled_domains(&self, tool: &str) -> Vec<String>;

    // ---- Analytics ----

    /// Claim a once-per-install milestone. True exactly once per name.
    fn claim_milestone(&self, name: &str) -> anyhow::Result<bool>;

    /// Why local Cowork cannot run here, from Claude Desktop's settings.
    fn cowork_setting_missing(&self) -> Option<&'static str>;
}

/// The running app: the process-wide handle from `setup`, and the core.
pub(crate) struct TauriPlatform;

impl Platform for TauriPlatform {
    fn os(&self) -> &'static str {
        std::env::consts::OS
    }

    fn os_name(&self) -> String {
        gate_connect_core::diagnostics::os_name()
    }

    fn diagnostics(&self) -> gate_connect_core::diagnostics::Diagnostics {
        gate_connect_core::diagnostics::collect()
    }

    fn install_id(&self) -> anyhow::Result<String> {
        gate_connect_core::primitives::install_id()
    }

    fn log(&self, level: Level, message: &str) {
        gate_connect_core::logging::log(level, message);
    }

    fn integrations(&self) -> Vec<Box<dyn Integration>> {
        gate_connect_core::registry::registry()
    }

    fn resolve_binary(&self, well_known: &[&str], names: &[&str]) -> Option<PathBuf> {
        gate_connect_core::integrations::binaries::resolve_binary(well_known, names)
    }

    fn version_at(&self, path: &Path) -> Option<String> {
        gate_connect_core::integrations::binaries::version_at(path)
    }

    fn providers(&self) -> Vec<gate_connect_core::provider::ProviderState> {
        gate_connect_core::provider::list()
    }

    fn preferences(&self) -> gate_connect_core::preferences::Preferences {
        gate_connect_core::preferences::load()
    }

    fn device_name(&self) -> String {
        gate_connect_core::preferences::device_name()
    }

    fn set_device_name(&self, name: &str) -> anyhow::Result<()> {
        gate_connect_core::preferences::set_device_name(name)
    }

    fn emit(&self, event: &str, payload: Value) {
        use tauri::Emitter;
        // Unset before `setup` runs, as at every other emit site here.
        if let Some(handle) = crate::APP_HANDLE.get() {
            let _ = handle.emit(event, payload);
        }
    }

    fn set_notifications(&self, enabled: bool) -> anyhow::Result<()> {
        gate_connect_core::preferences::set_notifications(enabled)
    }

    fn set_security_notification_sound(&self, enabled: bool) -> anyhow::Result<()> {
        gate_connect_core::preferences::set_security_notification_sound(enabled)
    }

    fn set_share_diagnostics(&self, enabled: bool) -> anyhow::Result<()> {
        gate_connect_core::preferences::set_share_diagnostics(enabled)
    }

    fn record_auto_enabled_domains(&self, tool: &str, domains: Vec<String>) -> anyhow::Result<()> {
        gate_connect_core::preferences::record_auto_enabled_domains(tool, domains)
    }

    fn read_auto_enabled_domains(&self, tool: &str) -> Vec<String> {
        gate_connect_core::preferences::read_auto_enabled_domains(tool)
    }

    fn claim_milestone(&self, name: &str) -> anyhow::Result<bool> {
        gate_connect_core::analytics::claim(name)
    }

    fn cowork_setting_missing(&self) -> Option<&'static str> {
        gate_connect_core::analytics::cowork_setting_missing()
    }
}

#[cfg(test)]
pub(crate) mod fake {
    use super::Platform;
    use gate_connect_core::logging::Level;
    use gate_connect_core::registry::{Mechanism, Status, ToolId};
    use gate_connect_core::taxonomy::Client;
    use gate_connect_core::{ConnectInput, Integration};
    use serde_json::Value;
    use std::collections::{BTreeMap, HashSet};
    use std::path::{Path, PathBuf};
    use std::sync::Mutex;

    /// Records what a command did and answers what the test set up.
    ///
    /// A preference write either lands in its slot or fails with
    /// `fail_writes`, so a test can see both what was stored and what the
    /// caller was told.
    #[derive(Default)]
    pub(crate) struct FakePlatform {
        pub os: &'static str,
        pub os_name: String,
        pub install_id: Option<String>,
        pub fail_writes: Option<String>,
        pub tools: Vec<FakeTool>,
        /// Binary name to the version it reports (`None`: found, would not say).
        /// A name not listed is not installed.
        pub binaries: BTreeMap<&'static str, Option<String>>,
        pub cowork_missing: Option<&'static str>,
        pub emitted: Mutex<Vec<(String, Value)>>,
        pub logged: Mutex<Vec<(Level, String)>>,
        pub notifications: Mutex<Option<bool>>,
        pub share_diagnostics: Mutex<Option<bool>>,
        pub security_sound: Mutex<Option<bool>>,
        pub device_name: Mutex<String>,
        pub auto_enabled: Mutex<BTreeMap<String, Vec<String>>>,
        pub milestones: Mutex<HashSet<String>>,
    }

    impl FakePlatform {
        fn check_write(&self) -> anyhow::Result<()> {
            match &self.fail_writes {
                Some(err) => Err(anyhow::anyhow!("{err}").context("writing preferences")),
                None => Ok(()),
            }
        }

        fn write<T>(&self, slot: &Mutex<T>, value: T) -> anyhow::Result<()> {
            self.check_write()?;
            *slot.lock().unwrap() = value;
            Ok(())
        }

        pub fn emitted(&self) -> Vec<(String, Value)> {
            self.emitted.lock().unwrap().clone()
        }
    }

    impl Platform for FakePlatform {
        fn os(&self) -> &'static str {
            self.os
        }

        fn os_name(&self) -> String {
            self.os_name.clone()
        }

        fn diagnostics(&self) -> gate_connect_core::diagnostics::Diagnostics {
            unimplemented!("no test reads the diagnostics report through the fake")
        }

        fn install_id(&self) -> anyhow::Result<String> {
            self.install_id
                .clone()
                .ok_or_else(|| anyhow::anyhow!("no install id").context("reading install id"))
        }

        fn log(&self, level: Level, message: &str) {
            self.logged
                .lock()
                .unwrap()
                .push((level, message.to_string()));
        }

        fn integrations(&self) -> Vec<Box<dyn Integration>> {
            self.tools
                .iter()
                .map(|t| Box::new(t.clone()) as Box<dyn Integration>)
                .collect()
        }

        fn resolve_binary(&self, _well_known: &[&str], names: &[&str]) -> Option<PathBuf> {
            names
                .iter()
                .find(|n| self.binaries.contains_key(*n))
                .map(PathBuf::from)
        }

        fn version_at(&self, path: &Path) -> Option<String> {
            self.binaries.get(path.to_str()?).cloned().flatten()
        }

        fn providers(&self) -> Vec<gate_connect_core::provider::ProviderState> {
            Vec::new()
        }

        fn preferences(&self) -> gate_connect_core::preferences::Preferences {
            gate_connect_core::preferences::Preferences::default()
        }

        fn device_name(&self) -> String {
            self.device_name.lock().unwrap().clone()
        }

        fn set_device_name(&self, name: &str) -> anyhow::Result<()> {
            self.write(&self.device_name, name.to_string())
        }

        fn emit(&self, event: &str, payload: Value) {
            self.emitted
                .lock()
                .unwrap()
                .push((event.to_string(), payload));
        }

        fn set_notifications(&self, enabled: bool) -> anyhow::Result<()> {
            self.write(&self.notifications, Some(enabled))
        }

        fn set_security_notification_sound(&self, enabled: bool) -> anyhow::Result<()> {
            self.write(&self.security_sound, Some(enabled))
        }

        fn set_share_diagnostics(&self, enabled: bool) -> anyhow::Result<()> {
            self.write(&self.share_diagnostics, Some(enabled))
        }

        fn record_auto_enabled_domains(
            &self,
            tool: &str,
            domains: Vec<String>,
        ) -> anyhow::Result<()> {
            self.check_write()?;
            self.auto_enabled
                .lock()
                .unwrap()
                .insert(tool.to_string(), domains);
            Ok(())
        }

        fn read_auto_enabled_domains(&self, tool: &str) -> Vec<String> {
            self.auto_enabled
                .lock()
                .unwrap()
                .get(tool)
                .cloned()
                .unwrap_or_default()
        }

        fn claim_milestone(&self, name: &str) -> anyhow::Result<bool> {
            self.check_write()?;
            Ok(self.milestones.lock().unwrap().insert(name.to_string()))
        }

        fn cowork_setting_missing(&self) -> Option<&'static str> {
            self.cowork_missing
        }
    }

    /// A tool the registry would list. Only what the moved commands read is
    /// configurable; the rest answers fixed values or refuses.
    #[derive(Clone)]
    pub(crate) struct FakeTool {
        pub id: ToolId,
        pub hidden: bool,
        /// Binary names to look for. Empty: the tool has no binary.
        pub binaries: &'static [&'static str],
        /// `Err` carries the message `status()` fails with.
        pub status: Result<Status, String>,
    }

    impl FakeTool {
        pub fn new(id: ToolId) -> Self {
            Self {
                id,
                hidden: false,
                binaries: &[],
                status: Ok(Status::Detected),
            }
        }
    }

    impl Integration for FakeTool {
        fn id(&self) -> ToolId {
            self.id
        }
        fn display_name(&self) -> &'static str {
            "Fake Tool"
        }
        fn row_label(&self) -> &'static str {
            "Fake"
        }
        fn client(&self) -> Client {
            Client::ClaudeCode
        }
        fn binary(&self) -> (&'static [&'static str], &'static [&'static str]) {
            (&[], self.binaries)
        }
        fn upstream_provider_name(&self) -> &'static str {
            "Fake Provider"
        }
        fn default_upstream_url(&self) -> &'static str {
            "https://fake.example.com"
        }
        fn mechanism(&self) -> Mechanism {
            Mechanism::Relay
        }
        fn configured_addresses(&self) -> anyhow::Result<Vec<String>> {
            Ok(Vec::new())
        }
        fn watch_paths(&self) -> Vec<PathBuf> {
            Vec::new()
        }
        fn detect(&self) -> anyhow::Result<bool> {
            Ok(true)
        }
        fn status(&self) -> anyhow::Result<Status> {
            self.status
                .clone()
                .map_err(|e| anyhow::anyhow!("{e}").context("reading the config"))
        }
        fn connect(&self, _input: &ConnectInput) -> anyhow::Result<()> {
            unimplemented!("no moved command connects a tool")
        }
        fn disconnect(&self) -> anyhow::Result<()> {
            unimplemented!("no moved command disconnects a tool")
        }
        fn hidden_in_ui(&self) -> bool {
            self.hidden
        }
    }
}
