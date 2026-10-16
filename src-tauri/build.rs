/// Every application command `invoke_handler` registers on desktop, in the same
/// order. Naming them here is what turns the app ACL on: tauri-build generates
/// an `allow-<command>` permission for each, and once the app has a manifest a
/// window can only call the commands its capability grants. Without one, every
/// window (the tray popover included) could call every command, and the
/// capability files only ever narrowed the plugin APIs. Which window gets which
/// command is in `capabilities/main-commands.json`, `onboarding-commands.json`
/// and `tray-commands.json`; `src/lib/capabilities.test.ts` keeps this list in
/// step with `generate_handler!` and each window's grants in step with its code.
const APP_COMMANDS: &[&str] = &[
    "list_tools",
    "tool_versions",
    "tool_status",
    "connect_tool",
    "disconnect_tool",
    "get_account",
    "get_account_key_prefix",
    "save_account",
    "clear_account",
    "switch_gateway",
    "oauth_begin_login",
    "oauth_cancel_login",
    "oauth_status",
    "oauth_sign_out",
    "set_auth_mode",
    "set_billing_mode",
    "oauth_list_orgs",
    "activity_overview",
    "activity_installations",
    "activity_cached_overview",
    "activity_cached_tool_overviews",
    "activity_tool_events",
    "tool_model_preferences",
    "set_tool_model",
    "gate_model_catalogue",
    "gate_credits",
    "log_message",
    "set_org",
    "app_platform",
    "os_name",
    "diagnostics",
    "unpin_popover",
    "pin_popover",
    "open_onboarding_window",
    "reveal_popover",
    "request_switch_org",
    "quit_app",
    "list_providers",
    "proxy_status",
    "proxy_browser_store",
    "proxy_enable",
    "proxy_disable",
    "proxy_set_domain",
    "hermes_upstream_coverage",
    "proxy_set_env_export",
    "proxy_trust_ca",
    "proxy_untrust_ca",
    "launch_at_login_status",
    "set_launch_at_login",
    "get_preferences",
    "set_notifications",
    "set_share_diagnostics",
    "analytics_milestone_claim",
    "cowork_setting_check",
    "analytics_identity",
    "set_analytics_identity",
    "record_auto_enabled_domains",
    "read_auto_enabled_domains",
    "install_id",
    "device_name",
    "set_device_name",
    "set_updater_relaunching",
    "routed_clients_stale",
    "routing_startup_pending",
    "routing_verdicts",
    "teardown_report",
    "running_agents",
    "close_running_agents",
    "reopen_running_agents",
    "drain_backend_errors",
    "security_feed_state",
    "security_feed_history_ok",
    "security_feed_recent",
    "security_feed_retry",
    "set_security_notification_sound",
];

fn main() {
    tauri_build::try_build(
        tauri_build::Attributes::new()
            .app_manifest(tauri_build::AppManifest::new().commands(APP_COMMANDS)),
    )
    .expect("failed to run tauri-build");
    embed_windows_manifest_for_examples();
}

/// Give the package's examples the application manifest tauri-build gives its
/// binaries.
///
/// tauri-build embeds a manifest declaring a dependency on Common Controls 6.0
/// through embed-resource, which links it with `rustc-link-arg-bins`: binaries
/// only. `examples/ui-harness.rs` is an example target, so on Windows it was
/// linked with no manifest, and a process without one is handed the 5.82
/// comctl32 from System32. The desktop library imports `TaskDialogIndirect`
/// (tauri-runtime-wry's dialog), which only the 6.0 assembly exports, so the
/// loader refused the exe before `main` with STATUS_ENTRYPOINT_NOT_FOUND - seen
/// from Git Bash as a bare exit 127, from Playwright as "Process from
/// config.webServer was not able to start". The main binary never hit it
/// because it has the manifest; the crate's tests never hit it because nothing
/// they call keeps the import alive.
///
/// MSVC only: `/MANIFEST:EMBED` is a link.exe flag. The GNU toolchain would
/// need the manifest compiled into a resource instead, and nothing builds this
/// crate that way.
fn embed_windows_manifest_for_examples() {
    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let target_env = std::env::var("CARGO_CFG_TARGET_ENV").unwrap_or_default();
    if target_os != "windows" || target_env != "msvc" {
        return;
    }
    let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("examples")
        .join("ui-harness.manifest");
    println!("cargo:rerun-if-changed={}", manifest.display());
    println!("cargo:rustc-link-arg-examples=/MANIFEST:EMBED");
    println!(
        "cargo:rustc-link-arg-examples=/MANIFESTINPUT:{}",
        manifest.display()
    );
}
