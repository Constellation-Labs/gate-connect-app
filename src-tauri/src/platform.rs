//! The part of the outside world a command reaches through: the app handle it
//! emits on, the OS it reports, and the store it writes. Behind a trait so the
//! command bodies in `commands.rs` can run in a unit test against
//! [`fake::FakePlatform`] instead of a running app (AG-903).
//!
//! Kept to what the extracted commands use today. A batch that moves another
//! command adds the methods that command needs, and nothing ahead of it.

use serde_json::Value;

pub(crate) trait Platform {
    /// The OS identifier the UI tailors its copy on ("macos" / "windows" /
    /// "linux").
    fn os(&self) -> &'static str;

    /// Broadcast an event to every window. Fire and forget, like every emit in
    /// the shell: a window that is not listening is not a failure.
    fn emit(&self, event: &str, payload: Value);

    /// Persist the native-notifications switch.
    fn set_notifications(&self, enabled: bool) -> anyhow::Result<()>;

    /// Persist the answer to "may Gate Connect send diagnostic data".
    fn set_share_diagnostics(&self, enabled: bool) -> anyhow::Result<()>;
}

/// The running app: the process-wide handle from `setup`, and the core.
pub(crate) struct TauriPlatform;

impl Platform for TauriPlatform {
    fn os(&self) -> &'static str {
        std::env::consts::OS
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

    fn set_share_diagnostics(&self, enabled: bool) -> anyhow::Result<()> {
        gate_connect_core::preferences::set_share_diagnostics(enabled)
    }
}

#[cfg(test)]
pub(crate) mod fake {
    use super::Platform;
    use serde_json::Value;
    use std::sync::Mutex;

    /// Records what a command did and answers what the test set up.
    ///
    /// A preference write either lands in `notifications` /
    /// `share_diagnostics` or fails with `fail_writes`, so a test can see both
    /// what was stored and what the caller was told.
    #[derive(Default)]
    pub(crate) struct FakePlatform {
        pub os: &'static str,
        pub fail_writes: Option<String>,
        pub emitted: Mutex<Vec<(String, Value)>>,
        pub notifications: Mutex<Option<bool>>,
        pub share_diagnostics: Mutex<Option<bool>>,
    }

    impl FakePlatform {
        fn write(&self, slot: &Mutex<Option<bool>>, enabled: bool) -> anyhow::Result<()> {
            if let Some(err) = &self.fail_writes {
                return Err(anyhow::anyhow!("{err}").context("writing preferences"));
            }
            *slot.lock().unwrap() = Some(enabled);
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

        fn emit(&self, event: &str, payload: Value) {
            self.emitted
                .lock()
                .unwrap()
                .push((event.to_string(), payload));
        }

        fn set_notifications(&self, enabled: bool) -> anyhow::Result<()> {
            self.write(&self.notifications, enabled)
        }

        fn set_share_diagnostics(&self, enabled: bool) -> anyhow::Result<()> {
            self.write(&self.share_diagnostics, enabled)
        }
    }
}
