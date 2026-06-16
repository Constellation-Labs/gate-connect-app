//! Provider abstraction: one user-facing switch per model provider that
//! orchestrates both the config-level tool integrations ([`registry`]) and —
//! on every platform with the proxy subsystem (macOS, Windows, Linux), only
//! when the system proxy is already running — the matching proxy domains
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
/// to route (a CLI that ignores the system proxy — Claude Code, Codex). Desktop
/// apps that honor the system proxy (Cowork / Claude Desktop) ride the proxy
/// domain instead, so they're covered by `proxy_domain_slugs` without a
/// credential or sudo prompt. That's why Cowork isn't in `tool_ids`. A provider
/// with no native CLI integration (OpenRouter) is proxy-only: empty `tool_ids`,
/// routed entirely through its proxy domain.
pub fn providers() -> Vec<Provider> {
    vec![
        Provider {
            slug: "anthropic",
            display_name: "Claude Code / Cowork",
            subtitle: "Claude Code + Claude Desktop",
            tool_ids: &[ToolId::ClaudeCode],
            proxy_domain_slugs: &["anthropic"],
        },
        Provider {
            slug: "openai",
            display_name: "OpenAI / Codex",
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
    /// Neither mechanism can act — surface a helpful error instead.
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
/// OpenRouter) relies on this for its headline on/off state — without it the
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
/// of its config tools is connected *or* any of its proxy domains is enabled —
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
    }
}

/// State of every provider in the catalog.
pub fn list() -> Vec<ProviderState> {
    providers().iter().map(state).collect()
}

/// Turn a provider on. Configures installed tools and, if the proxy is already
/// running, enables the provider's proxy domains. Requires a signed-in
/// account. Idempotent — re-running re-applies the same config.
pub fn enable(slug: &str) -> Result<ProviderState> {
    let p = find(slug).with_context(|| format!("unknown provider {slug:?}"))?;
    let account = account::load()?
        .context("no Gate account configured — sign in before enabling a provider")?;

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
                continue; // tool not installed — nothing to configure
            }
            let input = ConnectInput {
                gateway_base_url: account.gateway_base_url.clone(),
                upstream_url: integ.default_upstream_url().to_string(),
            };
            integ
                .connect(&input)
                .with_context(|| format!("configuring {}", integ.display_name()))?;
        }
    }

    #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
    if plan.enable_domain {
        for domain in p.proxy_domain_slugs {
            crate::proxy::manager()
                .set_domain(domain, true)
                .with_context(|| format!("enabling proxy domain {domain:?}"))?;
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

    #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
    if proxy_running() {
        for domain in p.proxy_domain_slugs {
            // Best-effort: an already-off or unknown domain isn't an error.
            let _ = crate::proxy::manager().set_domain(domain, false);
        }
    }

    Ok(state(&p))
}

// ---- Global kill / restore (the "Route through Gate" master switch) ----
//
// Turning the master off should stop *all* routing — including config-based
// providers like Codex, which the proxy never touched. We snapshot which
// providers were on, disconnect them, then (the caller) stops the proxy.
// Turning the master back on re-applies that snapshot, so the user's apps come
// back exactly as they were.

fn snapshot_path() -> Result<PathBuf> {
    Ok(crate::env::app_support_dir()?
        .join("provider")
        .join("restore-snapshot.json"))
}

fn save_snapshot(slugs: &[String]) -> Result<()> {
    let path = snapshot_path()?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
    }
    let raw = serde_json::to_string(slugs).context("serializing provider snapshot")?;
    fs::write(&path, raw).with_context(|| format!("writing {}", path.display()))
}

fn load_snapshot() -> Result<Vec<String>> {
    let path = snapshot_path()?;
    match fs::read_to_string(&path) {
        Ok(raw) => Ok(serde_json::from_str(&raw).unwrap_or_default()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(e) => Err(e).with_context(|| format!("reading {}", path.display())),
    }
}

fn clear_snapshot() -> Result<()> {
    let path = snapshot_path()?;
    match fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e).with_context(|| format!("removing {}", path.display())),
    }
}

/// Master OFF: record every currently-enabled provider, then disconnect them
/// all. Call this *before* stopping the proxy so each provider's domain is
/// still flippable. Best-effort per provider so one failure can't strand the
/// rest. The snapshot survives until the master is turned back on.
pub fn snapshot_and_disable_all() -> Result<()> {
    let enabled: Vec<String> = list()
        .into_iter()
        .filter(|p| p.enabled)
        .map(|p| p.slug)
        .collect();
    save_snapshot(&enabled)?;
    for slug in &enabled {
        if let Err(e) = disable(slug) {
            eprintln!("[gate] disabling provider {slug:?} during master-off failed: {e}");
        }
    }
    Ok(())
}

/// Master ON: re-enable every provider that was on when routing was last
/// turned off, then clear the snapshot. Idempotent; a missing snapshot is a
/// no-op. Call this *after* the proxy is running so domains re-enable too.
pub fn restore_all() -> Result<()> {
    for slug in load_snapshot()? {
        if let Err(e) = enable(&slug) {
            eprintln!("[gate] restoring provider {slug:?} on master-on failed: {e}");
        }
    }
    clear_snapshot()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn openai_provider_maps_to_codex_and_openai_domain() {
        let p = find("openai").expect("openai provider present");
        assert_eq!(p.display_name, "OpenAI / Codex");
        assert!(p.tool_ids.contains(&ToolId::Codex));
        assert_eq!(p.proxy_domain_slugs, &["openai"]);
    }

    #[test]
    fn anthropic_provider_maps_to_claude_code_and_anthropic_domain() {
        let p = find("anthropic").expect("anthropic provider present");
        assert_eq!(p.display_name, "Claude Code / Cowork");
        assert!(p.tool_ids.contains(&ToolId::ClaudeCode));
        assert_eq!(p.proxy_domain_slugs, &["anthropic"]);
    }

    #[test]
    fn openrouter_provider_is_proxy_only() {
        let p = find("openrouter").expect("openrouter provider present");
        assert_eq!(p.display_name, "OpenRouter");
        assert!(
            p.tool_ids.is_empty(),
            "OpenRouter has no CLI integration — it's proxy-only"
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
}
