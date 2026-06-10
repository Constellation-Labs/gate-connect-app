//! Verify the user has an active Claude Code session.
//!
//! Gate Connect doesn't copy or cache the Claude Code session token —
//! instead, our credential helper reads the live token straight out of
//! Claude Code's session store on every Cowork request (Cowork caches it
//! for ~50min via `inferenceCredentialHelperTtlSec`). That way refresh
//! is handled by Claude Code itself; when Claude Code rotates the
//! access token, we transparently pick up the new one on the next
//! helper invocation.
//!
//! The session store differs by platform:
//! - macOS: the `Claude Code-credentials` Keychain entry.
//! - Windows: a plaintext `~/.claude/.credentials.json` file (honoring
//!   `CLAUDE_CONFIG_DIR`).
//!
//! What we verify here is just that the entry *exists* and contains a
//! parseable `claudeAiOauth.accessToken` — enough to fail fast at
//! connect time rather than at first-request time.
//!
//! Trust model: this is session *delegation*, not OAuth. There is no PKCE,
//! state, or token exchange — we (and the credential helper) simply read
//! Claude Code's existing session store. Any local actor that can read that
//! store can impersonate the user through Gate indefinitely: there is no
//! rotation hook we control and no per-tool scoping. The Claude Code session
//! store is effectively the trust root.

use anyhow::{bail, Context, Result};
#[cfg(target_os = "macos")]
use std::process::Command;

#[cfg(target_os = "macos")]
const CLAUDE_KEYCHAIN_SERVICE: &str = "Claude Code-credentials";

/// Sentinel saved into our per-tool keychain entry to mark "delegate
/// to the Claude Code session". The helper script recognizes this and
/// reads `Claude Code-credentials` instead of treating it as a literal
/// credential.
pub const CLAUDE_CODE_SENTINEL: &str = "@claude-code-session";

/// Confirm Claude Code is signed in and the JSON entry contains an
/// `accessToken`. Returns the sentinel string the caller should
/// persist to the per-tool keychain entry.
pub fn verify_claude_code_session() -> Result<&'static str> {
    let user = crate::env::current_user()?;
    let raw = read_claude_credentials(&user).context(
        "No Claude Code session found. Open Claude Code and sign in (run `claude`, then `/login`), then retry.",
    )?;
    let token = parse_json_access_token(&raw).context(
        "Claude Code session store doesn't contain a claudeAiOauth.accessToken — Claude Code may have changed its format.",
    )?;
    if !token.starts_with("sk-ant-oat") {
        bail!(
            "Claude Code accessToken doesn't have the expected sk-ant-oat prefix — refusing to use."
        );
    }
    Ok(CLAUDE_CODE_SENTINEL)
}

/// Read Claude Code's raw session-store JSON for `user` from the
/// `Claude Code-credentials` Keychain entry.
#[cfg(target_os = "macos")]
pub(crate) fn read_claude_credentials(user: &str) -> Option<String> {
    let out = Command::new("/usr/bin/security")
        .args([
            "find-generic-password",
            "-s",
            CLAUDE_KEYCHAIN_SERVICE,
            "-a",
            user,
            "-w",
        ])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let mut raw = String::from_utf8(out.stdout).ok()?;
    if raw.ends_with('\n') {
        raw.pop();
    }
    Some(raw)
}

/// Read Claude Code's session-store JSON from its per-user credentials
/// file. Honors `CLAUDE_CONFIG_DIR`, otherwise `%USERPROFILE%\.claude`.
/// The file is plaintext JSON (not DPAPI-encrypted).
#[cfg(target_os = "windows")]
pub(crate) fn read_claude_credentials(_user: &str) -> Option<String> {
    let dir = std::env::var_os("CLAUDE_CONFIG_DIR")
        .map(std::path::PathBuf::from)
        .or_else(|| {
            std::env::var_os("USERPROFILE").map(|p| std::path::PathBuf::from(p).join(".claude"))
        })?;
    std::fs::read_to_string(dir.join(".credentials.json")).ok()
}

/// Claude Desktop (and thus Cowork) doesn't exist on other platforms;
/// this stub keeps `claude_session_delegate` compiling in the core crate everywhere.
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
pub(crate) fn read_claude_credentials(_user: &str) -> Option<String> {
    None
}

/// Extract the `claudeAiOauth.accessToken` field from the session-store
/// JSON. Uses serde_json so escapes in either the field name or the
/// value parse correctly.
pub(crate) fn parse_json_access_token(s: &str) -> Option<String> {
    let parsed: serde_json::Value = serde_json::from_str(s).ok()?;
    parsed
        .get("claudeAiOauth")?
        .get("accessToken")?
        .as_str()
        .map(|s| s.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_access_token_from_claude_credentials_json() {
        let raw = r#"{"claudeAiOauth":{"accessToken":"sk-ant-oat01-REAL","refreshToken":"sk-ant-ort01-OTHER","expiresAt":1234}}"#;
        assert_eq!(
            parse_json_access_token(raw).as_deref(),
            Some("sk-ant-oat01-REAL")
        );
    }

    #[test]
    fn picks_accesstoken_not_refresh_token() {
        let raw = r#"{"claudeAiOauth":{"refreshToken":"sk-ant-ort01-WRONG","accessToken":"sk-ant-oat01-RIGHT"}}"#;
        assert_eq!(
            parse_json_access_token(raw).as_deref(),
            Some("sk-ant-oat01-RIGHT")
        );
    }

    #[test]
    fn ignores_mcp_oauth_section() {
        let raw = r#"{"claudeAiOauth":{"accessToken":"sk-ant-oat01-REAL"},"mcpOAuth":{"foo":{"accessToken":"WRONG"}}}"#;
        assert_eq!(
            parse_json_access_token(raw).as_deref(),
            Some("sk-ant-oat01-REAL")
        );
    }

    #[test]
    fn rejects_non_json_payload() {
        assert!(parse_json_access_token("sk-ant-oat01-bare").is_none());
    }
}
