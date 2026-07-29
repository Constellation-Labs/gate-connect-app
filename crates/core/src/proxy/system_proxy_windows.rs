//! Windows system HTTP/HTTPS proxy wiring via the per-user WinINET settings in
//! the registry (`HKCU\Software\Microsoft\Windows\CurrentVersion\Internet
//! Settings`). Enabling points WinINET at the engine's loopback PAC
//! (`AutoConfigURL`) so only Gate's intercepted hosts route to us and all other
//! traffic stays DIRECT; disabling restores the exact prior state from a
//! snapshot (mirrors the `previousEnv` restore pattern in [`crate::env`], and
//! the macOS `system_proxy` snapshot/restore). Everything here lives under
//! `HKCU`, so - like the macOS `networksetup` path - none of it needs admin;
//! the only privileged step in the subsystem is trusting the CA.
//!
//! WinINET caches the proxy config, so after every registry change we poke it
//! with `InternetSetOption(INTERNET_OPTION_SETTINGS_CHANGED + _REFRESH)` -
//! otherwise already-running apps keep using the stale settings until the next
//! logon.
//!
//! The PAC only steers apps that honor the system proxy. CLI tools - Node-based
//! ones especially (the Gemini CLI) - read the `HTTP(S)_PROXY` env vars and
//! trust only `NODE_EXTRA_CA_CERTS` instead, so enabling also injects those as
//! per-user env vars under `HKCU\Environment` (the Windows analog of the macOS
//! `~/.zshenv` block / Linux `environment.d` drop-in), broadcast via
//! `WM_SETTINGCHANGE` so new terminals pick them up. The snapshot captures the
//! prior values so disable puts back exactly what was there. Note this is
//! all-or-nothing for CLI processes - unlike the PAC, an env-var proxy routes
//! *all* their traffic through the engine (non-intercepted hosts are
//! blind-tunnelled), matching the Linux model.

use std::collections::BTreeMap;
use std::ffi::c_void;
use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use winreg::enums::{HKEY_CURRENT_USER, KEY_READ, KEY_SET_VALUE};
use winreg::RegKey;

use crate::env;

const INTERNET_SETTINGS: &str = r"Software\Microsoft\Windows\CurrentVersion\Internet Settings";

/// Per-user environment block, read (merged under the machine block) into the
/// environment of every process the user starts afterwards.
const USER_ENV: &str = r"Environment";

/// The env vars we own under `HKCU\Environment` while the proxy is on.
/// `HTTP(S)_PROXY` point CLIs at the engine; `NO_PROXY` keeps loopback traffic
/// off the proxy; `NODE_EXTRA_CA_CERTS` makes Node trust the engine's minted
/// leaf certs (it ships its own bundle and ignores the system store). Windows
/// env names are case-insensitive, so no lowercase aliases are needed.
const ENV_VARS: [&str; 4] = [
    "HTTP_PROXY",
    "HTTPS_PROXY",
    "NO_PROXY",
    "NODE_EXTRA_CA_CERTS",
];

// INTERNET_OPTION_* constants from wininet.h.
const INTERNET_OPTION_REFRESH: u32 = 37;
const INTERNET_OPTION_SETTINGS_CHANGED: u32 = 39;

// Broadcast constants from winuser.h, used to tell Explorer (and anything else
// listening) that the user environment changed - otherwise new terminals
// launched from the shell keep the stale environment until the next logon.
const HWND_BROADCAST: usize = 0xffff;
const WM_SETTINGCHANGE: u32 = 0x001A;
const SMTO_ABORTIFHUNG: u32 = 0x0002;

#[link(name = "wininet")]
extern "system" {
    fn InternetSetOptionW(
        h_internet: *mut c_void,
        dw_option: u32,
        lp_buffer: *mut c_void,
        dw_buffer_length: u32,
    ) -> i32;
}

#[link(name = "user32")]
extern "system" {
    fn SendMessageTimeoutW(
        hwnd: *mut c_void,
        msg: u32,
        wparam: usize,
        lparam: *const u16,
        flags: u32,
        timeout: u32,
        result: *mut usize,
    ) -> isize;
}

/// Snapshot of the user's WinINET proxy configuration - the three values we
/// touch, so `restore` can put them back exactly.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProxySnapshot {
    /// `ProxyEnable` (0 / 1).
    pub enable: u32,
    /// `ProxyServer` (e.g. `127.0.0.1:8080`, or `http=...;https=...`).
    pub server: String,
    /// `ProxyOverride` - the bypass list (e.g. `<local>`).
    pub bypass: String,
    /// `AutoConfigURL` - the PAC URL, empty when unset. Defaulted so a
    /// snapshot written by an older build (before PAC mode) still loads.
    #[serde(default)]
    pub auto_config_url: String,
    /// Prior values of the `HKCU\Environment` vars we inject ([`ENV_VARS`]):
    /// present keys are re-set on restore, absent keys are deleted. `None`
    /// (the `serde(default)` for a snapshot written by an older build) means
    /// that build never injected env vars, so restore leaves the user
    /// environment untouched rather than deleting vars it never owned.
    #[serde(default)]
    pub prior_env: Option<BTreeMap<String, String>>,
}

fn snapshot_path() -> Result<PathBuf> {
    Ok(env::app_support_dir()?
        .join("proxy")
        .join("system-proxy.snapshot.json"))
}

/// The engine's chosen loopback port persists (via [`super::port_persist`])
/// so it can be reused across restarts. WinINET consumers re-resolve after
/// our `INTERNET_OPTION_SETTINGS_CHANGED` poke, but clients that resolve the
/// proxy once at their own launch (e.g. Electron apps doing Node-side
/// requests) keep dialing the old port after we restart - a stable port keeps
/// them working without a client restart.
pub fn load_port() -> Result<Option<u16>> {
    super::port_persist::load("port")
}

/// Persist the engine port for reuse on the next run (see [`load_port`]).
pub fn save_port(port: u16) -> Result<()> {
    super::port_persist::save("port", port)
}

/// The PAC listener's port persists too, companion to [`load_port`]: the
/// `AutoConfigURL` bakes this port in, and a client that captured that URL
/// at its own launch must find a fresh PAC there after we restart, or its PAC
/// fetch fails and it silently falls back to DIRECT (bypassing Gate).
pub fn load_pac_port() -> Result<Option<u16>> {
    super::port_persist::load("pac-port")
}

/// Persist the PAC port for reuse on the next run (see [`load_pac_port`]).
pub fn save_pac_port(port: u16) -> Result<()> {
    super::port_persist::save("pac-port", port)
}

fn settings_key(write: bool) -> Result<RegKey> {
    let access = if write {
        KEY_READ | KEY_SET_VALUE
    } else {
        KEY_READ
    };
    RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey_with_flags(INTERNET_SETTINGS, access)
        .with_context(|| format!("opening HKCU\\{INTERNET_SETTINGS}"))
}

/// Read the current WinINET proxy config. A missing value is its empty/off
/// default rather than an error - a machine that has never had a proxy set
/// simply has no `ProxyServer` value. Non-privileged.
pub fn snapshot() -> Result<ProxySnapshot> {
    let key = settings_key(false)?;
    Ok(ProxySnapshot {
        enable: key.get_value("ProxyEnable").unwrap_or(0),
        server: key.get_value("ProxyServer").unwrap_or_default(),
        bypass: key.get_value("ProxyOverride").unwrap_or_default(),
        auto_config_url: key.get_value("AutoConfigURL").unwrap_or_default(),
        prior_env: Some(read_user_env()?),
    })
}

pub fn save_snapshot(snapshot: &ProxySnapshot) -> Result<()> {
    let path = snapshot_path()?;
    let raw = serde_json::to_string_pretty(snapshot).context("serializing proxy snapshot")?;
    // Atomic write (handles parent dirs too): a torn snapshot would make
    // disable/reconcile fall back to force-off instead of an exact restore.
    crate::primitives::write_file(&path, raw.as_bytes(), 0o600)
        .with_context(|| format!("writing {}", path.display()))
}

pub fn load_snapshot() -> Result<Option<ProxySnapshot>> {
    let path = snapshot_path()?;
    match fs::read_to_string(&path) {
        Ok(raw) => {
            Ok(Some(serde_json::from_str(&raw).with_context(|| {
                format!("parsing {} as JSON", path.display())
            })?))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e).with_context(|| format!("reading {}", path.display())),
    }
}

pub fn clear_snapshot() -> Result<()> {
    let path = snapshot_path()?;
    match fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e).with_context(|| format!("removing {}", path.display())),
    }
}

/// Tell WinINET to reload proxy settings from the registry. Without this,
/// already-running apps keep their cached config until the next logon.
fn notify_wininet() {
    // SAFETY: both calls pass a null handle and null buffer - the documented
    // way to broadcast a global settings change. There are no out-parameters.
    unsafe {
        InternetSetOptionW(
            std::ptr::null_mut(),
            INTERNET_OPTION_SETTINGS_CHANGED,
            std::ptr::null_mut(),
            0,
        );
        InternetSetOptionW(
            std::ptr::null_mut(),
            INTERNET_OPTION_REFRESH,
            std::ptr::null_mut(),
            0,
        );
    }
}

fn set_values(enable: u32, server: &str, bypass: &str) -> Result<()> {
    let key = settings_key(true)?;
    key.set_value("ProxyEnable", &enable)
        .context("setting ProxyEnable")?;
    key.set_value("ProxyServer", &server.to_string())
        .context("setting ProxyServer")?;
    key.set_value("ProxyOverride", &bypass.to_string())
        .context("setting ProxyOverride")?;
    notify_wininet();
    Ok(())
}

/// Set (or, when empty, delete) `AutoConfigURL` and poke WinINET. A missing
/// value is the "no PAC" state, so restoring an empty snapshot deletes it
/// rather than leaving an empty string WinINET would try to fetch.
fn set_auto_config(url: &str) -> Result<()> {
    let key = settings_key(true)?;
    if url.is_empty() {
        match key.delete_value("AutoConfigURL") {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e).context("clearing AutoConfigURL"),
        }
    } else {
        key.set_value("AutoConfigURL", &url.to_string())
            .context("setting AutoConfigURL")?;
    }
    notify_wininet();
    Ok(())
}

/// Path to our CA cert, mirrored from [`super::ca`] - used for
/// `NODE_EXTRA_CA_CERTS` so Node CLIs trust the engine's leaf certs.
fn ca_cert_path() -> Result<PathBuf> {
    Ok(env::app_support_dir()?.join("proxy").join("ca-cert.pem"))
}

fn env_key(write: bool) -> Result<RegKey> {
    let access = if write {
        KEY_READ | KEY_SET_VALUE
    } else {
        KEY_READ
    };
    RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey_with_flags(USER_ENV, access)
        .with_context(|| format!("opening HKCU\\{USER_ENV}"))
}

/// Tell Explorer (and anything else listening) that the user environment
/// changed, so terminals launched from the shell afterwards see the new vars.
/// Already-running processes keep their environment until relaunched.
fn broadcast_env_change() {
    let param: Vec<u16> = "Environment\0".encode_utf16().collect();
    let mut result: usize = 0;
    // SAFETY: the documented broadcast of an environment change - a valid
    // NUL-terminated wide string and a writable result out-param; the message
    // carries no other buffers. `param` outlives the call.
    unsafe {
        SendMessageTimeoutW(
            HWND_BROADCAST as *mut c_void,
            WM_SETTINGCHANGE,
            0,
            param.as_ptr(),
            SMTO_ABORTIFHUNG,
            5000,
            &mut result,
        );
    }
}

/// Current values of the env vars we manage ([`ENV_VARS`]). A missing var is
/// simply absent from the map, not an error. Non-privileged.
fn read_user_env() -> Result<BTreeMap<String, String>> {
    let key = env_key(false)?;
    let mut map = BTreeMap::new();
    for var in ENV_VARS {
        if let Ok(value) = key.get_value::<String, _>(var) {
            map.insert(var.to_string(), value);
        }
    }
    Ok(map)
}

fn delete_env_value(key: &RegKey, var: &str) -> Result<()> {
    match key.delete_value(var) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e).with_context(|| format!("deleting {var}")),
    }
}

/// Put the managed env vars back exactly as `prior` recorded them: re-set the
/// ones that existed, delete the ones we added. Broadcasts the change.
fn restore_user_env(prior: &BTreeMap<String, String>) -> Result<()> {
    let key = env_key(true)?;
    for var in ENV_VARS {
        match prior.get(var) {
            Some(value) => key
                .set_value(var, value)
                .with_context(|| format!("restoring {var}"))?,
            None => delete_env_value(&key, var)?,
        }
    }
    broadcast_env_change();
    Ok(())
}

/// Delete every managed env var. Fail-safe used when no snapshot is available,
/// so new CLI processes never inherit a proxy pointed at a dead engine.
/// Broadcasts the change.
fn clear_user_env() -> Result<()> {
    let key = env_key(true)?;
    for var in ENV_VARS {
        delete_env_value(&key, var)?;
    }
    broadcast_env_change();
    Ok(())
}

/// Inject the proxy + CA env vars for CLI tools into `HKCU\Environment` and
/// broadcast the change. The env-var counterpart of [`enable_pac`]: WinINET
/// only steers apps that honor the system proxy, while CLI tools (Node-based
/// ones especially) read `HTTP(S)_PROXY` / `NODE_EXTRA_CA_CERTS` instead.
/// Only processes started after the broadcast see the vars; open terminals
/// keep their environment until relaunched. Promptless (HKCU).
pub fn enable_env(port: u16) -> Result<()> {
    let endpoint = format!("http://127.0.0.1:{port}");
    let ca = ca_cert_path()?.display().to_string();
    let key = env_key(true)?;
    key.set_value("HTTP_PROXY", &endpoint)
        .context("setting HTTP_PROXY")?;
    key.set_value("HTTPS_PROXY", &endpoint)
        .context("setting HTTPS_PROXY")?;
    key.set_value("NO_PROXY", &"localhost,127.0.0.1,::1".to_string())
        .context("setting NO_PROXY")?;
    key.set_value("NODE_EXTRA_CA_CERTS", &ca)
        .context("setting NODE_EXTRA_CA_CERTS")?;
    broadcast_env_change();
    Ok(())
}

/// The user's pre-existing upstream proxy as a `host:port` string suitable for
/// a PAC `PROXY` directive, so non-Gate traffic keeps flowing through it while
/// routing is on. Returns `None` (PAC falls back to DIRECT) when there was no
/// enabled static proxy, or when the prior config was itself a PAC or a
/// non-HTTP proxy this can't express. For the per-protocol `ProxyServer` form
/// (`http=..;https=..`) the `https=` entry wins, then `http=`.
pub fn upstream_proxy(snapshot: &ProxySnapshot) -> Option<String> {
    if snapshot.enable == 0 {
        return None;
    }
    let server = snapshot.server.trim();
    if server.is_empty() {
        return None;
    }
    if !server.contains('=') {
        return Some(server.to_string());
    }
    let scheme = |prefix: &str| {
        server
            .split(';')
            .map(str::trim)
            .find_map(|entry| entry.strip_prefix(prefix))
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
    };
    scheme("https=").or_else(|| scheme("http="))
}

/// Point WinINET at the engine's loopback PAC. The PAC sends only Gate's
/// intercepted hosts to the proxy and everything else DIRECT, so unrelated
/// traffic (Teams, other apps) never traverses the loopback engine. The static
/// proxy is turned off so it can't compete with the PAC. Promptless (HKCU).
pub fn enable_pac(pac_url: &str) -> Result<()> {
    let key = settings_key(true)?;
    key.set_value("ProxyEnable", &0u32)
        .context("clearing ProxyEnable")?;
    key.set_value("AutoConfigURL", &pac_url.to_string())
        .context("setting AutoConfigURL")?;
    notify_wininet();
    Ok(())
}

/// Restore the user's proxy config from a snapshot. Promptless; safety-critical.
pub fn restore(snapshot: &ProxySnapshot) -> Result<()> {
    set_values(snapshot.enable, &snapshot.server, &snapshot.bypass)?;
    set_auto_config(&snapshot.auto_config_url)?;
    // A pre-upgrade snapshot (None) was written by a build that never injected
    // env vars, so there is nothing of ours to undo - leave the user's
    // environment alone rather than deleting vars we never owned.
    snapshot
        .prior_env
        .as_ref()
        .map_or(Ok(()), |prior| restore_user_env(prior))
}

/// Turn the WinINET proxy off (leaving the server string intact), clear any
/// PAC URL we set, and delete the managed env vars. Promptless fail-safe used
/// when no snapshot is available, so a dead engine never strands traffic at
/// our proxy, a dead PAC, or a dead-port env proxy.
pub fn force_off() -> Result<()> {
    let key = settings_key(true)?;
    key.set_value("ProxyEnable", &0u32)
        .context("setting ProxyEnable")?;
    notify_wininet();
    set_auto_config("")?;
    clear_user_env()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pre_upgrade_snapshot_without_prior_env_loads_as_none() {
        // A snapshot written before the env-var injection existed has no
        // `prior_env` key; it must load as None (leave the environment
        // untouched on restore), not as an empty map (delete everything).
        let raw = r#"{ "enable": 1, "server": "proxy:8080", "bypass": "<local>" }"#;
        let snapshot: ProxySnapshot = serde_json::from_str(raw).unwrap();
        assert_eq!(snapshot.prior_env, None);
        assert_eq!(snapshot.auto_config_url, "");
    }

    #[test]
    fn snapshot_round_trips_prior_env() {
        let mut prior = BTreeMap::new();
        prior.insert("HTTP_PROXY".to_string(), "http://corp:3128".to_string());
        let snapshot = ProxySnapshot {
            enable: 0,
            server: String::new(),
            bypass: String::new(),
            auto_config_url: String::new(),
            prior_env: Some(prior.clone()),
        };
        let raw = serde_json::to_string(&snapshot).unwrap();
        let loaded: ProxySnapshot = serde_json::from_str(&raw).unwrap();
        assert_eq!(loaded.prior_env, Some(prior));
    }
}
