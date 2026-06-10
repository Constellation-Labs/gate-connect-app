//! OS-aware directory + identity lookups. Per-OS specifics live in the
//! `dirs` crate (XDG / Known-Folder / Application Support) so we don't
//! hand-roll `$HOME/Library/...` paths.

use anyhow::{Context, Result};
use std::path::PathBuf;

pub fn home() -> Result<PathBuf> {
    dirs::home_dir().context("could not resolve user home directory")
}

/// Username used as the keychain entry's `account` field (and, on macOS,
/// the per-user segment of the Managed Preferences path). On each OS the
/// native secret store also user-scopes the entry, so this string is
/// really a per-entry tag rather than an identity assertion.
///
/// On Windows the Cowork credential helper resolves this from inside a
/// process *Claude Desktop spawns*, so it must not hinge on inherited
/// environment. We read it from the OS (`GetUserNameW`) first and only
/// fall back to `USER` / `USERNAME`. The OS value matches `%USERNAME%` in
/// a normal session, so an entry written by the app and read by the helper
/// resolve to the same account tag.
pub fn current_user() -> Result<String> {
    #[cfg(windows)]
    if let Some(name) = windows_username() {
        return Ok(name);
    }
    #[cfg(unix)]
    return unix_username();
    #[cfg(windows)]
    return std::env::var("USERNAME").context("USERNAME is not set");
}

/// Resolve the current user from the password database via
/// `getpwuid(geteuid())`, rather than trusting inherited `$USER` /
/// `$USERNAME`. A parent process can pre-set those env vars before
/// launching us, which would otherwise point the keychain lookup
/// (`-a <user>`) at a different account's entry.
#[cfg(unix)]
fn unix_username() -> Result<String> {
    use std::ffi::CStr;
    // SAFETY: `getpwuid` returns a pointer into a libc-owned static buffer;
    // we copy the name out immediately and never retain the pointer.
    unsafe {
        let uid = libc::geteuid();
        let pw = libc::getpwuid(uid);
        if pw.is_null() {
            anyhow::bail!("getpwuid({uid}) returned null");
        }
        let name = (*pw).pw_name;
        if name.is_null() {
            anyhow::bail!("getpwuid({uid}) returned a null pw_name");
        }
        Ok(CStr::from_ptr(name).to_string_lossy().into_owned())
    }
}

/// Current login name via Win32 `GetUserNameW`, independent of any
/// environment variable. Returns `None` on failure so the caller can fall
/// back to the env vars.
#[cfg(windows)]
fn windows_username() -> Option<String> {
    use std::ffi::OsString;
    use std::os::windows::ffi::OsStringExt;

    #[link(name = "advapi32")]
    extern "system" {
        fn GetUserNameW(lp_buffer: *mut u16, pcb_buffer: *mut u32) -> i32;
    }

    // UNLEN (max username length) is 256; +1 leaves room for the NUL.
    let mut len: u32 = 257;
    let mut buf = vec![0u16; len as usize];
    // SAFETY: `buf` has `len` slots; GetUserNameW writes at most `len` UTF-16
    // code units (including the NUL) and updates `len` to the count written.
    let ok = unsafe { GetUserNameW(buf.as_mut_ptr(), &mut len) };
    if ok == 0 || len == 0 {
        return None;
    }
    // `len` counts the trailing NUL — exclude it.
    let name = OsString::from_wide(&buf[..(len as usize - 1)]);
    name.into_string().ok().filter(|s| !s.is_empty())
}

/// Per-OS data directory for Gate Connect's own files (helper scripts,
/// install-id, account.json).
///
/// - macOS: `~/Library/Application Support/Gate Connect`
/// - Windows: `%LOCALAPPDATA%\Gate Connect`
/// - Linux: `$XDG_DATA_HOME/Gate Connect` (or `~/.local/share/Gate Connect`)
pub fn app_support_dir() -> Result<PathBuf> {
    Ok(dirs::data_local_dir()
        .context("could not resolve user data directory")?
        .join("Gate Connect"))
}

/// `~/.claude` — Claude Code's user config root. Same on all platforms;
/// Claude Code itself reads `~/.claude/settings.json` regardless of OS.
pub fn claude_code_config_dir() -> Result<PathBuf> {
    Ok(home()?.join(".claude"))
}

/// `~/.claude/settings.json` — Claude Code's user settings. Supports an
/// `env` block whose entries are injected as process env vars at every
/// `claude` invocation; that's the surface Gate Connect writes to.
pub fn claude_code_settings_path() -> Result<PathBuf> {
    Ok(claude_code_config_dir()?.join("settings.json"))
}

/// `~/.codex` — Codex CLI's user config root. Same on all platforms.
pub fn codex_config_dir() -> Result<PathBuf> {
    Ok(home()?.join(".codex"))
}

/// `~/.codex/config.toml` — Codex CLI's user config. Gate Connect adds a
/// `[model_providers.gate]` block and flips top-level `model_provider`
/// to point at it, leaving the rest of the file untouched.
pub fn codex_config_toml_path() -> Result<PathBuf> {
    Ok(codex_config_dir()?.join("config.toml"))
}

/// `~/.codex/auth.json` — Codex CLI's login store. Gate Connect reads
/// the `auth_mode` field at connect time to pick the right upstream URL
/// shape (ChatGPT vs. OpenAI API). The credential helper also reads this
/// file at every Codex request to emit the current bearer.
pub fn codex_auth_json_path() -> Result<PathBuf> {
    Ok(codex_config_dir()?.join("auth.json"))
}

/// `~/.config/opencode` — OpenCode's user config root. OpenCode is a
/// Node-based CLI that uses XDG-style paths on every OS (it does not
/// follow the macOS `~/Library/Application Support` or Windows
/// `%APPDATA%` conventions), so this path is the same everywhere.
pub fn opencode_config_dir() -> Result<PathBuf> {
    Ok(home()?.join(".config/opencode"))
}

/// `~/.config/opencode/opencode.json` — OpenCode's user config. Gate
/// Connect adds (or replaces) a `provider.gate` block and leaves the
/// rest of the file untouched.
pub fn opencode_config_path() -> Result<PathBuf> {
    Ok(opencode_config_dir()?.join("opencode.json"))
}

/// `~/.local/share/opencode/auth.json` — OpenCode's credential store.
/// `opencode auth login <provider-id>` writes into this file, and
/// OpenCode injects the matching entry into `provider.<id>.options.apiKey`
/// at request time. Gate Connect writes the `gate` entry here on
/// connect so users do not have to drop into the terminal to attach an
/// upstream key. Path layout is the same on every OS .
pub fn opencode_auth_path() -> Result<PathBuf> {
    Ok(home()?.join(".local/share/opencode/auth.json"))
}

// ---------------------------------------------------------------------
// macOS-only paths used by the Cowork integration and the standard-mode
// → 3P-mode migration. Cowork itself is macOS-only (it routes via
// `/Library/Managed Preferences`, a macOS subsystem), so these helpers
// are gated to match.
// ---------------------------------------------------------------------

/// `~/Library/Application Support/Claude` — Cowork's standard-mode userData.
#[cfg(target_os = "macos")]
pub fn claude_user_data_dir() -> Result<PathBuf> {
    Ok(home()?.join("Library/Application Support/Claude"))
}

/// `~/Library/Application Support/Claude-3p` — Cowork's gateway-mode userData.
/// Cowork creates this on first launch in 3P mode and never touches it from
/// standard mode, so it's a clean target for the migration.
#[cfg(target_os = "macos")]
pub fn claude_3p_user_data_dir() -> Result<PathBuf> {
    Ok(home()?.join("Library/Application Support/Claude-3p"))
}

/// `~/Documents/Claude` — where Cowork stashes scheduled-task `SKILL.md`
/// bodies and artifact files. Mode-agnostic (path is the same whether
/// Cowork is running in standard or 3P mode).
#[cfg(target_os = "macos")]
pub fn claude_documents_dir() -> Result<PathBuf> {
    Ok(home()?.join("Documents/Claude"))
}

/// `/Library/Managed Preferences/<user>/com.anthropic.claudefordesktop.plist`
///
/// This is the documented Cowork 3P managed-preferences location. Writing
/// here requires root because `/Library/Managed Preferences` is owned by
/// `root:wheel`. The CLI re-execs itself under `sudo` to land bytes here.
#[cfg(target_os = "macos")]
pub fn cowork_managed_plist_path() -> Result<PathBuf> {
    let user = current_user()?;
    Ok(PathBuf::from("/Library/Managed Preferences")
        .join(&user)
        .join("com.anthropic.claudefordesktop.plist"))
}

/// Candidate filesystem locations whose presence indicates Claude Desktop
/// is installed on Windows. Claude ships through several channels and its
/// exact layout isn't contractual, so detection is best-effort over a set
/// of known paths rather than one guessed directory:
///
/// - `%LOCALAPPDATA%\AnthropicClaude` — Squirrel per-user install root.
/// - `%LOCALAPPDATA%\Programs\Claude` — per-user "Programs" install.
/// - `%LOCALAPPDATA%\Claude` / `%APPDATA%\Claude` — userData dirs created
///   on first run.
///
/// MSIX/Store installs and Uninstall-registry entries are checked
/// separately by the Windows Cowork integration's `detect`.
#[cfg(target_os = "windows")]
pub fn claude_desktop_path_candidates() -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Some(local) = dirs::data_local_dir() {
        out.push(local.join("AnthropicClaude"));
        out.push(local.join("Programs").join("Claude"));
        out.push(local.join("Programs").join("claude"));
        out.push(local.join("Claude"));
    }
    if let Some(roaming) = dirs::config_dir() {
        out.push(roaming.join("Claude"));
    }
    out
}
