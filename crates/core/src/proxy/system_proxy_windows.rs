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
    })
}

pub fn save_snapshot(snapshot: &ProxySnapshot) -> Result<()> {
    let path = snapshot_path()?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
    }
    let raw = serde_json::to_string_pretty(snapshot).context("serializing proxy snapshot")?;
    fs::write(&path, raw).with_context(|| format!("writing {}", path.display()))
}

pub fn load_snapshot() -> Result<Option<ProxySnapshot>> {
    let path = snapshot_path()?;
    match fs::read_to_string(&path) {
        Ok(raw) => Ok(Some(
            serde_json::from_str(&raw)
                .with_context(|| format!("parsing {} as JSON", path.display()))?,
        )),
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

/// Point the WinINET proxy at the loopback engine. `<local>` keeps localhost /
/// intranet traffic off the proxy. Promptless (HKCU).
pub fn enable(port: u16) -> Result<()> {
    set_values(1, &format!("127.0.0.1:{port}"), "<local>")
}

/// Restore the user's proxy config from a snapshot. Promptless; safety-critical.
pub fn restore(snapshot: &ProxySnapshot) -> Result<()> {
    set_values(snapshot.enable, &snapshot.server, &snapshot.bypass)
}

/// Turn the WinINET proxy off (leaving the server string intact). Promptless
/// fail-safe used when no snapshot is available, so a dead engine never strands
/// traffic.
pub fn force_off() -> Result<()> {
    let key = settings_key(true)?;
    key.set_value("ProxyEnable", &0u32)
        .context("setting ProxyEnable")?;
    notify_wininet();
    Ok(())
}
