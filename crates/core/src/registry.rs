use anyhow::Result;
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ToolId {
    ClaudeCode,
    Codex,
    OpenCode,
    OpenClaw,
    Hermes,
    /// Not a tool: the proxy variables exported into the user's environment,
    /// which route OpenCode and anything else that reads `HTTPS_PROXY`.
    EnvProxy,
}

impl ToolId {
    pub const fn slug(self) -> &'static str {
        match self {
            ToolId::ClaudeCode => "claude-code",
            ToolId::Codex => "codex",
            ToolId::OpenCode => "opencode",
            ToolId::OpenClaw => "openclaw",
            ToolId::Hermes => "hermes",
            ToolId::EnvProxy => "env-proxy",
        }
    }

    /// The gateway's `agent_framework` id for this tool, or `None` when the
    /// gateway cannot name it.
    ///
    /// **Not the same namespace as [`Self::slug`], even where the strings
    /// coincide.** The slug is ours: it names a tool this app can configure, and
    /// it is what the desktop app stamps on `x-gate-client` after guessing from a
    /// User-Agent. A platform id is the gateway's: `platform-registry.ts` derives
    /// it per request from evidence a client cannot as easily fake, and it is
    /// what `gateway_requests.agent_framework` holds.
    ///
    /// Four of ours line up with one of theirs and one does not, which is exactly
    /// why this is a written-out match rather than passing the slug through.
    /// Passing it through would work today for four tools and 400 for Hermes, and
    /// would silently break the day either side renames anything. Being a match
    /// on the enum, adding a tool forces whoever adds it to answer this question.
    ///
    /// `None` is not a gap to fill in later by guessing. A model preference keyed
    /// on a platform the gateway never stamps could never be enforced, so the
    /// pane must decline to offer the choice rather than store one that does
    /// nothing - see `tool_models`.
    pub const fn platform_id(self) -> Option<&'static str> {
        match self {
            ToolId::ClaudeCode => Some("claude-code"),
            ToolId::Codex => Some("codex"),
            ToolId::OpenCode => Some("opencode"),
            ToolId::OpenClaw => Some("openclaw"),
            // Absent from the gateway's registry: nothing there detects Hermes,
            // so its requests are attributed to `direct-api` and a preference
            // keyed on it would have nothing to match.
            ToolId::Hermes => None,
            // Not a tool at all - the exported proxy variables. Whatever routes
            // through them is attributed as itself.
            ToolId::EnvProxy => None,
        }
    }

    pub fn from_slug(s: &str) -> Option<Self> {
        match s {
            "claude-code" => Some(ToolId::ClaudeCode),
            "codex" => Some(ToolId::Codex),
            "opencode" => Some(ToolId::OpenCode),
            "openclaw" => Some(ToolId::OpenClaw),
            "hermes" => Some(ToolId::Hermes),
            "env-proxy" => Some(ToolId::EnvProxy),
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
    /// Loopback URL of the MITM engine's forward proxy
    /// ([`crate::proxy::engine_proxy_url`]), for integrations that hand their
    /// whole egress to a proxy rather than repointing a base URL. `None` unless
    /// the proxy is actually routing, so such an integration declines to
    /// connect rather than stranding the tool with no network.
    pub engine_proxy_url: Option<String>,
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

    /// Does the tool's current on-disk config carry Gate Connect's own
    /// management marker? Distinguishes drift in config *we* wrote (a stale
    /// scheme from an older build, a changed relay port) from a setup the
    /// user made by hand out-of-app. [`crate::provider::reconcile_enabled`]
    /// only auto-reapplies a `Drifted` tool when this is true; the
    /// conservative default keeps integrations without a marker out of the
    /// auto-reapply path.
    fn config_is_managed(&self) -> Result<bool> {
        Ok(false)
    }

    /// The file this integration rewrites, for the copy that tells the user what
    /// is about to change - "Gate will update ~/.codex/config.toml".
    ///
    /// A display string rather than a `PathBuf`: the only caller is UI copy, and
    /// resolving it can fail (no home directory), which is not worth propagating
    /// into a sentence. `None` when the integration edits nothing a single path
    /// names - the environment channel writes machine-wide settings, not a file
    /// of its own - and callers then name the tool without a location rather than
    /// inventing one.
    ///
    /// Not a secret: it is a path in the user's own home directory, and the point
    /// of showing it is that they can go and read it.
    fn config_location(&self) -> Option<String> {
        None
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

    /// Keep this tool out of the popover's ledger.
    ///
    /// The integration stays in [`registry`] regardless, so
    /// [`disconnect_all_managed`], the master-off sweep and the restore path
    /// still cover anyone who connected it with an earlier build - removing it
    /// from the registry would strand their config pointing at a relay they
    /// can no longer turn off. The `gate-connect` CLI also keeps listing it.
    ///
    /// Used for integrations whose config strategy has not been validated
    /// against the tool's current documentation; see
    /// `docs/harness-integration-validation.md`.
    fn hidden_in_ui(&self) -> bool {
        false
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
        Box::new(crate::integrations::env_proxy::EnvProxy),
    ]
}

/// Integrations kept out of the popover's ledger. Present in [`registry`] so
/// cleanup still reaches them; see [`Integration::hidden_in_ui`].
pub fn hidden_in_ui_slugs() -> Vec<&'static str> {
    registry()
        .iter()
        .filter(|i| i.hidden_in_ui())
        .map(|i| i.id().slug())
        .collect()
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Hiding must stay a UI decision. If these ever leave `registry()`, a user
    /// who connected them with an earlier build loses every path that could
    /// disconnect them: the master-off sweep, sign-out, and the restore.
    #[test]
    fn hidden_integrations_are_still_registered() {
        let slugs: Vec<&str> = registry().iter().map(|i| i.id().slug()).collect();
        for hidden in hidden_in_ui_slugs() {
            assert!(
                slugs.contains(&hidden),
                "{hidden} is hidden but missing from the registry, so nothing can clean it up"
            );
        }
    }

    /// The multi-provider harnesses are in the ledger. They were hidden while
    /// their config strategy was a base-URL rewrite that something else could
    /// override; OpenClaw and Hermes now route through the proxy engine, and
    /// OpenCode's remaining exposure (a project config outranking the global
    /// one) is covered by the environment channel, which routes it whatever
    /// `opencode.json` says.
    ///
    /// Still true and not pinned here: none of the three has been exercised
    /// end-to-end against a real install. See docs/routing-architecture.md.
    #[test]
    fn agent_harnesses_are_listed() {
        let hidden = hidden_in_ui_slugs();
        for slug in ["opencode", "openclaw", "hermes"] {
            assert!(
                !hidden.contains(&slug),
                "{slug} should be in the ledger, got hidden: {hidden:?}"
            );
        }
    }

    /// The single-provider integrations, whose config strategy is a plain
    /// documented override, stay visible.
    #[test]
    fn single_provider_integrations_stay_visible() {
        let hidden = hidden_in_ui_slugs();
        for slug in ["claude-code", "codex"] {
            assert!(!hidden.contains(&slug), "{slug} must remain in the ledger");
        }
    }

    /// The environment channel is registered like anything else, so sign-out and
    /// the master-off sweep reach it. It is hidden for a different reason than
    /// the harnesses: not "unvalidated", but "no correct home in a ledger that
    /// groups by model family" - it is a mechanism, not a tool.
    #[test]
    fn the_environment_channel_is_registered_and_hidden() {
        let slugs: Vec<&str> = registry().iter().map(|i| i.id().slug()).collect();
        assert!(
            slugs.contains(&"env-proxy"),
            "env-proxy must be registered so cleanup reaches it, got {slugs:?}"
        );
        assert!(hidden_in_ui_slugs().contains(&"env-proxy"));
        assert_eq!(ToolId::from_slug("env-proxy"), Some(ToolId::EnvProxy));
    }

    /// The platform id is a *different namespace* from the slug, and this is the
    /// test that keeps the coincidence from being mistaken for a rule.
    ///
    /// Four of our slugs happen to equal a gateway platform id and one does not.
    /// Anyone who reads only the four would reasonably conclude the mapping is
    /// the identity and replace it with `slug()`, which would 400 for Hermes and
    /// break silently the day either side renames anything.
    #[test]
    fn platform_ids_are_not_just_the_slug() {
        // Where they coincide, they must coincide exactly: a preference is stored
        // under this string and matched against `agent_framework`.
        for tool in [
            ToolId::ClaudeCode,
            ToolId::Codex,
            ToolId::OpenCode,
            ToolId::OpenClaw,
        ] {
            assert_eq!(tool.platform_id(), Some(tool.slug()), "{tool}");
        }

        // And where they do not, there is no id to invent. Nothing in the
        // gateway's registry detects Hermes, so its traffic is attributed to
        // `direct-api`; the environment channel is not a tool at all. A
        // preference keyed on either could never be applied, which is why the
        // app pane withholds the control rather than storing a choice that does
        // nothing.
        assert_eq!(ToolId::Hermes.platform_id(), None);
        assert_eq!(ToolId::EnvProxy.platform_id(), None);
    }
}
