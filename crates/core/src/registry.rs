use anyhow::Result;
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ToolId {
    ClaudeCode,
    Codex,
    OpenCode,
    OpenClaw,
    Hermes,
}

impl ToolId {
    pub const fn slug(self) -> &'static str {
        match self {
            ToolId::ClaudeCode => "claude-code",
            ToolId::Codex => "codex",
            ToolId::OpenCode => "opencode",
            ToolId::OpenClaw => "openclaw",
            ToolId::Hermes => "hermes",
        }
    }

    pub fn from_slug(s: &str) -> Option<Self> {
        match s {
            "claude-code" => Some(ToolId::ClaudeCode),
            "codex" => Some(ToolId::Codex),
            "opencode" => Some(ToolId::OpenCode),
            "openclaw" => Some(ToolId::OpenClaw),
            "hermes" => Some(ToolId::Hermes),
            _ => None,
        }
    }
}

impl fmt::Display for ToolId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.slug())
    }
}

/// Inputs the user (or web deep-link) provides to a connect action.
///
/// `gateway_base_url` comes from the account (entered once at sign-in).
/// `upstream_url` populates `X-Gate-Upstream-Url`. The Gate API key
/// (workspace identity) lives in the account keychain entry and is
/// read by the credential helper at request time.
#[derive(Debug, Clone)]
pub struct ConnectInput {
    pub gateway_base_url: String,
    pub upstream_url: String,
    /// Loopback base URL of the reverse-proxy relay
    /// ([`crate::proxy::relay_base_url`]). Relay-routed integrations point
    /// their tool config here and inject no credential (the relay injects the
    /// live one). `None` when no relay port has been bound yet - a relay-routed
    /// integration then declines to connect.
    pub relay_base_url: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Status {
    NotInstalled,
    Detected,
    Connected,
    Drifted(String),
}

impl fmt::Display for Status {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Status::NotInstalled => f.write_str("not installed"),
            Status::Detected => f.write_str("detected"),
            Status::Connected => f.write_str("connected"),
            Status::Drifted(reason) => write!(f, "drifted: {reason}"),
        }
    }
}

pub trait Integration: Send + Sync {
    fn id(&self) -> ToolId;
    fn display_name(&self) -> &'static str;

    /// Human-readable name of the upstream model provider this tool talks
    /// to natively (e.g. "Anthropic" for Claude Code, "OpenAI" for Codex).
    /// Shown in the connect form so the user knows which API key to enter.
    fn upstream_provider_name(&self) -> &'static str;

    /// Default upstream endpoint URL. The user can override it in the
    /// connect form, but ~all users want the canonical provider URL.
    fn default_upstream_url(&self) -> &'static str;

    /// Does this tool need Gate Connect to store an upstream provider
    /// credential separately? Claude Code, Codex, and OpenCode all bring
    /// their own creds (OAuth token, `ANTHROPIC_API_KEY`, `codex login`,
    /// per-provider `opencode auth`, etc.) and Gate forwards whatever they
    /// send, so they return false. The `true` default is kept for any
    /// future tool that can't authenticate upstream on its own; UI / CLI
    /// use this to decide whether to show the credential-picker step.
    fn requires_upstream_credential(&self) -> bool {
        true
    }

    /// Is the underlying tool installed on this machine?
    fn detect(&self) -> Result<bool>;

    /// Has Gate Connect already configured this tool?
    fn status(&self) -> Result<Status>;

    /// Apply gateway config. Idempotent: a second call with the same
    /// inputs results in the same state. Requires that an upstream
    /// credential has already been saved via `save_upstream_credential`.
    fn connect(&self, input: &ConnectInput) -> Result<()>;

    /// Revert everything `connect` wrote. After this returns the tool
    /// must be back to its prior configuration with zero Gate residue.
    fn disconnect(&self) -> Result<()>;

    /// Persist the upstream provider credential (e.g. Anthropic API key
    /// or Claude OAuth token) to keychain. Replaces any prior value.
    fn save_upstream_credential(&self, credential: &str) -> Result<()>;

    /// Expected prefix for this tool's upstream credential (e.g. "sk-"
    /// for OpenAI). An empty string means no prefix is enforced - the
    /// credential is still length/charset-validated. The IPC layer passes
    /// this to `validate_api_key` so a compromised renderer can't write
    /// arbitrary bytes to a tool's keychain entry under a mismatched slug.
    fn upstream_credential_prefix(&self) -> &'static str {
        ""
    }

    /// Is an upstream credential currently saved for this tool?
    fn has_upstream_credential(&self) -> Result<bool>;

    /// Forget the saved upstream credential. Independent of connect
    /// state - disconnect() does not call this.
    fn clear_upstream_credential(&self) -> Result<()>;
}

pub fn registry() -> Vec<Box<dyn Integration>> {
    vec![
        Box::new(crate::integrations::claude_code::ClaudeCode),
        Box::new(crate::integrations::codex::Codex),
        Box::new(crate::integrations::opencode::OpenCode),
        Box::new(crate::integrations::openclaw::OpenClaw),
        Box::new(crate::integrations::hermes::Hermes),
    ]
}

pub fn find(id: ToolId) -> Option<Box<dyn Integration>> {
    registry().into_iter().find(|i| i.id() == id)
}

/// Disconnect every tool Gate Connect currently manages (Connected or
/// Drifted). Sign-out runs this first - clearing the account while tool
/// configs still embed the key would leave them routing to the gateway
/// with a dead credential on disk. Best-effort across tools; failures
/// are collected into one error so the caller can abort the sign-out.
pub fn disconnect_all_managed() -> Result<()> {
    let mut failures = Vec::new();
    for integ in registry() {
        let managed = match integ.status() {
            Ok(Status::Connected) | Ok(Status::Drifted(_)) => true,
            Ok(_) => false,
            // status() failing (e.g. unparsable config) doesn't prove the
            // tool is clean - attempt the disconnect so a config that still
            // embeds the key aborts the sign-out instead of being skipped.
            Err(_) => true,
        };
        if !managed {
            continue;
        }
        if let Err(e) = integ.disconnect() {
            failures.push(format!("{}: {e}", integ.display_name()));
        }
    }
    if failures.is_empty() {
        return Ok(());
    }
    anyhow::bail!(
        "sign-out stopped: disconnecting these tools failed: {}",
        failures.join("; ")
    )
}
