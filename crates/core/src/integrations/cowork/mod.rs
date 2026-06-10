//! Cowork (Claude Desktop) integration.
//!
//! Points Claude Desktop's Cowork feature at Constellation Gate in 3P /
//! gateway mode (`inferenceProvider: gateway`). The credential plumbing is
//! identical on every OS — both secrets live in the native secret store —
//! but the *config surface* Claude Desktop reads differs by platform, so
//! `detect` / `status` / `connect` / `disconnect` delegate to a
//! per-platform submodule:
//!
//! - macOS: a managed-preferences plist plus an at-request credential
//!   helper script (see [`macos`]).
//! - Windows: the `HKCU\SOFTWARE\Policies\Claude` registry policy, with
//!   credentials baked into the policy values (see [`windows`]).
//!
//! Linux has no Claude Desktop, so this module is not compiled there.

use anyhow::Result;

use crate::env;
use crate::keychain;
use crate::registry::{ConnectInput, Integration, Status, ToolId};

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
use macos as platform;

#[cfg(target_os = "windows")]
mod windows;
#[cfg(target_os = "windows")]
use windows as platform;

/// Emit the credential-helper JSON for Cowork on Windows. Called by the
/// bundled `gate-connect-cowork-helper` binary, which Claude Desktop spawns
/// at request time.
#[cfg(target_os = "windows")]
pub fn windows_helper_emit() -> Result<String> {
    windows::helper_emit()
}

const UPSTREAM_PROVIDER_NAME: &str = "Anthropic";
const DEFAULT_UPSTREAM_URL: &str = "https://api.anthropic.com";

/// Per-tool keychain label for the upstream provider credential.
const UPSTREAM_KEYCHAIN_LABEL: &str = "upstream-credential";

fn upstream_service() -> String {
    keychain::tool_service(ToolId::Cowork.slug(), UPSTREAM_KEYCHAIN_LABEL)
}

pub struct Cowork;

impl Integration for Cowork {
    fn id(&self) -> ToolId {
        ToolId::Cowork
    }

    fn display_name(&self) -> &'static str {
        "Cowork (Claude Desktop)"
    }

    fn upstream_provider_name(&self) -> &'static str {
        UPSTREAM_PROVIDER_NAME
    }

    fn upstream_credential_prefix(&self) -> &'static str {
        // Cowork's user-entered upstream credential is always an Anthropic key
        // — `sk-ant-api*` (API key) or `sk-ant-oat*` (OAuth token) — both of
        // which share this prefix. The session-delegate sentinel is stored via
        // a separate path that does not flow through this validation.
        "sk-ant-"
    }

    fn default_upstream_url(&self) -> &'static str {
        DEFAULT_UPSTREAM_URL
    }

    fn detect(&self) -> Result<bool> {
        platform::detect()
    }

    fn status(&self) -> Result<Status> {
        platform::status()
    }

    fn connect(&self, input: &ConnectInput) -> Result<()> {
        platform::connect(input)
    }

    fn disconnect(&self) -> Result<()> {
        platform::disconnect()
    }

    fn save_upstream_credential(&self, credential: &str) -> Result<()> {
        let trimmed = credential.trim();
        if trimmed.is_empty() {
            anyhow::bail!("credential is empty");
        }
        let user = env::current_user()?;
        keychain::set(&upstream_service(), &user, trimmed)
    }

    fn has_upstream_credential(&self) -> Result<bool> {
        let user = env::current_user()?;
        Ok(keychain::get(&upstream_service(), &user)?.is_some())
    }

    fn clear_upstream_credential(&self) -> Result<()> {
        let user = env::current_user()?;
        keychain::delete(&upstream_service(), &user)?;
        Ok(())
    }
}
