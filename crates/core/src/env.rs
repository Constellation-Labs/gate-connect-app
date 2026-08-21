//! OS-aware directory + identity lookups. Per-OS specifics live in the
//! `dirs` crate (XDG / Known-Folder / Application Support) so we don't
//! hand-roll `$HOME/Library/...` paths.

use anyhow::{Context, Result};
use std::path::PathBuf;
use std::sync::Mutex;

/// Test seam: when `GATE_CONNECT_TEST_HOME` is set, root every per-user path
/// under it instead of the real OS home / data dir. This is the only portable
/// way to redirect the filesystem on Windows too (`dirs` reads Known Folders,
/// not `$HOME`), so the CLI flow tests can run hermetically on all three OSes.
/// Unset in production, so it never affects real runs.
fn test_home_override() -> Option<PathBuf> {
    std::env::var_os("GATE_CONNECT_TEST_HOME")
        .map(PathBuf::from)
        .filter(|p| !p.as_os_str().is_empty())
}

/// Serializes the unit tests that redirect per-user paths for the whole process.
///
/// The seam above is an environment variable, so it is process-global, while
/// libtest runs unit tests as threads of one process. A test that sets it and a
/// test that depends on its absence cannot overlap: whichever reads during the
/// other's window resolves against the wrong filesystem, and the failure surfaces
/// in the reader rather than in whoever moved the ground.
///
/// A lock private to one module does not buy this, since it excludes that module's
/// own tests and nothing else. Every test that redirects these paths, or that reads
/// them and would be wrong to see someone else's, takes this one lock; what each
/// one stands to read instead is documented at its own call site.
#[cfg(test)]
pub(crate) fn path_env_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: Mutex<()> = Mutex::new(());
    // A panicking test poisons the lock; the rest should still run.
    LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

/// A non-empty path from an environment variable, or `None`.
fn env_path(var: &str) -> Option<PathBuf> {
    std::env::var_os(var)
        .map(PathBuf::from)
        .filter(|p| !p.as_os_str().is_empty())
}

/// Test seam: the login keychain to install CA trust into.
///
/// A login keychain belongs to the OS session, not to `$HOME`, so it is the
/// one path that must *not* follow the home redirect above. The e2e harness
/// points every other path at a throwaway home, which made this resolve to a
/// keychain that had never existed and failed every `add-trusted-cert` with
/// "The specified keychain could not be found" - so CA trust, and with it the
/// whole proxy engine, was silently untested on macOS CI.
///
/// Unset in production, where the login keychain is exactly where `$HOME` says
/// it is.
#[cfg(target_os = "macos")]
pub fn test_login_keychain() -> Option<PathBuf> {
    env_path("GATE_CONNECT_TEST_LOGIN_KEYCHAIN")
}

pub fn home() -> Result<PathBuf> {
    if let Some(home) = test_home_override() {
        return Ok(home);
    }
    dirs::home_dir().context("could not resolve user home directory")
}

/// Username used as the keychain entry's `account` field and as the
/// per-user segment of the proxy CA's trust-store lookup. On each OS the
/// native secret store also user-scopes the entry, so this string is
/// really a per-entry tag rather than an identity assertion.
///
/// We read it from the OS (`GetUserNameW` on Windows) first and only fall
/// back to `USER` / `USERNAME`, so the value stays stable regardless of
/// inherited environment and matches `%USERNAME%` in a normal session.
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
    // `len` counts the trailing NUL - exclude it.
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
    // Env-var seam first so it works across a spawned `gate-connect` process
    // (the CLI flow tests run the binary as a child - a process-global mutex
    // override wouldn't reach it).
    if let Some(home) = test_home_override() {
        return Ok(home.join("app-support").join("Gate Connect"));
    }
    if let Some(dir) = APP_SUPPORT_OVERRIDE
        .lock()
        .expect("app-support override mutex poisoned")
        .as_ref()
    {
        return Ok(dir.clone());
    }
    Ok(dirs::data_local_dir()
        .context("could not resolve user data directory")?
        .join("Gate Connect"))
}

/// Optional process-global override for [`app_support_dir`], installed only by
/// tests via [`set_app_support_dir_for_tests`]. `None` in every normal build, so
/// production always resolves the real per-OS data dir above. It exists because
/// `dirs::data_local_dir()` reads the Windows `FOLDERID_LocalAppData` Known
/// Folder via the OS API - a `$HOME`/env override redirects it on macOS and
/// Linux but not on Windows, so tests need a direct seam to land their files in
/// a throwaway dir on every platform.
static APP_SUPPORT_OVERRIDE: Mutex<Option<PathBuf>> = Mutex::new(None);

/// Point [`app_support_dir`] at `dir` for the rest of this process (or clear the
/// override with `None`). Test-only seam; never called in production.
#[doc(hidden)]
pub fn set_app_support_dir_for_tests(dir: Option<PathBuf>) {
    *APP_SUPPORT_OVERRIDE
        .lock()
        .expect("app-support override mutex poisoned") = dir;
}

/// `~/.claude` - Claude Code's user config root. Same on all platforms;
/// Claude Code itself reads `~/.claude/settings.json` regardless of OS.
pub fn claude_code_config_dir() -> Result<PathBuf> {
    Ok(home()?.join(".claude"))
}

/// `~/.claude/settings.json` - Claude Code's user settings. Supports an
/// `env` block whose entries are injected as process env vars at every
/// `claude` invocation; that's the surface Gate Connect writes to.
pub fn claude_code_settings_path() -> Result<PathBuf> {
    Ok(claude_code_config_dir()?.join("settings.json"))
}

/// `~/.codex` - Codex CLI's user config root. Same on all platforms.
pub fn codex_config_dir() -> Result<PathBuf> {
    Ok(home()?.join(".codex"))
}

/// `~/.codex/config.toml` - Codex CLI's user config. Gate Connect adds a
/// `[model_providers.gate]` block and flips top-level `model_provider`
/// to point at it, leaving the rest of the file untouched.
pub fn codex_config_toml_path() -> Result<PathBuf> {
    Ok(codex_config_dir()?.join("config.toml"))
}

/// `~/.codex/auth.json` - Codex CLI's login store. Gate Connect reads
/// the `auth_mode` field at connect time to pick the right upstream URL
/// shape (ChatGPT vs. OpenAI API). The credential helper also reads this
/// file at every Codex request to emit the current bearer.
pub fn codex_auth_json_path() -> Result<PathBuf> {
    Ok(codex_config_dir()?.join("auth.json"))
}

/// OpenCode's user config root. OpenCode is a Node-based CLI that uses
/// XDG-style paths on every OS (it does not follow the macOS `~/Library/
/// Application Support` or Windows `%APPDATA%` conventions).
///
/// OpenCode resolves this via XDG and documents two overrides, so hardcoding
/// `~/.config/opencode` meant Gate Connect could edit a file OpenCode never
/// reads. Precedence, highest first:
/// - `OPENCODE_CONFIG_DIR`
/// - `$XDG_CONFIG_HOME/opencode`
/// - `~/.config/opencode`
///
/// XDG is skipped while the test-home seam is active: a test that redirects HOME
/// must not pick up the developer's real `XDG_CONFIG_HOME` and write outside its
/// scratch directory.
pub fn opencode_config_dir() -> Result<PathBuf> {
    if let Some(dir) = env_path("OPENCODE_CONFIG_DIR") {
        return Ok(dir);
    }
    if test_home_override().is_none() {
        if let Some(xdg) = env_path("XDG_CONFIG_HOME") {
            return Ok(xdg.join("opencode"));
        }
    }
    Ok(home()?.join(".config/opencode"))
}

/// OpenCode's user config file, `opencode.json` inside
/// [`opencode_config_dir`]. `OPENCODE_CONFIG` (a full file path) overrides it,
/// matching OpenCode's own precedence.
///
/// Gate Connect edits `provider.<id>.options` in place for the providers it
/// redirects and leaves the rest of the file untouched.
///
/// Note what this cannot see: OpenCode also merges a project-level
/// `opencode.json` from the working directory, and that wins over the global
/// file. A project config touching the same provider options silently overrides
/// what we write here, and no amount of care in this function detects it.
pub fn opencode_config_path() -> Result<PathBuf> {
    if let Some(path) = env_path("OPENCODE_CONFIG") {
        return Ok(path);
    }
    Ok(opencode_config_dir()?.join("opencode.json"))
}

/// `~/.local/share/opencode/auth.json` - OpenCode's credential store.
/// `opencode auth login <provider-id>` writes into this file, and OpenCode
/// injects the matching entry into `provider.<id>.options.apiKey` at request
/// time. Gate Connect only *reads* this, to tell which providers the user has
/// authenticated and is therefore worth redirecting; it writes no credential of
/// its own. Honors `XDG_DATA_HOME` for the same reason as the config dir.
pub fn opencode_auth_path() -> Result<PathBuf> {
    if test_home_override().is_none() {
        if let Some(xdg) = env_path("XDG_DATA_HOME") {
            return Ok(xdg.join("opencode/auth.json"));
        }
    }
    Ok(home()?.join(".local/share/opencode/auth.json"))
}

/// `~/.openclaw` — OpenClaw's user state/config root. Same on every OS
/// (it uses a home-relative dir, not the macOS `~/Library/Application
/// Support` or Windows `%APPDATA%` conventions).
pub fn openclaw_config_dir() -> Result<PathBuf> {
    Ok(home()?.join(".openclaw"))
}

/// OpenClaw's config file. Defaults to `~/.openclaw/openclaw.json`, but
/// OpenClaw lets the user point `OPENCLAW_CONFIG_PATH` at a config file
/// kept outside the state dir — honor that here so Gate Connect edits the
/// file OpenClaw actually reads. The test-home seam still applies to the
/// default path via [`openclaw_config_dir`].
pub fn openclaw_config_path() -> Result<PathBuf> {
    if let Some(path) = std::env::var_os("OPENCLAW_CONFIG_PATH")
        .map(PathBuf::from)
        .filter(|p| !p.as_os_str().is_empty())
    {
        return Ok(path);
    }
    Ok(openclaw_config_dir()?.join("openclaw.json"))
}

/// Hermes config root.
///
/// Honors the `HERMES_HOME` override that both the Hermes CLI and Desktop app
/// read (they share this directory), falling back to the per-OS default:
/// - `HERMES_HOME` if set
/// - Linux/macOS: `~/.hermes`
/// - Windows: `%LOCALAPPDATA%\hermes`
pub fn hermes_config_dir() -> Result<PathBuf> {
    if let Some(path) = std::env::var_os("HERMES_HOME")
        .map(PathBuf::from)
        .filter(|p| !p.as_os_str().is_empty())
    {
        return Ok(path);
    }
    #[cfg(target_os = "windows")]
    {
        if let Some(h) = test_home_override() {
            return Ok(h.join(".hermes"));
        }
        dirs::data_local_dir()
            .context("could not resolve %LOCALAPPDATA%")
            .map(|d| d.join("hermes"))
    }
    #[cfg(not(target_os = "windows"))]
    Ok(home()?.join(".hermes"))
}

/// `~/.hermes/config.yaml` -- Hermes's config file.
pub fn hermes_config_path() -> Result<PathBuf> {
    Ok(hermes_config_dir()?.join("config.yaml"))
}
