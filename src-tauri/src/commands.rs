//! The bodies of the Tauri commands, as plain functions over a [`Platform`].
//!
//! Each `#[tauri::command]` in `lib.rs` that has moved here is one call into
//! this module with [`crate::platform::TauriPlatform`], and nothing else. The
//! wiring stays in `lib.rs` (the attribute, the handler table, the async
//! runtime); what the command decides lives here, where a unit test can run it
//! against [`crate::platform::fake::FakePlatform`] (AG-903).

use crate::platform::Platform;

/// OS identifier ("macos" / "windows" / "linux") so the UI can tailor copy.
pub(crate) fn app_platform(platform: &impl Platform) -> &'static str {
    platform.os()
}

/// Turn native notifications on or off.
pub(crate) fn set_notifications(platform: &impl Platform, enabled: bool) -> Result<(), String> {
    platform
        .set_notifications(enabled)
        .map_err(|e| format!("{e:#}"))
}

/// Record whether Gate Connect may send diagnostic data, then tell every
/// window.
pub(crate) fn set_share_diagnostics(platform: &impl Platform, enabled: bool) -> Result<(), String> {
    platform
        .set_share_diagnostics(enabled)
        .map_err(|e| format!("{e:#}"))?;
    // Every window runs its own analytics client, and only the one the user
    // clicked in knows the answer changed. Broadcast it so the tray and the
    // intro stop (or start) too, instead of acting on the answer they read at
    // launch until the next one (AG-960).
    platform.emit(
        crate::ANALYTICS_CONSENT_EVENT,
        serde_json::json!({ "share_diagnostics": enabled, "recorded": true }),
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform::fake::FakePlatform;

    #[test]
    fn app_platform_reports_the_platform_os() {
        for os in ["macos", "windows", "linux"] {
            let platform = FakePlatform {
                os,
                ..Default::default()
            };
            assert_eq!(app_platform(&platform), os);
        }
    }

    #[test]
    fn set_notifications_stores_the_answer() {
        let platform = FakePlatform::default();
        assert_eq!(set_notifications(&platform, false), Ok(()));
        assert_eq!(*platform.notifications.lock().unwrap(), Some(false));
        assert!(platform.emitted().is_empty(), "nothing to announce");
    }

    /// The whole anyhow chain reaches the webview, not only the outermost
    /// context, so the reason a write failed is still in the message.
    #[test]
    fn set_notifications_reports_the_whole_error_chain() {
        let platform = FakePlatform {
            fail_writes: Some("disk full".into()),
            ..Default::default()
        };
        assert_eq!(
            set_notifications(&platform, true),
            Err("writing preferences: disk full".into())
        );
        assert_eq!(*platform.notifications.lock().unwrap(), None);
    }

    /// The payload the webview reads (`src/lib/analytics.ts`), on the event it
    /// listens on, after the answer is stored.
    #[test]
    fn set_share_diagnostics_stores_then_announces_the_answer() {
        for enabled in [true, false] {
            let platform = FakePlatform::default();
            assert_eq!(set_share_diagnostics(&platform, enabled), Ok(()));
            assert_eq!(*platform.share_diagnostics.lock().unwrap(), Some(enabled));
            assert_eq!(
                platform.emitted(),
                vec![(
                    "analytics-consent-changed".to_string(),
                    serde_json::json!({ "share_diagnostics": enabled, "recorded": true }),
                )]
            );
        }
    }

    /// A refused write announces nothing: the other windows would otherwise
    /// follow an answer that was never recorded.
    #[test]
    fn set_share_diagnostics_announces_nothing_when_the_write_fails() {
        let platform = FakePlatform {
            fail_writes: Some("read-only file system".into()),
            ..Default::default()
        };
        assert_eq!(
            set_share_diagnostics(&platform, false),
            Err("writing preferences: read-only file system".into())
        );
        assert!(platform.emitted().is_empty());
    }
}
