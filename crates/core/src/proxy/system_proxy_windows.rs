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

use std::ffi::c_void;
use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use winreg::enums::{HKEY_CURRENT_USER, KEY_READ, KEY_SET_VALUE};
use winreg::RegKey;

use crate::env;

const INTERNET_SETTINGS: &str = r"Software\Microsoft\Windows\CurrentVersion\Internet Settings";

// INTERNET_OPTION_* constants from wininet.h.
const INTERNET_OPTION_REFRESH: u32 = 37;
const INTERNET_OPTION_SETTINGS_CHANGED: u32 = 39;

#[link(name = "wininet")]
extern "system" {
    fn InternetSetOptionW(
        h_internet: *mut c_void,
        dw_option: u32,
        lp_buffer: *mut c_void,
        dw_buffer_length: u32,
    ) -> i32;
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
    set_auto_config(&snapshot.auto_config_url)
}

/// Turn the WinINET proxy off (leaving the server string intact) and clear any
/// PAC URL we set. Promptless fail-safe used when no snapshot is available, so
/// a dead engine never strands traffic at our proxy or a dead PAC.
pub fn force_off() -> Result<()> {
    let key = settings_key(true)?;
    key.set_value("ProxyEnable", &0u32)
        .context("setting ProxyEnable")?;
    notify_wininet();
    set_auto_config("")
}

// --- Proxy environment variables -----------------------------------------
//
// WinINET's PAC above only reaches clients that go through WinINET. The
// command-line AI tools don't: Node, Bun and Python read `HTTPS_PROXY` and
// friends, and some (OpenCode) expose no proxy setting of their own at all.
// `HKCU\Environment` is the per-user environment every new process inherits,
// so writing there is the Windows equivalent of the Linux `environment.d`
// drop-in and the macOS `launchctl setenv`.
//
// Two differences from the other platforms:
//
// - **These persist across a reboot.** macOS launchd variables evaporate at
//   logout, so a crashed session self-heals there; here a stale value would
//   outlive the crash. `reconcile_on_startup` clears them for that reason.
// - **Names are case-insensitive**, so we write only the upper-case four
//   ([`super::proxy_env::VARS_CASE_INSENSITIVE`]); adding `https_proxy` as well
//   would be two writes contending for one entry.
//
// As everywhere else, already-running processes keep their old environment
// until relaunched.

const ENVIRONMENT: &str = "Environment";

// WM_SETTINGCHANGE broadcast constants from winuser.h.
const HWND_BROADCAST: isize = 0xffff;
const WM_SETTINGCHANGE: u32 = 0x001a;
const SMTO_ABORTIFHUNG: u32 = 0x0002;

#[link(name = "user32")]
extern "system" {
    fn SendMessageTimeoutW(
        hwnd: isize,
        msg: u32,
        wparam: usize,
        lparam: *const u16,
        flags: u32,
        timeout: u32,
        result: *mut usize,
    ) -> isize;
}

fn environment_key(write: bool) -> Result<RegKey> {
    let access = if write {
        KEY_READ | KEY_SET_VALUE
    } else {
        KEY_READ
    };
    RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey_with_flags(ENVIRONMENT, access)
        .with_context(|| format!("opening HKCU\\{ENVIRONMENT}"))
}

/// Tell the shell the environment block changed, so processes launched from
/// Explorer afterwards inherit the new values instead of the ones Explorer
/// captured at *its* startup. Without this the variables only take effect at
/// the next logon.
///
/// `SMTO_ABORTIFHUNG` with a short timeout because this is a broadcast to every
/// top-level window: one wedged application must not block the proxy toggle.
fn broadcast_environment_change() {
    let param: Vec<u16> = ENVIRONMENT
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    let mut result: usize = 0;
    // SAFETY: `param` is a NUL-terminated UTF-16 buffer that outlives the call,
    // and `result` is a valid out-pointer. The timeout bounds the call.
    unsafe {
        SendMessageTimeoutW(
            HWND_BROADCAST,
            WM_SETTINGCHANGE,
            0,
            param.as_ptr(),
            SMTO_ABORTIFHUNG,
            5_000,
            &mut result,
        );
    }
}

/// Current value of one managed variable, or `None` when unset. An empty string
/// counts as unset so restoring it becomes a delete rather than an empty entry.
fn env_get(key: &str) -> Option<String> {
    environment_key(false)
        .ok()?
        .get_value::<String, _>(key)
        .ok()
        .filter(|value| !value.is_empty())
}

/// Write the proxy variables into the per-user environment, after recording
/// what they held before so [`disable_env`] can put a user-owned proxy back.
///
/// Takes the *engine* port, not the PAC port: an env-var proxy has no PAC
/// equivalent, so tools that honour it dial the engine directly and it
/// blind-tunnels whatever isn't an intercepted domain.
pub fn enable_env(port: u16) -> Result<()> {
    let assignments = super::proxy_env::case_insensitive(port)?;
    super::proxy_env::snapshot_prior(&super::proxy_env::VARS_CASE_INSENSITIVE, env_get);
    let key = environment_key(true)?;
    for (name, value) in &assignments {
        key.set_value(name, value)
            .with_context(|| format!("setting HKCU\\{ENVIRONMENT}\\{name}"))?;
    }
    broadcast_environment_change();
    Ok(())
}

/// The proxy URL currently exported to the per-user environment, read back
/// from the registry rather than from anything we remember writing.
pub fn exported_proxy() -> Result<Option<String>> {
    Ok(env_get("HTTPS_PROXY"))
}

/// The PAC and the exported variables are separate mechanisms here, so the
/// user can keep GUI routing while declining the machine-wide environment
/// change (contrast Linux, where the drop-in is both at once).
pub const ENV_CHANNEL_SEPARABLE: bool = true;

/// Put the environment back the way it was: a variable the user owned is
/// restored to their value, one only we set is deleted.
///
/// Best-effort per variable and idempotent - it runs on the disable, crash and
/// startup-reconcile paths, where giving up halfway would leave the rest of the
/// set pointing at a dead engine.
pub fn disable_env() -> Result<()> {
    let prior = super::proxy_env::load_snapshot().unwrap_or_else(|e| {
        eprintln!("gate proxy: unreadable proxy env snapshot ({e}); clearing our variables");
        None
    });
    let key = environment_key(true)?;
    let mut failed = Vec::new();
    for name in super::proxy_env::VARS_CASE_INSENSITIVE {
        // No snapshot means we cannot know the user's prior value, so delete -
        // leaving a dead proxy behind is the one outcome we must not pick.
        let restore_to = prior
            .as_ref()
            .and_then(|p| p.vars.get(name).cloned())
            .flatten();
        let outcome = match restore_to {
            Some(value) => key.set_value(name, &value).map_err(anyhow::Error::from),
            None => match key.delete_value(name) {
                Ok(()) => Ok(()),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
                Err(e) => Err(anyhow::Error::from(e)),
            },
        };
        if let Err(e) = outcome {
            failed.push(format!("{name}: {e}"));
        }
    }
    broadcast_environment_change();
    let _ = super::proxy_env::clear_snapshot();
    if !failed.is_empty() {
        anyhow::bail!("could not revert proxy environment ({})", failed.join("; "));
    }
    Ok(())
}
