//! Windows Cowork implementation.
//!
//! Claude Desktop on Windows reads its 3P / gateway configuration from the
//! `HKCU\SOFTWARE\Policies\Claude` registry policy (the documented
//! per-user enterprise-config hive), not from a managed-preferences plist.
//!
//! Like macOS, secrets never touch the config surface: they stay in the
//! Windows Credential Manager (DPAPI-backed, via the `keyring` crate) and
//! are resolved at request time by a credential helper. The registry holds
//! only non-secret config plus the absolute path of that helper:
//!
//! - `inferenceProvider = gateway`, `inferenceGatewayBaseUrl`,
//!   `inferenceGatewayAuthScheme = bearer` — plain routing config.
//! - `inferenceCredentialKind = helper-script` — tells Claude Desktop to
//!   resolve the credential by running the helper rather than reading a
//!   static `inferenceGatewayApiKey`. Without it Claude's config validator
//!   never selects the helper path and the gateway call goes out unauthed.
//! - `inferenceCredentialHelper` — absolute path to
//!   `gate-connect-cowork-helper.exe`. Claude spawns it at session start,
//!   on TTL expiry, and on an upstream 401; it reads the helper's stdout.
//! - `inferenceCredentialHelperTtlSec` — how long Claude caches the
//!   helper's output before re-running it.
//!
//! At request time the helper ([`helper_emit`]) reads the upstream
//! credential and the Gate API key from Credential Manager and emits the
//! `{"token","headers"}` JSON Claude expects — the same shape the macOS
//! helper script prints. The only on-disk artifact `connect` writes besides
//! the registry is a **non-secret** sidecar JSON holding the upstream URL,
//! so the helper knows which `X-Gate-Upstream-Url` to emit.
//!
//! Claude Desktop reads the policy once at launch — the user must fully
//! quit and relaunch for config changes to take effect. The helper itself
//! is re-run on the cadence above, so rotating the stored credential is
//! picked up without a relaunch.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ, KEY_SET_VALUE};
use winreg::RegKey;

use crate::account;
use crate::env;
use crate::keychain;
use crate::primitives;
use crate::registry::{ConnectInput, Status};

use super::upstream_service;

const POLICY_SUBKEY: &str = r"SOFTWARE\Policies\Claude";

/// Filename of the bundled credential helper, resolved next to the running
/// executable (the Tauri app bundles it as a sidecar). Overridable via the
/// `GATE_CONNECT_COWORK_HELPER` env var for dev / CLI use.
const HELPER_EXE_NAME: &str = "gate-connect-cowork-helper.exe";
const HELPER_ENV_OVERRIDE: &str = "GATE_CONNECT_COWORK_HELPER";

/// Claude caches helper output this long (seconds) before re-running it.
/// Stored as a string: these policy values are all REG_SZ. Matches the
/// macOS helper TTL.
const HELPER_TTL_SECONDS: &str = "3000";

const HELPER_CONFIG_FILENAME: &str = "cowork-helper.json";

/// Values `connect` writes. `disconnect` removes these.
const WRITTEN_VALUES: &[&str] = &[
    "inferenceProvider",
    "inferenceCredentialKind",
    "inferenceGatewayBaseUrl",
    "inferenceGatewayAuthScheme",
    "inferenceCredentialHelper",
    "inferenceCredentialHelperTtlSec",
];

/// Values an earlier plaintext-mode `connect` may have written. Cleaned up
/// on both connect and disconnect so upgrades don't leave a stale key.
const LEGACY_VALUES: &[&str] = &["inferenceGatewayApiKey", "inferenceCustomHeaders"];

/// Non-secret sidecar the helper reads to learn the upstream URL. Service
/// names and the account are derived deterministically by the helper, so
/// the only per-connect datum here is the URL — no secrets on disk.
#[derive(Serialize, Deserialize)]
struct HelperConfig {
    upstream_url: String,
}

fn helper_config_path() -> Result<PathBuf> {
    Ok(env::app_support_dir()?.join(HELPER_CONFIG_FILENAME))
}

/// Absolute path to the credential helper exe. Honors
/// `GATE_CONNECT_COWORK_HELPER`, else looks next to the running executable
/// (where the Tauri sidecar lands after bundling).
fn helper_exe_path() -> Result<PathBuf> {
    if let Ok(p) = std::env::var(HELPER_ENV_OVERRIDE) {
        if !p.is_empty() {
            return Ok(PathBuf::from(p));
        }
    }
    let exe = std::env::current_exe().context("resolving current executable path")?;
    let dir = exe
        .parent()
        .context("current executable has no parent directory")?;
    Ok(dir.join(HELPER_EXE_NAME))
}

pub fn detect() -> Result<bool> {
    // 1. Known filesystem install / userData locations.
    if env::claude_desktop_path_candidates()
        .iter()
        .any(|p| p.exists())
    {
        return Ok(true);
    }
    // 2. MSIX / Microsoft Store install. The app itself lives under
    //    `C:\Program Files\WindowsApps\Claude_…`, but that directory is
    //    ACL-locked and not enumerable by a normal process. The readable
    //    signal is the per-user package-data dir created on first run:
    //    `%LOCALAPPDATA%\Packages\<PackageFamilyName>`. We still probe
    //    WindowsApps best-effort in case ACLs allow it.
    if let Some(local) = dirs::data_local_dir() {
        if dir_has_claude_entry(&local.join("Packages")) {
            return Ok(true);
        }
    }
    for var in ["ProgramFiles", "ProgramW6432", "ProgramFiles(x86)"] {
        if let Ok(pf) = std::env::var(var) {
            if dir_has_claude_entry(&Path::new(&pf).join("WindowsApps")) {
                return Ok(true);
            }
        }
    }
    // 3. AppX package repository + classic Uninstall registry entries.
    Ok(appx_repository_mentions_claude() || uninstall_entry_mentions_claude())
}

/// True if `dir` contains an entry whose name mentions "claude"
/// (case-insensitive). Any read failure (missing dir, ACL denial) is
/// treated as "no match".
fn dir_has_claude_entry(dir: &Path) -> bool {
    match std::fs::read_dir(dir) {
        Ok(entries) => entries.flatten().any(|e| {
            e.file_name()
                .to_string_lossy()
                .to_lowercase()
                .contains("claude")
        }),
        Err(_) => false,
    }
}

/// Scan the MSIX/AppX package repository for a Claude package. This is the
/// machine-wide registry the OS uses to track installed Store apps, so it
/// catches a Store install even before first launch (no `Packages` dir yet).
fn appx_repository_mentions_claude() -> bool {
    const REPO: &str = r"SOFTWARE\Microsoft\Windows\CurrentVersion\AppModel\Repository\Packages";
    for hive in [HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE] {
        let Ok(root) = RegKey::predef(hive).open_subkey(REPO) else {
            continue;
        };
        if root
            .enum_keys()
            .flatten()
            .any(|k| k.to_lowercase().contains("claude"))
        {
            return true;
        }
    }
    false
}

/// Scan the Add/Remove Programs registry for an entry whose DisplayName
/// mentions Claude. Best-effort: any read failure is treated as "absent".
fn uninstall_entry_mentions_claude() -> bool {
    const UNINSTALL: &str = r"Software\Microsoft\Windows\CurrentVersion\Uninstall";
    for hive in [HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE] {
        let Ok(root) = RegKey::predef(hive).open_subkey(UNINSTALL) else {
            continue;
        };
        for name in root.enum_keys().flatten() {
            let Ok(sub) = root.open_subkey(&name) else {
                continue;
            };
            let display: String = sub.get_value("DisplayName").unwrap_or_default();
            if display.to_lowercase().contains("claude") {
                return true;
            }
        }
    }
    false
}

pub fn status() -> Result<Status> {
    if !detect()? {
        return Ok(Status::NotInstalled);
    }
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let key = match hkcu.open_subkey(POLICY_SUBKEY) {
        Ok(k) => k,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Status::Detected),
        Err(e) => return Err(e).context(r"opening HKCU\SOFTWARE\Policies\Claude"),
    };

    let provider: String = key.get_value("inferenceProvider").unwrap_or_default();
    if provider.is_empty() {
        return Ok(Status::Detected);
    }
    if provider != "gateway" {
        return Ok(Status::Drifted(format!(
            "inferenceProvider is {provider:?}, expected \"gateway\""
        )));
    }
    let base_url: String = key.get_value("inferenceGatewayBaseUrl").unwrap_or_default();
    if base_url.is_empty() {
        return Ok(Status::Drifted("inferenceGatewayBaseUrl is empty".into()));
    }
    let helper: String = key
        .get_value("inferenceCredentialHelper")
        .unwrap_or_default();
    if helper.is_empty() {
        return Ok(Status::Drifted("inferenceCredentialHelper is empty".into()));
    }
    if !PathBuf::from(&helper).exists() {
        return Ok(Status::Drifted(format!(
            "credential helper missing at {helper}"
        )));
    }
    if !helper_config_path()?.exists() {
        return Ok(Status::Drifted("helper config missing".into()));
    }
    let user = env::current_user()?;
    if keychain::get(&account::service(), &user)?.is_none() {
        return Ok(Status::Drifted(
            "credential-manager entry for gate-api-key missing".into(),
        ));
    }
    if keychain::get(&upstream_service(), &user)?.is_none() {
        return Ok(Status::Drifted(
            "credential-manager entry for upstream credential missing".into(),
        ));
    }
    Ok(Status::Connected)
}

pub fn connect(input: &ConnectInput) -> Result<()> {
    if !detect()? {
        anyhow::bail!("Claude Desktop is not installed; install from claude.com/download first");
    }
    if !input.gateway_base_url.starts_with("https://") {
        anyhow::bail!("gateway base URL must be https://");
    }
    if !input.upstream_url.starts_with("https://") {
        anyhow::bail!("upstream URL must be https://");
    }
    // Fail fast if either secret is missing — the helper would otherwise
    // error at request time. We do not copy them anywhere; they stay in
    // Credential Manager and the helper reads them live.
    let user = env::current_user()?;
    if keychain::get(&upstream_service(), &user)?.is_none() {
        anyhow::bail!("no upstream Anthropic credential saved — paste an API key first");
    }
    if keychain::get(&account::service(), &user)?.is_none() {
        anyhow::bail!("not signed in to Gate AI");
    }

    let helper = helper_exe_path()?;
    if !helper.exists() {
        anyhow::bail!(
            "credential helper not found at {} (set {HELPER_ENV_OVERRIDE} to override)",
            helper.display()
        );
    }

    // Non-secret sidecar: just the upstream URL.
    let config = serde_json::to_vec_pretty(&HelperConfig {
        upstream_url: input.upstream_url.clone(),
    })
    .context("serializing helper config")?;
    primitives::write_file(&helper_config_path()?, &config, 0o600)
        .context("writing helper config")?;

    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let (key, _) = hkcu
        .create_subkey(POLICY_SUBKEY)
        .context(r"creating HKCU\SOFTWARE\Policies\Claude")?;
    key.set_value("inferenceProvider", &"gateway")
        .context("writing inferenceProvider")?;
    key.set_value("inferenceCredentialKind", &"helper-script")
        .context("writing inferenceCredentialKind")?;
    key.set_value("inferenceGatewayBaseUrl", &input.gateway_base_url)
        .context("writing inferenceGatewayBaseUrl")?;
    key.set_value("inferenceGatewayAuthScheme", &"bearer")
        .context("writing inferenceGatewayAuthScheme")?;
    key.set_value("inferenceCredentialHelper", &helper.display().to_string())
        .context("writing inferenceCredentialHelper")?;
    key.set_value("inferenceCredentialHelperTtlSec", &HELPER_TTL_SECONDS)
        .context("writing inferenceCredentialHelperTtlSec")?;

    // Remove any plaintext values a previous (pre-helper) connect wrote.
    for value in LEGACY_VALUES {
        let _ = key.delete_value(value);
    }
    Ok(())
}

pub fn disconnect() -> Result<()> {
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    match hkcu.open_subkey_with_flags(POLICY_SUBKEY, KEY_READ | KEY_SET_VALUE) {
        Ok(key) => {
            for value in WRITTEN_VALUES.iter().chain(LEGACY_VALUES) {
                match key.delete_value(value) {
                    Ok(()) => {}
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                    Err(e) => return Err(e).with_context(|| format!("deleting {value}")),
                }
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e).context(r"opening HKCU\SOFTWARE\Policies\Claude"),
    }

    let config_path = helper_config_path()?;
    if config_path.exists() {
        std::fs::remove_file(&config_path)
            .with_context(|| format!("removing {}", config_path.display()))?;
    }
    // Credential Manager entries stay — use sign out / clear to wipe them.
    Ok(())
}

/// Produce the `{"token","headers"}` JSON Claude Desktop expects on the
/// helper's stdout. Reads the upstream credential and Gate key from
/// Credential Manager and the upstream URL from the sidecar config. Mirrors
/// the macOS helper script's output exactly.
pub fn helper_emit() -> Result<String> {
    let raw = std::fs::read_to_string(helper_config_path()?)
        .context("reading helper config (is Cowork connected?)")?;
    let config: HelperConfig = serde_json::from_str(&raw).context("parsing helper config")?;

    let user = env::current_user()?;
    let stored = keychain::get(&upstream_service(), &user)?
        .context("upstream credential missing from Credential Manager")?;
    // The sentinel means "delegate to the live Claude Code session": read the
    // current access token from Claude Code's credentials file on every request
    // so we trail its refreshes. Any other value is a literal credential.
    let upstream = if stored == crate::claude_session_delegate::CLAUDE_CODE_SENTINEL {
        let raw = crate::claude_session_delegate::read_claude_credentials(&user).context(
      "Claude Code session unavailable — open Claude Code and sign in, or reconnect Cowork with a pasted token instead.",
    )?;
        crate::claude_session_delegate::parse_json_access_token(&raw)
            .context("Claude Code credentials file has no claudeAiOauth.accessToken")?
    } else {
        stored
    };
    let gate_key = keychain::get(&account::service(), &user)?
        .context("Gate API key missing from Credential Manager")?;

    let headers = build_headers_map(&upstream, &gate_key, &config.upstream_url);
    let out = serde_json::json!({
      "token": upstream,
      "headers": serde_json::Value::Object(headers),
    });
    Ok(out.to_string())
}

/// Build the helper's `headers` object. Mirrors the per-credential-kind
/// logic in the macOS helper: OAuth tokens get `anthropic-beta` and no
/// `X-Api-Key`; API keys dual-emit `X-Api-Key`.
fn build_headers_map(
    upstream: &str,
    gate_key: &str,
    upstream_url: &str,
) -> serde_json::Map<String, serde_json::Value> {
    let mut map = serde_json::Map::new();
    map.insert(
        "X-Gate-Api-Key".into(),
        serde_json::Value::String(gate_key.to_string()),
    );
    map.insert(
        "X-Gate-Upstream-Url".into(),
        serde_json::Value::String(upstream_url.to_string()),
    );
    if upstream.starts_with("sk-ant-oat") {
        map.insert(
            "anthropic-beta".into(),
            serde_json::Value::String("oauth-2025-04-20".into()),
        );
    } else {
        map.insert(
            "X-Api-Key".into(),
            serde_json::Value::String(upstream.to_string()),
        );
    }
    map
}
