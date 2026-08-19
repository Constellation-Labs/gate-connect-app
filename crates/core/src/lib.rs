//! Gate Connect core. Native primitives + a small registry of integrations.
//!
//! Per the PRD the per-tool logic should ultimately live in a declarative
//! JSON registry served from the Gate API. For this prototype the registry
//! is in-process.
//!
//! Cross-platform note: the config integrations (Claude Code, Codex,
//! OpenCode) run on macOS, Linux, and Windows. Claude Desktop / Cowork has
//! no config integration - it routes through the built-in proxy's
//! `anthropic` domain instead (see [`proxy`]).

pub mod account;
/// The AG-572 activity overview fetch, behind the Overview pane.
pub mod activity;
/// The last overview that landed, held on disk so the pane opens on numbers.
pub mod activity_cache;
pub mod audit;
/// Read-only snapshot of this install, for the copy-pasteable support report.
pub mod diagnostics;
pub mod env;
pub mod keychain;
pub mod oauth;
pub mod org;
/// Non-secret user choices from Settings, defaulting to on.
pub mod preferences;
pub mod primitives;
pub mod provider;
pub mod proxy;
pub mod registry;
/// What a tool is *doing*, as opposed to what its config says. Kept separate
/// from [`registry::Status`] on purpose - see the module docs.
pub mod routing_health;

pub mod integrations {
    pub mod claude_code;
    pub mod codex;
    /// Managed `.env` edits, shared by the proxy-routed harnesses.
    pub(crate) mod dotenv;
    /// Not a tool: the environment channel itself, as a thing users can decline.
    pub mod env_proxy;
    pub mod hermes;
    pub mod openclaw;
    pub mod opencode;
}

pub use registry::{registry, ConnectInput, Integration, Status, ToolId};
