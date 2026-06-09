//! Gate Connect core. Native primitives + a small registry of integrations.
//!
//! Per the PRD the per-tool logic should ultimately live in a declarative
//! JSON registry served from the Gate API. For this prototype the registry
//! is in-process and contains a single entry (Cowork) so we can validate
//! the connect / disconnect / status mechanism end to end.
//!
//! Cross-platform note: the Cowork integration runs on macOS (via
//! `/Library/Managed Preferences`) and Windows (via the
//! `HKCU\SOFTWARE\Policies\Claude` registry policy). Linux has no Claude
//! Desktop, so Cowork is excluded there. The supporting modules
//! (`migrate`, `claude_session_delegate`) back Cowork flows that only make sense on
//! macOS (standard-mode userData migration; Claude Code session
//! delegation), so they stay macOS-gated. Claude Code and Codex
//! integrations run on macOS, Linux, and Windows.

pub mod account;
pub mod env;
pub mod keychain;
pub mod primitives;
pub mod provider;
pub mod proxy;
pub mod registry;

#[cfg(any(target_os = "macos", target_os = "windows"))]
pub mod claude_session_delegate;
#[cfg(target_os = "macos")]
pub mod migrate;

pub mod integrations {
    pub mod claude_code;
    pub mod codex;
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    pub mod cowork;
    pub mod opencode;
}

pub use registry::{registry, ConnectInput, Integration, Status, ToolId};
