//! Provider abstraction: one user-facing switch per model provider that
//! orchestrates both the config-level tool integrations ([`registry`]) and -
//! on every platform with the proxy subsystem (macOS, Windows, Linux), only
//! when the system proxy is already running - the matching proxy domains
//! ([`crate::proxy`]). This is the layer that lets the UI show a single
//! "OpenAI / Codex" toggle instead of exposing the proxy-vs-config split.
//!
//! Policy (see [`enable_plan`]): config-first, proxy-if-already-on. Flipping a
//! provider on always configures its installed tools (Codex edits
//! `~/.codex/config.toml`, no proxy/CA needed); it additionally flips the
//! provider's proxy domains only when the proxy is already running, so the
//! switch never triggers a CA / admin prompt on its own.

use anyhow::{Context, Result};
use serde::Serialize;
use std::fs;
use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard};

use crate::account;
use crate::registry::{self, ConnectInput, Status, ToolId};

/// A user-facing provider: the union of the config integrations and proxy
/// domains that route one model provider through Gate.
pub struct Provider {
    /// Stable identifier used by the Tauri commands. Shares the proxy domain
    /// slug where they line up (e.g. "openai") but is its own namespace.
    pub slug: &'static str,
    pub display_name: &'static str,
    pub subtitle: &'static str,
    /// Config integrations to connect/disconnect (cross-platform).
    pub tool_ids: &'static [ToolId],
    /// Proxy domains to flip when the proxy is running (macOS / Windows /
    /// Linux, best-effort).
    pub proxy_domain_slugs: &'static [&'static str],
}

/// Built-in provider catalog. Claude leads, then OpenAI/Codex; both follow the
/// same one-switch model and others can be added the same way.
///
/// Mapping note: each provider lists only the tools that need *config* editing
/// to route (a CLI that ignores the system proxy - Claude Code, Codex). Desktop
/// apps that honor the system proxy (Cowork / Claude Desktop) ride the proxy
/// domain instead, so they're covered by `proxy_domain_slugs` without a
/// credential or sudo prompt. That's why Cowork isn't in `tool_ids`. A provider
/// with no native CLI integration (OpenRouter) is proxy-only: empty `tool_ids`,
/// routed entirely through its proxy domain.
pub fn providers() -> Vec<Provider> {
    vec![
        Provider {
            slug: "anthropic",
            display_name: "Claude",
            subtitle: "Claude Code + Claude Desktop",
            tool_ids: &[ToolId::ClaudeCode],
            // Only the api.anthropic.com domain. The `claude-web` chat domain is
            // deliberately absent: `enable` below turns on EVERY domain a
            // provider lists, so adding it here would start intercepting the
            // user's claude.ai session the moment they enabled Claude. That
            // surface is reached through its own domain toggle instead.
            proxy_domain_slugs: &["anthropic"],
        },
        Provider {
            slug: "openai",
            display_name: "OpenAI",
            subtitle: "Codex + OpenAI API",
            tool_ids: &[ToolId::Codex],
            proxy_domain_slugs: &["openai"],
        },
        Provider {
            slug: "openrouter",
            display_name: "OpenRouter",
            subtitle: "OpenRouter API",
            // Proxy-only: OpenRouter has no Gate Connect CLI integration, so it
            // routes entirely through the proxy domain (requires the proxy to
            // be running, like Cowork).
            tool_ids: &[],
            proxy_domain_slugs: &["openrouter"],
        },
    ]
}

pub fn find(slug: &str) -> Option<Provider> {
    providers().into_iter().find(|p| p.slug == slug)
}

/// UI snapshot of one provider.
#[derive(Debug, Clone, Serialize)]
pub struct ProviderState {
    pub slug: String,
    pub display_name: String,
    pub subtitle: String,
    /// Headline on/off: at least one of the provider's tool integrations is
    /// configured to route through Gate.
    pub enabled: bool,
    /// Whether the switch can do anything right now: a tool is installed (the
    /// config route) or the proxy is running (the domain route). When false
    /// the UI should render the switch disabled.
    pub available: bool,
    /// Slugs of the config-file tools this provider's switch governs, so the
    /// UI can show the coupling between the provider switch and the per-tool
    /// switches.
    pub tool_slugs: Vec<String>,
    /// Slugs of the proxy domains this provider covers. With `tool_slugs` this
    /// is a family's whole membership, which is what the popover's ledger
    /// groups by - keyed on real ids rather than the display prose in
    /// `Integration::upstream_provider_name`, which is deliberately "your
    /// existing providers" for the multi-provider tools.
    pub domain_slugs: Vec<String>,
}

/// What [`enable`] should do, given the two facts that drive the locked
/// "config-first, proxy-if-already-on" policy. Pure so it can be unit-tested
/// without touching the keychain, config files, or the proxy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct EnablePlan {
    /// Run the config integration(s) for installed tools.
    configure_tool: bool,
    /// Flip the provider's proxy domains on .
    enable_domain: bool,
    /// Neither mechanism can act - surface a helpful error instead.
    nothing: bool,
}

fn enable_plan(tool_detected: bool, proxy_running: bool) -> EnablePlan {
    EnablePlan {
        configure_tool: tool_detected,
        enable_domain: proxy_running,
        nothing: !tool_detected && !proxy_running,
    }
}

/// Is the system proxy currently running? Always false on platforms without
/// the proxy subsystem.
#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
fn proxy_running() -> bool {
    crate::proxy::manager()
        .status()
        .map(|s| s.running)
        .unwrap_or(false)
}
#[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
fn proxy_running() -> bool {
    false
}

/// True if any of the provider's proxy domains is currently enabled in the
/// proxy catalog. A provider with no config tools (proxy-only, e.g.
/// OpenRouter) relies on this for its headline on/off state - without it the
/// switch would always read off. Always false on platforms without the proxy
/// subsystem.
#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
fn proxy_domains_enabled(p: &Provider) -> bool {
    if p.proxy_domain_slugs.is_empty() {
        return false;
    }
    crate::proxy::manager()
        .status()
        .map(|s| {
            s.domains
                .iter()
                .any(|d| d.enabled && p.proxy_domain_slugs.contains(&d.slug.as_str()))
        })
        .unwrap_or(false)
}
#[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
fn proxy_domains_enabled(_p: &Provider) -> bool {
    false
}

fn tool_detected(id: ToolId) -> bool {
    registry::find(id)
        .and_then(|i| i.detect().ok())
        .unwrap_or(false)
}

fn tool_connected(id: ToolId) -> bool {
    registry::find(id)
        .and_then(|i| i.status().ok())
        .map(|s| matches!(s, Status::Connected))
        .unwrap_or(false)
}

/// Current state of one provider for the UI. A provider reads as on when any
/// of its config tools is connected *or* any of its proxy domains is enabled -
/// so a proxy-only provider (OpenRouter) reflects its domain, and a config
/// provider that's also riding the proxy still reads on.
pub fn state(p: &Provider) -> ProviderState {
    let enabled = p.tool_ids.iter().any(|&id| tool_connected(id)) || proxy_domains_enabled(p);
    let any_detected = p.tool_ids.iter().any(|&id| tool_detected(id));
    ProviderState {
        slug: p.slug.into(),
        display_name: p.display_name.into(),
        subtitle: p.subtitle.into(),
        enabled,
        available: any_detected || proxy_running(),
        tool_slugs: p.tool_ids.iter().map(|id| id.slug().to_string()).collect(),
        domain_slugs: p.proxy_domain_slugs.iter().map(|s| s.to_string()).collect(),
    }
}

/// State of every provider in the catalog.
pub fn list() -> Vec<ProviderState> {
    providers().iter().map(state).collect()
}

/// Turn a provider on. Configures installed tools and, if the proxy is already
/// running, enables the provider's proxy domains. Requires a signed-in
/// account. Idempotent - re-running re-applies the same config.
pub fn enable(slug: &str) -> Result<ProviderState> {
    let p = find(slug).with_context(|| format!("unknown provider {slug:?}"))?;
    let account = account::load()?
        .context("no Gate account configured - sign in before enabling a provider")?;

    let any_detected = p.tool_ids.iter().any(|&id| tool_detected(id));
    let plan = enable_plan(any_detected, proxy_running());

    if plan.nothing {
        anyhow::bail!(
            "nothing to configure for {}: install its app, or turn on \
             \u{201c}Route through Gate\u{201d} to route it through the proxy",
            p.display_name
        );
    }

    if plan.configure_tool {
        for &id in p.tool_ids {
            let Some(integ) = registry::find(id) else {
                continue;
            };
            if !integ.detect().unwrap_or(false) {
                continue; // tool not installed - nothing to configure
            }
            let input = ConnectInput {
                gateway_base_url: account.gateway_base_url.clone(),
                upstream_url: integ.default_upstream_url().to_string(),
                relay_base_url: crate::proxy::relay_base_url(),
            };
            integ
                .connect(&input)
                .with_context(|| format!("configuring {}", integ.display_name()))?;
        }
    }

    // Record the on state durably so a later reconcile ([`reconcile_enabled`])
    // or reboot re-applies the provider. When the engine is live, route the
    // change through the manager so routing also starts immediately; otherwise
    // persist the flag directly. Mirrors [`disable`], which always persists the
    // off-intent regardless of proxy state.
    #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
    for domain in p.proxy_domain_slugs {
        if plan.enable_domain {
            crate::proxy::manager()
                .set_domain(domain, true)
                .with_context(|| format!("enabling proxy domain {domain:?}"))?;
        } else {
            crate::proxy::config::set_enabled(domain, true)
                .with_context(|| format!("persisting proxy domain {domain:?}"))?;
        }
    }

    Ok(state(&p))
}

/// Turn a provider off. Reverts the config integration(s) and, if the proxy is
/// running, disables the provider's proxy domains. Promptless and idempotent.
pub fn disable(slug: &str) -> Result<ProviderState> {
    let p = find(slug).with_context(|| format!("unknown provider {slug:?}"))?;

    for &id in p.tool_ids {
        let Some(integ) = registry::find(id) else {
            continue;
        };
        let connected = matches!(integ.status(), Ok(Status::Connected | Status::Drifted(_)));
        if connected || integ.detect().unwrap_or(false) {
            integ
                .disconnect()
                .with_context(|| format!("disconnecting {}", integ.display_name()))?;
        }
    }

    // Record the off state durably so a later reconcile ([`reconcile_enabled`])
    // won't treat the provider as still-on and re-apply it. When the engine is
    // live, route the change through the manager so routing also stops
    // immediately; otherwise persist the flag directly (the config-route tools
    // don't need the proxy running to be turned off).
    #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
    for domain in p.proxy_domain_slugs {
        // Best-effort: an already-off or unknown domain isn't an error.
        let _ = if proxy_running() {
            crate::proxy::manager()
                .set_domain(domain, false)
                .map(|_| ())
        } else {
            crate::proxy::config::set_enabled(domain, false).map(|_| ())
        };
    }

    Ok(state(&p))
}

/// Persisted (on-disk) view of whether any of the provider's proxy domains are
/// enabled - the durable "the user wants this provider on" signal, readable even
/// when the proxy engine is stopped. Distinct from [`proxy_domains_enabled`],
/// which reflects the live engine's current domain set.
#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
fn domains_enabled_persisted(p: &Provider) -> bool {
    crate::proxy::config::load_domains()
        .map(|ds| {
            ds.iter()
                .any(|d| d.enabled && p.proxy_domain_slugs.contains(&d.slug.as_str()))
        })
        .unwrap_or(false)
}
#[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
fn domains_enabled_persisted(_p: &Provider) -> bool {
    false
}

/// Configure any installed-but-unconfigured tool of a provider the user has
/// turned on. Closes the "installed the tool *after* enabling the provider" gap:
/// [`enable`] only wires up tools present at that instant, so a tool that shows
/// up later stays unrouted until this runs (at startup). Idempotent and
/// best-effort - one tool's failure never strands the rest.
///
/// Only tools that carry their own upstream credential (`requires_upstream_credential
/// == false`, e.g. Claude Code) are auto-applied; a tool that needs a
/// Gate-stored key is left for the explicit connect flow. Tools in
/// [`Status::Detected`] (installed, no Gate config) are connected; a
/// [`Status::Drifted`] tool is *re*-connected only when its config carries our
/// own management marker ([`Integration::config_is_managed`]) - i.e. the stale
/// values are ours (an old scheme, a changed relay port), not a setup the user
/// made by hand - and the relay is up so there's a live base URL to point it
/// at. Unmarked drift is left alone so this never clobbers an out-of-app setup.
pub fn reconcile_enabled() -> Result<()> {
    let Some(account) = account::load()? else {
        return Ok(()); // no gateway configured yet - nothing to point tools at
    };
    let relay_base_url = crate::proxy::relay_base_url();
    for p in providers() {
        if !domains_enabled_persisted(&p) {
            continue;
        }
        for &id in p.tool_ids {
            let Some(integ) = registry::find(id) else {
                continue;
            };
            if integ.requires_upstream_credential() {
                continue; // needs a stored key; not safe to auto-apply
            }
            let reapply = match integ.status() {
                Ok(Status::Detected) => true,
                // Our own writes gone stale - safe to reassert, but only with
                // a relay to point at (connect() bails without one, and this
                // drift may *be* "relay not enabled yet").
                Ok(Status::Drifted(_)) => {
                    relay_base_url.is_some() && integ.config_is_managed().unwrap_or(false)
                }
                _ => false, // NotInstalled / Connected / status error - leave as-is
            };
            if !reapply {
                continue;
            }
            let input = ConnectInput {
                gateway_base_url: account.gateway_base_url.clone(),
                upstream_url: integ.default_upstream_url().to_string(),
                relay_base_url: relay_base_url.clone(),
            };
            if let Err(e) = integ.connect(&input) {
                eprintln!(
                    "[gate] auto-configuring {} failed: {e:#}",
                    integ.display_name()
                );
            }
        }
    }
    Ok(())
}

// ---- Global kill / restore (the "Route through Gate" master switch) ----
//
// Turning the master off should stop *all* routing - including config-based
// providers like Codex, which the proxy never touched. We snapshot which
// providers were on, disconnect them, then (the caller) stops the proxy.
// Turning the master back on re-applies that snapshot, so the user's apps come
// back exactly as they were.

/// Provider slugs to re-enable on master-on.
const PROVIDER_SNAPSHOT: &str = "restore-snapshot.json";
/// Tool slugs no provider maps (OpenCode and friends), disconnected by the
/// master-off sweep and reconnected alongside the provider snapshot.
const SWEPT_TOOLS_SNAPSHOT: &str = "restore-tools-snapshot.json";

fn snapshot_path(file: &str) -> Result<PathBuf> {
    Ok(crate::env::app_support_dir()?.join("provider").join(file))
}

fn save_snapshot(file: &str, slugs: &[String]) -> Result<()> {
    let path = snapshot_path(file)?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
    }
    let raw = serde_json::to_string(slugs).context("serializing provider snapshot")?;
    fs::write(&path, raw).with_context(|| format!("writing {}", path.display()))
}

fn load_snapshot(file: &str) -> Result<Vec<String>> {
    let path = snapshot_path(file)?;
    match fs::read_to_string(&path) {
        Ok(raw) => Ok(serde_json::from_str(&raw).unwrap_or_default()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(e) => Err(e).with_context(|| format!("reading {}", path.display())),
    }
}

fn clear_snapshot(file: &str) -> Result<()> {
    let path = snapshot_path(file)?;
    match fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e).with_context(|| format!("removing {}", path.display())),
    }
}

/// Serializes the master-off/on flows. All of them read-modify-write the
/// restore snapshots, so an interleaved tray-quit teardown and master toggle
/// could otherwise clobber each other's snapshot mid-flight. Poisoning is
/// ignored: the snapshots are plain files and every flow is retryable.
static MASTER_FLOW_LOCK: Mutex<()> = Mutex::new(());

fn master_flow_guard() -> MutexGuard<'static, ()> {
    MASTER_FLOW_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// The provider half of master-off: record every currently-enabled provider,
/// then disconnect them all. Runs *before* the proxy stops so each provider's
/// domain is still flippable. Best-effort per provider so one failure can't
/// strand the rest. The snapshot survives until the master is turned back on.
///
/// Deliberately not public: on its own it only covers tools the catalog
/// claims, and every caller wants [`snapshot_and_disable_everything`]. Master
/// off used to call the provider pass alone, which is how OpenCode and friends
/// ended up stranded on a dead relay.
fn snapshot_and_disable_all_locked() -> Result<()> {
    let enabled: Vec<String> = list()
        .into_iter()
        .filter(|p| p.enabled)
        .map(|p| p.slug)
        .collect();
    // Union with any existing snapshot rather than overwrite: an existing
    // file is a pending restore, and a second off-flow (e.g. a quit teardown
    // right after a master-off) sees fewer - possibly zero - enabled
    // providers, which would otherwise shrink the restore set to nothing.
    let mut snapshot = load_snapshot(PROVIDER_SNAPSHOT)?;
    for slug in &enabled {
        if !snapshot.contains(slug) {
            snapshot.push(slug.clone());
        }
    }
    save_snapshot(PROVIDER_SNAPSHOT, &snapshot)?;
    for slug in &enabled {
        if let Err(e) = disable(slug) {
            eprintln!("[gate] disabling provider {slug:?} during master-off failed: {e}");
        }
    }
    Ok(())
}

/// Master OFF, in full: the provider snapshot + disable, then a sweep that
/// disconnects every registry tool still managed (Connected or Drifted)
/// afterwards - standalone tools no provider maps (OpenCode and friends), and
/// provider tools the provider pass missed (a drifted config, a failed
/// disable). Their configs point at the loopback relay, which dies with the
/// engine. Swept tools are recorded in their own snapshot so [`restore_all`]
/// reconnects them alongside the providers. Best-effort per tool, mirroring
/// the provider pass.
///
/// Both master-off paths use this: the routing switch and the quit-time "turn
/// off integrations and quit" choice. They are the same event as far as the
/// user's tools are concerned - the relay stops either way - and using the
/// narrower [`snapshot_and_disable_all`] for the switch left the harnesses
/// pointed at a dead port while the UI reported "not routing".
pub fn snapshot_and_disable_everything() -> Result<()> {
    let _guard = master_flow_guard();
    snapshot_and_disable_all_locked()?;
    let mut disconnected = Vec::new();
    for integ in registry::registry() {
        if !matches!(integ.status(), Ok(Status::Connected | Status::Drifted(_))) {
            continue;
        }
        match integ.disconnect() {
            Ok(()) => disconnected.push(integ.id().slug().to_string()),
            Err(e) => eprintln!(
                "[gate] disconnecting {} during quit failed: {e}",
                integ.display_name()
            ),
        }
    }
    // Union for the same reason as the provider snapshot.
    let mut snapshot = load_snapshot(SWEPT_TOOLS_SNAPSHOT)?;
    for slug in disconnected {
        if !snapshot.contains(&slug) {
            snapshot.push(slug);
        }
    }
    save_snapshot(SWEPT_TOOLS_SNAPSHOT, &snapshot)
}

/// Master ON: re-enable every provider that was on when routing was last
/// turned off, then reconnect any standalone tools the master-off sweep
/// disconnected. Entries that fail to restore stay in their snapshot so a
/// later call can retry them; each snapshot is cleared once everything in it
/// is back. Idempotent; a missing snapshot is a no-op. Callers run this twice
/// per master-on: once before the proxy comes up (config-based tools, and the
/// engine's "at least one provider" precondition) and once after (domain-only
/// providers, which have nothing to configure until the proxy is running).
pub fn restore_all() -> Result<()> {
    let _guard = master_flow_guard();
    let mut failed = Vec::new();
    for slug in load_snapshot(PROVIDER_SNAPSHOT)? {
        if let Err(e) = enable(&slug) {
            eprintln!("[gate] restoring provider {slug:?} on master-on failed: {e}");
            failed.push(slug);
        }
    }
    if failed.is_empty() {
        clear_snapshot(PROVIDER_SNAPSHOT)?;
    } else {
        save_snapshot(PROVIDER_SNAPSHOT, &failed)?;
    }
    restore_swept_tools()
}

/// Reconnect the standalone tools the master-off sweep disconnected (see
/// [`snapshot_and_disable_everything`]). Same retry semantics as the
/// provider snapshot: failures stay recorded, the file clears once every tool
/// is back. Tools uninstalled (or slugs unknown) since the quit are dropped.
/// Signed out since the quit: leave the snapshot for a later signed-in
/// restore - there's no gateway to point the tools at.
fn restore_swept_tools() -> Result<()> {
    let slugs = load_snapshot(SWEPT_TOOLS_SNAPSHOT)?;
    if slugs.is_empty() {
        return Ok(());
    }
    let Some(account) = account::load()? else {
        return Ok(());
    };
    let relay_base_url = crate::proxy::relay_base_url();
    let mut failed = Vec::new();
    for slug in slugs {
        let Some(integ) = ToolId::from_slug(&slug).and_then(registry::find) else {
            continue;
        };
        if !integ.detect().unwrap_or(false) {
            continue;
        }
        let input = ConnectInput {
            gateway_base_url: account.gateway_base_url.clone(),
            upstream_url: integ.default_upstream_url().to_string(),
            relay_base_url: relay_base_url.clone(),
        };
        if let Err(e) = integ.connect(&input) {
            eprintln!("[gate] restoring tool {slug:?} on master-on failed: {e:#}");
            failed.push(slug);
        }
    }
    if failed.is_empty() {
        clear_snapshot(SWEPT_TOOLS_SNAPSHOT)
    } else {
        save_snapshot(SWEPT_TOOLS_SNAPSHOT, &failed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn openai_provider_maps_to_codex_and_openai_domain() {
        let p = find("openai").expect("openai provider present");
        assert_eq!(p.display_name, "OpenAI");
        assert!(p.tool_ids.contains(&ToolId::Codex));
        assert_eq!(p.proxy_domain_slugs, &["openai"]);
    }

    #[test]
    fn anthropic_provider_maps_to_claude_code_and_anthropic_domain() {
        let p = find("anthropic").expect("anthropic provider present");
        assert_eq!(p.display_name, "Claude");
        assert!(p.tool_ids.contains(&ToolId::ClaudeCode));
        assert_eq!(p.proxy_domain_slugs, &["anthropic"]);
    }

    #[test]
    fn openrouter_provider_is_proxy_only() {
        let p = find("openrouter").expect("openrouter provider present");
        assert_eq!(p.display_name, "OpenRouter");
        assert!(
            p.tool_ids.is_empty(),
            "OpenRouter has no CLI integration - it's proxy-only"
        );
        assert_eq!(p.proxy_domain_slugs, &["openrouter"]);
    }

    #[test]
    fn claude_is_listed_before_codex() {
        let slugs: Vec<&str> = providers().iter().map(|p| p.slug).collect();
        let claude = slugs.iter().position(|&s| s == "anthropic");
        let openai = slugs.iter().position(|&s| s == "openai");
        assert!(
            claude < openai,
            "Claude must precede OpenAI/Codex in the catalog: {slugs:?}"
        );
    }

    #[test]
    fn find_unknown_is_none() {
        assert!(find("does-not-exist").is_none());
    }

    #[test]
    fn enable_plan_config_first_proxy_if_running() {
        // Codex installed + proxy on: do both.
        assert_eq!(
            enable_plan(true, true),
            EnablePlan {
                configure_tool: true,
                enable_domain: true,
                nothing: false
            }
        );
        // Codex installed, proxy off: config only, no proxy prompt.
        assert_eq!(
            enable_plan(true, false),
            EnablePlan {
                configure_tool: true,
                enable_domain: false,
                nothing: false
            }
        );
        // No Codex but proxy on: just the domain route.
        assert_eq!(
            enable_plan(false, true),
            EnablePlan {
                configure_tool: false,
                enable_domain: true,
                nothing: false
            }
        );
        // Nothing installed and proxy off: nothing to do.
        assert_eq!(
            enable_plan(false, false),
            EnablePlan {
                configure_tool: false,
                enable_domain: false,
                nothing: true
            }
        );
    }
    #[test]
    fn claude_web_is_not_reachable_by_enabling_the_anthropic_provider() {
        // `enable` flips every domain a provider lists. Attaching the chat
        // domain here would route the user's claude.ai SESSION cookie as a side
        // effect of enabling Claude, bypassing that domain's opt-in default.
        let p = find("anthropic").expect("anthropic provider present");
        assert!(!p.proxy_domain_slugs.contains(&"claude-web"));
        assert_eq!(p.proxy_domain_slugs, &["anthropic"]);
    }
}
