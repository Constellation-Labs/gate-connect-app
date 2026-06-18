//! Windows system HTTP/HTTPS proxy wiring via the per-user WinINET settings in
//! the registry (`HKCU\Software\Microsoft\Windows\CurrentVersion\Internet
//! Settings`). Enabling points the WinINET proxy at our loopback engine;
//! disabling restores the exact prior state from a snapshot (mirrors the
//! `previousEnv` restore pattern in [`crate::env`], and the macOS
//! `system_proxy` snapshot/restore). Everything here lives under `HKCU`, so —
//! like the macOS `networksetup` path — none of it needs admin; the only
//! privileged step in the subsystem is trusting the CA.
//!
//! WinINET caches the proxy config, so after every registry change we poke it
//! with `InternetSetOption(INTERNET_OPTION_SETTINGS_CHANGED + _REFRESH)` —
//! otherwise already-running apps keep using the stale settings until the next
//! logon.

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

/// Per-user environment block. Setting CLI-relevant proxy + CA vars here is the
/// Windows analog of the macOS shell-rc block / Linux `/etc/environment`:
/// WinINET steers only apps that honor the system proxy, but command-line dev
/// tools (Node-based CLIs like the Gemini CLI, reqwest-based CLIs like Codex)
/// read these from their environment instead.
const USER_ENV: &str = r"Environment";

/// The env vars we own under `HKCU\Environment`. `HTTP(S)_PROXY` point CLIs at
/// the engine; `NODE_EXTRA_CA_CERTS` makes Node trust the engine's minted leaf
/// certs (it ships its own bundle and ignores the system store); `NO_PROXY`
/// keeps loopback off the proxy.
const ENV_VARS: &[&str] = &[
    "HTTP_PROXY",
    "HTTPS_PROXY",
    "NO_PROXY",
    "NODE_EXTRA_CA_CERTS",
];

// INTERNET_OPTION_* constants from wininet.h.
const INTERNET_OPTION_REFRESH: u32 = 37;
const INTERNET_OPTION_SETTINGS_CHANGED: u32 = 39;

// Broadcast constants from winuser.h, used to tell already-running processes
// that the user environment changed (otherwise they keep their stale env).
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

/// Snapshot of the user's WinINET proxy configuration — the three values we
/// touch, so `restore` can put them back exactly.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProxySnapshot {
    /// `ProxyEnable` (0 / 1).
    pub enable: u32,
    /// `ProxyServer` (e.g. `127.0.0.1:8080`, or `http=...;https=...`).
    pub server: String,
    /// `ProxyOverride` — the bypass list (e.g. `<local>`).
    pub bypass: String,
    /// Prior values of the `HKCU\Environment` vars we inject ([`ENV_VARS`]),
    /// captured so `restore` can put back exactly what was there — re-setting
    /// the vars that existed and deleting the ones we added. `serde(default)`
    /// keeps pre-upgrade snapshots (which lack this field) loadable.
    #[serde(default)]
    pub prior_env: BTreeMap<String, String>,
}

fn snapshot_path() -> Result<PathBuf> {
    Ok(env::app_support_dir()?
        .join("proxy")
        .join("system-proxy.snapshot.json"))
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
/// default rather than an error — a machine that has never had a proxy set
/// simply has no `ProxyServer` value. Non-privileged.
pub fn snapshot() -> Result<ProxySnapshot> {
    let key = settings_key(false)?;
    Ok(ProxySnapshot {
        enable: key.get_value("ProxyEnable").unwrap_or(0),
        server: key.get_value("ProxyServer").unwrap_or_default(),
        bypass: key.get_value("ProxyOverride").unwrap_or_default(),
        prior_env: read_user_env()?,
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
    // SAFETY: both calls pass a null handle and null buffer — the documented
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

/// Path to our CA cert, mirrored from [`super::ca`] — used for
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

/// Tell already-running processes the user environment changed. Without this
/// they keep their stale environment until the next logon.
fn broadcast_env_change() {
    let param: Vec<u16> = "Environment\0".encode_utf16().collect();
    let mut result: usize = 0;
    // SAFETY: documented broadcast of an environment change — a valid
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

/// Read the current values of the env vars we manage. A missing var is simply
/// absent from the map (not an error). Non-privileged.
fn read_user_env() -> Result<BTreeMap<String, String>> {
    let key = env_key(false)?;
    let mut map = BTreeMap::new();
    for var in ENV_VARS {
        if let Ok(v) = key.get_value::<String, _>(var) {
            map.insert((*var).to_string(), v);
        }
    }
    Ok(map)
}

/// Inject our proxy + CA env into `HKCU\Environment`, then broadcast the change.
fn set_user_env(port: u16) -> Result<()> {
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

fn delete_env_value(key: &RegKey, var: &str) -> Result<()> {
    match key.delete_value(var) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e).with_context(|| format!("deleting {var}")),
    }
}

/// Restore `HKCU\Environment` to `prior`: re-set the vars that existed before,
/// delete the ones we added. Then broadcast the change.
fn restore_user_env(prior: &BTreeMap<String, String>) -> Result<()> {
    let key = env_key(true)?;
    for var in ENV_VARS {
        match prior.get(*var) {
            Some(v) => key
                .set_value(var, v)
                .with_context(|| format!("setting {var}"))?,
            None => delete_env_value(&key, var)?,
        }
    }
    broadcast_env_change();
    Ok(())
}

/// Delete the env vars we inject (fail-safe used when no snapshot survives).
fn clear_user_env() -> Result<()> {
    let key = env_key(true)?;
    for var in ENV_VARS {
        delete_env_value(&key, var)?;
    }
    broadcast_env_change();
    Ok(())
}

/// Point the WinINET proxy at the loopback engine and inject the proxy + CA env
/// for CLIs. `<local>` keeps localhost / intranet traffic off the proxy.
/// Promptless (HKCU). If the env injection fails, clear any partial vars so new
/// processes aren't left pointed at a port the caller is about to tear down.
pub fn enable(port: u16) -> Result<()> {
    set_values(1, &format!("127.0.0.1:{port}"), "<local>")?;
    if let Err(e) = set_user_env(port) {
        let _ = clear_user_env();
        return Err(e);
    }
    Ok(())
}

/// Restore the user's proxy config and environment from a snapshot. Promptless;
/// safety-critical. Both reverts are attempted even if one fails.
pub fn restore(snapshot: &ProxySnapshot) -> Result<()> {
    let wininet = set_values(snapshot.enable, &snapshot.server, &snapshot.bypass);
    let env = restore_user_env(&snapshot.prior_env);
    wininet.and(env)
}

/// Turn the WinINET proxy off (leaving the server string intact) and clear the
/// env vars we inject. Promptless fail-safe used when no snapshot is available,
/// so a dead engine never strands traffic. Both reverts are attempted.
pub fn force_off() -> Result<()> {
    let wininet = (|| -> Result<()> {
        let key = settings_key(true)?;
        key.set_value("ProxyEnable", &0u32)
            .context("setting ProxyEnable")?;
        notify_wininet();
        Ok(())
    })();
    let env = clear_user_env();
    wininet.and(env)
}
