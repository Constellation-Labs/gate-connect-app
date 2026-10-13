//! The bodies of the Tauri commands, as plain functions over a [`Platform`].
//!
//! Each `#[tauri::command]` in `lib.rs` that has moved here is one call into
//! this module with [`crate::platform::TauriPlatform`], and nothing else. The
//! wiring stays in `lib.rs` (the attribute, the handler table, the async
//! runtime); what the command decides lives here, where a unit test can run it
//! against [`crate::platform::fake::FakePlatform`] (AG-903).

use crate::platform::Platform;
use crate::{status_for, StatusDto, ToolDto};
use gate_connect_core::ToolId;
use std::collections::HashMap;

// ---- Tools ----

/// Every tool the UI shows, with its status.
pub(crate) fn list_tools(platform: &impl Platform) -> Vec<ToolDto> {
    // The UI boundary is where hiding happens. The registry itself keeps every
    // integration so the sweep, restore and sign-out paths still clean up a
    // tool someone connected with an earlier build.
    platform
        .integrations()
        .iter()
        .filter(|integ| !integ.hidden_in_ui())
        .map(|integ| ToolDto {
            slug: integ.id().to_string(),
            // The row label, not the product name: this feeds the ledger, whose
            // rows sit under a heading that names the vendor. Every other reader
            // of a tool's name - the CLI, the logs, the quit notice - wants
            // `display_name`, which is why the two are separate.
            name: integ.row_label().to_string(),
            // And the product name beside it, for the readers that are a flat
            // list rather than a grouped ledger: the reopen flow's dialogs and
            // its banner. `registry` has a test asserting display
            // names are distinct precisely because such a list cannot tell two
            // "CLI"s apart.
            product_name: integ.display_name().to_string(),
            upstream_provider_name: integ.upstream_provider_name().to_string(),
            default_upstream_url: integ.default_upstream_url().to_string(),
            config_location: integ.config_location(),
            status: status_for(integ.as_ref()),
            coverage: integ.upstream_coverage(),
            client: integ.client(),
            scope: integ.scope(),
            credential: integ.credential(),
        })
        .collect()
}

/// Every visible tool's version, keyed by slug. Absent where the binary could
/// not be found; null where it was found and would not say.
pub(crate) fn tool_versions(platform: &impl Platform) -> HashMap<String, Option<String>> {
    platform
        .integrations()
        .iter()
        .filter(|integ| !integ.hidden_in_ui())
        .filter_map(|integ| {
            let (well_known, names) = integ.binary();
            if names.is_empty() {
                return None;
            }
            // Resolved here rather than inside the probe so "no binary found"
            // can drop out of the map entirely, while "found, would not say"
            // stays as an explicit null.
            let path = platform.resolve_binary(well_known, names)?;
            Some((integ.id().to_string(), platform.version_at(&path)))
        })
        .collect()
}

/// One tool's status, by slug.
pub(crate) fn tool_status(platform: &impl Platform, slug: &str) -> Result<StatusDto, String> {
    let id = ToolId::from_slug(slug).ok_or_else(|| format!("unknown tool {slug:?}"))?;
    let integ = platform
        .integrations()
        .into_iter()
        .find(|i| i.id() == id)
        .ok_or_else(|| "integration missing from registry".to_string())?;
    Ok(status_for(integ.as_ref()))
}

/// One row per model provider.
pub(crate) fn list_providers(
    platform: &impl Platform,
) -> Vec<gate_connect_core::provider::ProviderState> {
    platform.providers()
}

// ---- This machine ----

/// OS identifier ("macos" / "windows" / "linux") so the UI can tailor copy.
pub(crate) fn app_platform(platform: &impl Platform) -> &'static str {
    platform.os()
}

/// OS marketing name and version, for the analytics error context.
pub(crate) fn os_name(platform: &impl Platform) -> String {
    platform.os_name()
}

/// The backend half of the diagnostics report.
pub(crate) fn diagnostics(platform: &impl Platform) -> gate_connect_core::diagnostics::Diagnostics {
    platform.diagnostics()
}

/// This install's stable id.
pub(crate) fn install_id(platform: &impl Platform) -> Result<String, String> {
    platform.install_id().map_err(|e| format!("{e:#}"))
}

/// Write a webview log line at the level it names. An unknown level is
/// information, never an error (`Level::from_wire`).
pub(crate) fn log_message(platform: &impl Platform, level: &str, message: &str) {
    platform.log(gate_connect_core::logging::Level::from_wire(level), message);
}

// ---- Preferences ----

/// The user's Settings choices.
pub(crate) fn get_preferences(
    platform: &impl Platform,
) -> gate_connect_core::preferences::Preferences {
    platform.preferences()
}

/// What to call this machine.
pub(crate) fn device_name(platform: &impl Platform) -> String {
    platform.device_name()
}

/// Rename this device, or clear the name and follow the hostname again.
pub(crate) fn set_device_name(platform: &impl Platform, name: &str) -> Result<(), String> {
    platform.set_device_name(name).map_err(|e| format!("{e:#}"))
}

/// Turn native notifications on or off.
pub(crate) fn set_notifications(platform: &impl Platform, enabled: bool) -> Result<(), String> {
    platform
        .set_notifications(enabled)
        .map_err(|e| format!("{e:#}"))
}

/// Turn the sound on security notifications on or off.
pub(crate) fn set_security_notification_sound(
    platform: &impl Platform,
    enabled: bool,
) -> Result<(), String> {
    platform
        .set_security_notification_sound(enabled)
        .map_err(|e| format!("{e:#}"))
}

/// Record the provider domains Gate switched on for one tool.
pub(crate) fn record_auto_enabled_domains(
    platform: &impl Platform,
    tool: &str,
    domains: Vec<String>,
) -> Result<(), String> {
    platform
        .record_auto_enabled_domains(tool, domains)
        .map_err(|e| format!("{e:#}"))
}

/// Read what Gate switched on for one tool, without clearing it.
pub(crate) fn read_auto_enabled_domains(platform: &impl Platform, tool: &str) -> Vec<String> {
    platform.read_auto_enabled_domains(tool)
}

// ---- Analytics ----

/// Claim a once-per-install analytics milestone. Any rejection reads as "do
/// not send" in the webview.
pub(crate) fn analytics_milestone_claim(
    platform: &impl Platform,
    name: &str,
) -> Result<bool, String> {
    platform.claim_milestone(name).map_err(|e| format!("{e:#}"))
}

/// Why local Cowork cannot run on this machine, or `None`.
pub(crate) fn cowork_setting_check(platform: &impl Platform) -> Option<&'static str> {
    platform.cowork_setting_missing()
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
    use crate::platform::fake::{FakePlatform, FakeTool};
    use gate_connect_core::logging::Level;
    use gate_connect_core::Status;

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

    fn failing_writes() -> FakePlatform {
        FakePlatform {
            fail_writes: Some("disk full".into()),
            ..Default::default()
        }
    }

    /// The registry keeps hidden tools so cleanup still reaches them; the UI
    /// must not list them.
    #[test]
    fn list_tools_leaves_out_tools_hidden_from_the_ui() {
        let platform = FakePlatform {
            tools: vec![
                FakeTool::new(ToolId::ClaudeCode),
                FakeTool {
                    hidden: true,
                    ..FakeTool::new(ToolId::Codex)
                },
                FakeTool::new(ToolId::Hermes),
            ],
            ..Default::default()
        };
        let slugs: Vec<_> = list_tools(&platform).into_iter().map(|t| t.slug).collect();
        assert_eq!(slugs, ["claude-code", "hermes"]);
    }

    /// A status that cannot be read is a row in the error state, carrying the
    /// whole error chain, not a failed listing.
    #[test]
    fn list_tools_shows_an_unreadable_status_as_an_error_row() {
        let platform = FakePlatform {
            tools: vec![FakeTool {
                status: Err("permission denied".into()),
                ..FakeTool::new(ToolId::OpenCode)
            }],
            ..Default::default()
        };
        let tools = list_tools(&platform);
        assert_eq!(tools.len(), 1);
        assert!(matches!(
            &tools[0].status,
            StatusDto::Error { message } if message == "reading the config: permission denied"
        ));
    }

    /// Not installed drops out of the map; installed but silent stays as null;
    /// a tool with no binary and a hidden tool are never probed.
    #[test]
    fn tool_versions_tells_not_installed_from_unreadable() {
        let platform = FakePlatform {
            tools: vec![
                FakeTool {
                    binaries: &["claude"],
                    ..FakeTool::new(ToolId::ClaudeCode)
                },
                FakeTool {
                    binaries: &["codex"],
                    ..FakeTool::new(ToolId::Codex)
                },
                FakeTool {
                    binaries: &["opencode"],
                    ..FakeTool::new(ToolId::OpenCode)
                },
                FakeTool::new(ToolId::EnvProxy),
                FakeTool {
                    binaries: &["hermes"],
                    hidden: true,
                    ..FakeTool::new(ToolId::Hermes)
                },
            ],
            binaries: [
                ("claude", Some("2.1.0".to_string())),
                ("codex", None),
                ("hermes", Some("0.1".to_string())),
            ]
            .into_iter()
            .collect(),
            ..Default::default()
        };
        let versions = tool_versions(&platform);
        assert_eq!(
            versions,
            HashMap::from([
                ("claude-code".to_string(), Some("2.1.0".to_string())),
                ("codex".to_string(), None),
            ])
        );
    }

    #[test]
    fn tool_status_reads_the_named_tool() {
        let platform = FakePlatform {
            tools: vec![
                FakeTool::new(ToolId::ClaudeCode),
                FakeTool {
                    status: Ok(Status::Drifted("base URL changed".into())),
                    ..FakeTool::new(ToolId::Codex)
                },
            ],
            ..Default::default()
        };
        assert!(matches!(
            tool_status(&platform, "codex"),
            Ok(StatusDto::Drifted { reason }) if reason == "base URL changed"
        ));
    }

    /// An unknown slug and a known tool the registry lacks are two different
    /// messages, so a support thread can tell a typo from a broken build.
    #[test]
    fn tool_status_names_why_it_found_nothing() {
        let platform = FakePlatform {
            tools: vec![FakeTool::new(ToolId::ClaudeCode)],
            ..Default::default()
        };
        assert!(matches!(
            tool_status(&platform, "nope"),
            Err(e) if e == "unknown tool \"nope\""
        ));
        assert!(matches!(
            tool_status(&platform, "codex"),
            Err(e) if e == "integration missing from registry"
        ));
    }

    /// The webview's level names map onto the log's, and anything else is
    /// information rather than an invented error.
    #[test]
    fn log_message_maps_the_wire_level() {
        let platform = FakePlatform::default();
        for level in ["error", "warn", "info", "debug", "ERROR"] {
            log_message(&platform, level, level);
        }
        assert_eq!(
            *platform.logged.lock().unwrap(),
            vec![
                (Level::Error, "error".to_string()),
                (Level::Warn, "warn".to_string()),
                (Level::Info, "info".to_string()),
                (Level::Info, "debug".to_string()),
                (Level::Info, "ERROR".to_string()),
            ]
        );
    }

    #[test]
    fn install_id_reports_the_whole_error_chain() {
        let platform = FakePlatform {
            install_id: Some("abc-123".into()),
            ..Default::default()
        };
        assert_eq!(install_id(&platform), Ok("abc-123".into()));
        assert_eq!(
            install_id(&FakePlatform::default()),
            Err("reading install id: no install id".into())
        );
    }

    #[test]
    fn set_device_name_stores_the_name_or_reports_why_not() {
        let platform = FakePlatform::default();
        assert_eq!(set_device_name(&platform, "Work laptop"), Ok(()));
        assert_eq!(device_name(&platform), "Work laptop");

        let platform = failing_writes();
        assert_eq!(
            set_device_name(&platform, "Work laptop"),
            Err("writing preferences: disk full".into())
        );
        assert_eq!(device_name(&platform), "");
    }

    #[test]
    fn set_security_notification_sound_stores_the_answer_or_reports_why_not() {
        let platform = FakePlatform::default();
        assert_eq!(set_security_notification_sound(&platform, false), Ok(()));
        assert_eq!(*platform.security_sound.lock().unwrap(), Some(false));
        assert_eq!(
            set_security_notification_sound(&failing_writes(), true),
            Err("writing preferences: disk full".into())
        );
    }

    /// What one tool recorded reads back for that tool only.
    #[test]
    fn auto_enabled_domains_read_back_per_tool() {
        let platform = FakePlatform::default();
        assert_eq!(
            record_auto_enabled_domains(&platform, "codex", vec!["api.openai.com".into()]),
            Ok(())
        );
        assert_eq!(
            read_auto_enabled_domains(&platform, "codex"),
            ["api.openai.com"]
        );
        assert!(read_auto_enabled_domains(&platform, "hermes").is_empty());
        assert_eq!(
            record_auto_enabled_domains(&failing_writes(), "codex", Vec::new()),
            Err("writing preferences: disk full".into())
        );
    }

    /// True once per name, and a failure is an error the webview reads as "do
    /// not send", never a second true.
    #[test]
    fn analytics_milestone_claim_is_true_once() {
        let platform = FakePlatform::default();
        assert_eq!(
            analytics_milestone_claim(&platform, "first_route"),
            Ok(true)
        );
        assert_eq!(
            analytics_milestone_claim(&platform, "first_route"),
            Ok(false)
        );
        assert_eq!(
            analytics_milestone_claim(&failing_writes(), "first_route"),
            Err("writing preferences: disk full".into())
        );
    }
}
