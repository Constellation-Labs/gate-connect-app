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
use crate::audit;
use crate::recovery;
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
    /// Every proxy domain that belongs to this family, cascaded or not.
    ///
    /// **One array, not two.** This used to be `proxy_domain_slugs` plus a
    /// `chat_domain_slugs` beside it whose only job was to hold the surfaces
    /// the family switch must never flip - a hand-kept exclusion list, with
    /// paragraphs of comment warning that adding a slug to the wrong one would
    /// route the user's signed-in identity the moment they enabled "Claude".
    ///
    /// The fact that decides it was never a property of the family; it is a
    /// property of the row. It lives on [`ProxyDomain::credential`] now, and
    /// [`cascade_domains`] derives the exclusion from it, so the two cannot
    /// drift and a new entry cannot join the cascade by being typed into the
    /// wrong array.
    ///
    /// [`ProxyDomain::credential`]: crate::proxy::ProxyDomain::credential
    pub domain_slugs: &'static [&'static str],
}

/// Built-in provider catalog. Claude leads, then OpenAI/Codex; both follow the
/// same one-switch model and others can be added the same way.
///
/// Mapping note: each provider lists only the tools that need per-tool config
/// editing for reliable routing (Claude Code gets `HTTPS_PROXY`; Codex gets a
/// model provider). Desktop apps that honor the system proxy (Cowork / Claude
/// Desktop) ride the proxy domain instead, so they're covered by
/// `domain_slugs` without per-tool config. That's why Cowork isn't in
/// `tool_ids`. A provider with no native CLI integration (OpenRouter) is
/// proxy-only: empty `tool_ids`, routed entirely through its proxy domain.
pub fn providers() -> Vec<Provider> {
    vec![
        Provider {
            slug: "anthropic",
            // The vendor, not the product. Its rows are named for the surface
            // they cover now ("App", "Web", "CLI"), so the heading is the only
            // thing left saying whose traffic this is - and "Claude" over a row
            // reading "CLI" leaves a user guessing between Claude Code and
            // claude.ai.
            display_name: "Anthropic",
            subtitle: "Claude Code + Claude Desktop",
            tool_ids: &[ToolId::ClaudeCode],
            // Both domains, in one array, and `claude-web` is still excluded
            // from the cascade - by its own `Credential::Additive` rather than
            // by living in a second field. The invariant is unchanged and the
            // way it is enforced is not: enabling "Claude" must never start
            // intercepting the user's claude.ai session, and now the reason it
            // does not is a property of that row instead of a slug someone
            // remembered to type into the other array. See [`cascade_domains`].
            domain_slugs: &["anthropic", "claude-web"],
        },
        Provider {
            slug: "openai",
            display_name: "OpenAI",
            subtitle: "Codex + OpenAI API",
            tool_ids: &[ToolId::Codex],
            // The `openai` domain's absence is the point, and it survives the
            // collapse into one array: this family lists the two chat surfaces
            // and nothing else.
            //
            // That entry is api.openai.com, and nothing in this family rides its
            // switch. Codex is config-routed: in API-key mode it points at the
            // relay, which resolves routes against the WHOLE catalog
            // (`relay.rs` builds from `default_domains()`, not the enabled set),
            // so Codex routes whether that switch is on or off. The ChatGPT
            // desktop app talks to chatgpt.com, which is the `chatgpt` entry
            // below. What the switch actually governs is MITM interception of
            // api.openai.com for any system-proxy-honouring client - generic
            // traffic, no OpenAI tool Gate configures.
            //
            // Its real dependants are the multi-provider harnesses: OpenClaw and
            // Hermes blind-tunnel anything outside the enabled catalog, so this
            // switch is what lets Gate see their OpenAI calls. The entry now
            // says so itself - it is `Client::AnyApp` in the catalog, and the
            // ledger draws it under the machine-wide heading with the other
            // rows that cover whatever happens to be running.
            //
            // Consequence worth stating: this family's switch governs Codex
            // alone. Both domains listed below are `Credential::Additive`, so
            // the cascade reaches neither.
            //
            // `chatgpt` is the ChatGPT-subscription Responses endpoint. It is
            // wired because OpenClaw's managed proxy mode sends its
            // subscription model calls to that host and this switch is the only
            // thing that lets Gate see them - `integrations/openclaw.rs` used to
            // flip the domain itself, which is what this row replaces.
            //
            // `chatgpt-apps` covers the ChatGPT app's own chat turn (a
            // session-cookie surface) alongside Codex's tool plane.
            domain_slugs: &["chatgpt", "chatgpt-apps"],
        },
        Provider {
            slug: "openrouter",
            display_name: "OpenRouter",
            subtitle: "OpenRouter API",
            // Proxy-only: OpenRouter has no Gate Connect CLI integration, so it
            // routes entirely through the proxy domain (requires the proxy to
            // be running, like Cowork).
            tool_ids: &[],
            // One brokered domain and no session surface: OpenRouter is an API
            // host, and there is no signed-in product in front of it.
            domain_slugs: &["openrouter"],
        },
    ]
}

pub fn find(slug: &str) -> Option<Provider> {
    providers().into_iter().find(|p| p.slug == slug)
}

/// The family's domains that a family switch may actually flip.
///
/// This is the rule that used to be a second array on [`Provider`]. It is
/// derived now, per row, from [`ProxyDomain::credential`]: a family switch
/// flips brokered rows and nothing else, because the others carry a credential
/// the user is already signed in with and routing that is a deliberate per-row
/// act.
///
/// Reads the built-in catalog rather than the persisted one on purpose. The
/// credential is a property of the entry, not of the user's state, so this
/// needs no I/O and cannot be changed by what is on disk.
///
/// A slug the catalog does not know is excluded rather than included. That is
/// the safe direction: the failure mode of including it is routing a surface
/// nobody classified, and the failure mode of excluding it is a switch that
/// leaves one row for the user to flip themselves.
///
/// [`ProxyDomain::credential`]: crate::proxy::ProxyDomain::credential
pub fn cascade_domains(p: &Provider) -> Vec<&'static str> {
    let catalog = crate::proxy::default_domains();
    p.domain_slugs
        .iter()
        .copied()
        .filter(|slug| {
            catalog
                .iter()
                .find(|d| d.slug == *slug)
                .is_some_and(|d| d.credential.cascades())
        })
        .collect()
}

/// UI snapshot of one provider.
#[derive(Debug, Clone, Serialize)]
pub struct ProviderState {
    pub slug: String,
    pub display_name: String,
    pub subtitle: String,
    /// Headline on/off: at least one of the provider's tool integrations is
    /// configured to route through Gate. Reads only what this provider's
    /// switch governs, so a family whose chat domain alone is on still reports
    /// off - that domain answers to its own switch and nothing else.
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
    /// The subset of `domain_slugs` this provider's switch actually flips:
    /// [`cascade_domains`]'s answer, reported rather than re-derived.
    ///
    /// Replaces the old `chat_domain_slugs`, which named the excluded half and
    /// left the included half to be inferred. Naming the included half instead
    /// means a consumer that wants "what does this switch do" reads it directly
    /// and a consumer that wants the excluded rows takes the difference - and
    /// neither has to know the credential rule.
    pub cascade_domain_slugs: Vec<String>,
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
    let cascaded = cascade_domains(p);
    if cascaded.is_empty() {
        return false;
    }
    crate::proxy::manager()
        .status()
        .map(|s| {
            s.domains
                .iter()
                .any(|d| d.enabled && cascaded.contains(&d.slug.as_str()))
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
        domain_slugs: p.domain_slugs.iter().map(|s| s.to_string()).collect(),
        cascade_domain_slugs: cascade_domains(p).iter().map(|s| s.to_string()).collect(),
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
    enable_inner(slug, &[], Request::ByName).map(|(_, state)| state)
}

/// Who asked, which is the only thing that separates the two callers below.
///
/// It replaces an `audit: bool`, and the replacement is the fix rather than
/// tidying. `enable_inner` needed to know whether it was serving a restore in
/// two places - whether to emit the audit event, and whether "nothing to
/// configure yet" is an error - and only the first had a parameter. The second
/// read `!skip.is_empty()` instead, on the reasoning that a restore is the
/// caller that passes a skip list. But a skip list is only non-empty when a
/// member was switched off before routing stopped, and `enable` passes an empty
/// one too - so an ordinary restore was indistinguishable from a by-name
/// request and got the by-name error. `restore_all` recorded that as
/// `WriteFailed`, and the recovery summary told the user Gate could not write a
/// config it had never opened.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Request {
    /// The user named this provider. Nothing happening is a result that needs
    /// explaining, and the action is theirs, so it is audited.
    ByName,
    /// A restore pass. Nothing to do yet is [`Applied::NotYet`], and the audit
    /// event belongs to the master switch that drove it - see
    /// [`enable_skipping`].
    Restore,
}

/// What an enable actually did, as distinct from whether it went wrong.
///
/// The awkward case is the third one: nothing was configured, and nothing is
/// broken. It used to be reported as `Ok` or as an error depending only on
/// whether the skip list happened to be non-empty, which made "did nothing"
/// indistinguishable from "done" for the restore path - and clearing the
/// restore snapshot on that reading is what left domain-only providers off for
/// a whole session. Saying which of the two happened is this enum's only job.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Applied {
    /// A route was configured: a tool's config, a proxy domain, or both.
    Enabled,
    /// No route to configure *yet*. With no detected tool and no running
    /// engine there is nothing this call can do; a later call, once the engine
    /// is up, can. Only the restore path sees this - a user who asked for a
    /// provider by name gets an error, because for them nothing happening is a
    /// result that needs explaining.
    NotYet,
    /// No route to configure, and no later call will change that.
    ///
    /// The distinction from [`NotYet`](Applied::NotYet) is *ever* versus *yet*,
    /// and it exists because a provider can have nothing to enable while the
    /// engine is running perfectly. `openai` is the worked example: its only
    /// tool is Codex, and both of its domains are `Credential::Additive`, so
    /// `cascade_domains` returns an empty list and the domain loop below has
    /// nothing to walk. With Codex uninstalled, `enable_inner` configures no
    /// tool, enables no domain, and returns a state whose `enabled` is false.
    ///
    /// Without this variant that outcome was indistinguishable from "the route
    /// did not take", which the restore leaves `Pending` on purpose. The
    /// entry then went back into the snapshot, rendered as "Not started", and
    /// every Retry re-ran the identical path to the identical result. AG-885
    /// reported it as a banner that would not clear and a Retry button that
    /// did nothing, which is exactly what it was: the same defect the
    /// unresolvable-slug guard in `restore_all` already fixed once, reached by
    /// a different route.
    NothingRoutable,
}

/// [`enable`] with members to leave alone: the restore path's flavour, so a
/// member that was already switched off when routing was turned off does not
/// come back on with the rest of its family. See [`RESTORE_SKIP_MEMBERS`].
/// No audit event: the master switch that drives the restore already emitted
/// one `proxy_enabled`, and `provider_enabled` is reserved for the operator
/// toggling that provider by hand (see the one-event-per-action rule in
/// [`crate::audit`]).
fn enable_skipping(slug: &str, skip: &[String]) -> Result<(Applied, ProviderState)> {
    enable_inner(slug, skip, Request::Restore)
}

fn enable_inner(slug: &str, skip: &[String], request: Request) -> Result<(Applied, ProviderState)> {
    let p = find(slug).with_context(|| format!("unknown provider {slug:?}"))?;
    let account = account::load()?
        .context("no Gate account configured - sign in before enabling a provider")?;
    let skipped = |s: &str| skip.iter().any(|x| x == s);
    let any_detected = p.tool_ids.iter().any(|&id| {
        tool_detected(id) && !registry::find(id).is_some_and(|i| skipped(i.id().slug()))
    });
    let plan = enable_plan(any_detected, proxy_running());

    if plan.nothing {
        // A restore pass has nothing to do here and nothing to complain about:
        // either the family's members are all switched off, or - the case this
        // used to get wrong - the provider's only route is a proxy domain and
        // the engine is not up yet, which is precisely what the second pass
        // exists for. Only a user who asked for this provider by name gets the
        // explanation, and the sentence below is written for them: telling
        // somebody mid-master-on to "turn on Route through Gate" describes the
        // operation they are already running.
        if request == Request::Restore {
            return Ok((Applied::NotYet, state(&p)));
        }
        anyhow::bail!(
            "nothing to configure for {}: install its app, or turn on \
             \u{201c}Route through Gate\u{201d} to route it through the proxy",
            p.display_name
        );
    }

    // Everything this restore is allowed to turn on, after the two exclusions
    // that can empty it: a credential that does not cascade, and a member the
    // user switched off before routing stopped.
    let routable: Vec<&'static str> = cascade_domains(&p)
        .into_iter()
        .filter(|domain| !skipped(domain))
        .collect();

    // Nothing to configure and nothing to enable, with the engine up. Disjoint
    // from `plan.nothing` above, which is the engine-down case and says "not
    // yet"; here a later pass would do exactly as much, so saying "yet" would
    // promise a second attempt that cannot differ. See `Applied::NothingRoutable`.
    //
    // The state is still returned rather than an error, so `enable` by name
    // behaves exactly as before - only the restore paths read the variant.
    if !any_detected && routable.is_empty() {
        return Ok((Applied::NothingRoutable, state(&p)));
    }

    if plan.configure_tool {
        for &id in p.tool_ids {
            let Some(integ) = registry::find(id) else {
                continue;
            };
            if !integ.detect().unwrap_or(false) {
                continue; // tool not installed - nothing to configure
            }
            if skipped(integ.id().slug()) {
                continue; // switched off before routing stopped; leave it off
            }
            let input = ConnectInput {
                gateway_base_url: account.gateway_base_url.clone(),
                upstream_url: integ.default_upstream_url().to_string(),
                billing_mode: account.billing_mode,
                relay_base_url: crate::proxy::relay_base_url(),
                engine_proxy_url: crate::proxy::tool_proxy_url(),
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
    for domain in cascade_domains(&p) {
        if skipped(domain) {
            continue; // switched off before routing stopped; leave it off
        }
        if plan.enable_domain {
            crate::proxy::manager()
                .set_domain(domain, true)
                .with_context(|| format!("enabling proxy domain {domain:?}"))?;
        } else {
            crate::proxy::config::set_enabled(domain, true)
                .with_context(|| format!("persisting proxy domain {domain:?}"))?;
        }
    }

    let state = state(&p);

    // Best-effort audit. The account is already loaded here, so its key is the
    // in-hand credential for ApiKey mode; OAuth mode ignores it and reads the
    // live access token.
    if request == Request::ByName {
        audit::provider_enabled(
            &account.gateway_base_url,
            Some(&account.api_key),
            p.display_name,
        );
    }

    Ok((Applied::Enabled, state))
}

/// Whether a teardown puts each tool back on its own configuration.
///
/// The distinction the proxy layer draws between a park and a release
/// (`proxy::manager_core::Teardown`), one level up. The two are the same
/// question asked of two different things Gate leaves behind: a bound port, and
/// a line in somebody's `settings.json`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ToolConfigs {
    /// Leave them naming Gate.
    ///
    /// For the routing toggle. The engine parks rather than stopping, so the
    /// addresses those configs hold keep answering and forward straight
    /// through - which is what the tool would have done with Gate not
    /// installed. Rewriting them would move no traffic and would cost every
    /// running tool a restart, because a tool reads its configuration once and
    /// the file's mtime is what says it missed a change.
    ///
    /// It also makes the switch *live*: a `codex` that was running before the
    /// toggle passes through while parked and routes again when the engine
    /// unparks, with no restart at either edge. Reverting the config is what
    /// used to break that, by handing the next-started process a different
    /// answer from the one the running process holds.
    Kept,
    /// Put each tool back on its own settings.
    ///
    /// For the explicit "Gate should let go of this machine" actions - the
    /// quit-and-disconnect choice, signing out, Reset. The same line
    /// `proxy::forwarder::stop` is on, and drawn in the same place.
    Reverted,
}

/// Turn a provider off. Reverts the config integration(s) and, if the proxy is
/// running, disables the provider's proxy domains. Promptless and idempotent.
pub fn disable(slug: &str) -> Result<ProviderState> {
    disable_inner(slug, true, ToolConfigs::Reverted)
}

/// [`disable`] with the audit emit optional: the master-off sweep passes
/// `false`, because that sweep is one operator action (the master switch) that
/// already emits a single `proxy_disabled` - see the one-event-per-action rule
/// in [`crate::audit`].
fn disable_inner(slug: &str, audit: bool, configs: ToolConfigs) -> Result<ProviderState> {
    let p = find(slug).with_context(|| format!("unknown provider {slug:?}"))?;

    if configs == ToolConfigs::Reverted {
        for &id in p.tool_ids {
            let Some(integ) = registry::find(id) else {
                continue;
            };
            let connected = matches!(
                integ.status(),
                Ok(Status::Connected | Status::Drifted(_) | Status::Overridden(_))
            );
            if connected || integ.detect().unwrap_or(false) {
                integ
                    .disconnect()
                    .with_context(|| format!("disconnecting {}", integ.display_name()))?;
            }
        }
    }

    // Record the off state durably so a later reconcile ([`reconcile_enabled`])
    // won't treat the provider as still-on and re-apply it. When the engine is
    // live, route the change through the manager so routing also stops
    // immediately; otherwise persist the flag directly (the config-route tools
    // don't need the proxy running to be turned off).
    #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
    for domain in cascade_domains(&p) {
        // Best-effort: an already-off or unknown domain isn't an error.
        let _ = if proxy_running() {
            crate::proxy::manager()
                .set_domain(domain, false)
                .map(|_| ())
        } else {
            crate::proxy::config::set_enabled(domain, false).map(|_| ())
        };
    }

    let state = state(&p);

    // Best-effort audit. `load_base_url` rather than `load`, because the URL is
    // all this path needs; `audit::credential` reaches for the key itself when
    // the mode calls for one, so passing `None` costs no coverage.
    if audit {
        if let Ok(Some(base_url)) = account::load_base_url() {
            audit::provider_disabled(&base_url, None, p.display_name);
        }
    }

    Ok(state)
}

/// Persisted (on-disk) view of whether any of the provider's proxy domains are
/// enabled - the durable "the user wants this provider on" signal, readable even
/// when the proxy engine is stopped. Distinct from [`proxy_domains_enabled`],
/// which reflects the live engine's current domain set.
#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
fn domains_enabled_persisted(p: &Provider) -> bool {
    let cascaded = cascade_domains(p);
    crate::proxy::config::load_domains()
        .map(|ds| {
            ds.iter()
                .any(|d| d.enabled && cascaded.contains(&d.slug.as_str()))
        })
        .unwrap_or(false)
}
#[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
fn domains_enabled_persisted(_p: &Provider) -> bool {
    false
}

/// Can this integration's `connect` succeed right now, as far as the engine is
/// concerned?
///
/// Only a question for the tools that route through the forward proxy, and
/// [`Integration::requires_engine`] is what says which those are. It matters to
/// the reconcile passes because they act on `Drifted`, and drift is the steady
/// state for those three while routing is off: their `status` asks whether the
/// engine is *routing*, it is not, and the config Gate wrote is still on disk
/// now that master-off keeps it there. Without this the pass would call
/// `connect` on every startup and every window focus, get the refusal those
/// integrations raise by design, and log a failure about a machine with nothing
/// wrong with it.
///
/// A relay tool is unaffected and deliberately so: its `connect` needs only a
/// persisted relay port, which it has whether or not anything is up, and
/// re-asserting a base URL that is already correct writes nothing (see
/// `primitives::write_file`).
fn engine_up_if_needed(integ: &dyn registry::Integration) -> bool {
    !integ.requires_engine() || crate::proxy::engine_proxy_url().is_some()
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
/// [`Status::Detected`] (installed, no Gate config) are connected when the
/// provider's switch is on; a [`Status::Drifted`] tool is *re*-connected
/// whenever its config carries our own management marker
/// ([`Integration::config_is_managed`]) - i.e. the stale values are ours (an old
/// scheme, a changed relay port), not a setup the user made by hand - and the
/// relay is up so there's a live base URL to point it at. The switch is not
/// consulted for that half: the marker is the user's own past connect, which
/// says more about intent than a domain flag does, and [`disable`] disconnects
/// each tool before persisting the off state, so a provider the user turned off
/// leaves nothing marked for this to find. Unmarked drift is left alone so this
/// never clobbers an out-of-app setup.
///
/// Tools no provider maps get the drift half of the same treatment via
/// [`reconcile_unmapped_tools`]; they have no provider flag to read as intent,
/// so they are never auto-*connected*.
pub fn reconcile_enabled() -> Result<()> {
    let Some(account) = account::load()? else {
        return Ok(()); // no gateway configured yet - nothing to point tools at
    };
    let relay_base_url = crate::proxy::relay_base_url();
    for p in providers() {
        // The switch gates auto-*connecting*, not repairing. A `Detected` tool
        // has never been routed, so something has to say the user wants it to
        // be, and the enabled domain is that something. Managed drift says it
        // already: the config carries our marker, which is the user's own past
        // connect, and reasserting a base URL of ours that went stale is
        // finishing that job rather than starting a new one. That is the test
        // [`reconcile_unmapped_tools`] applies to the tools no provider maps,
        // and this pass disagreeing with it is why Codex never repaired itself.
        //
        // Codex is the case that proves it rather than an exception to it. Its
        // provider's cascade is deliberately EMPTY - both `chatgpt` entries are
        // `Credential::Additive`, asserted in this module's tests - so no switch
        // on the machine can ever report the OpenAI family as on, and under the
        // old gate `config_is_managed` was unreachable for the one tool it was
        // written for.
        //
        // Turning a provider off does not leave a tool behind for this to pick
        // up: [`disable`] disconnects each one first, which removes the marker,
        // so a repaired tool is always one the user still has connected.
        let enabled = domains_enabled_persisted(&p);
        for &id in p.tool_ids {
            let Some(integ) = registry::find(id) else {
                continue;
            };
            if integ.requires_upstream_credential() {
                continue; // needs a stored key; not safe to auto-apply
            }
            let reapply = match integ.status() {
                Ok(Status::Detected) => enabled,
                // Our own writes gone stale - safe to reassert, but only with
                // a relay to point at (connect() bails without one, and this
                // drift may *be* "relay not enabled yet").
                Ok(Status::Drifted(_)) => {
                    relay_base_url.is_some()
                        && integ.config_is_managed().unwrap_or(false)
                        && engine_up_if_needed(integ.as_ref())
                }
                // NotInstalled / Connected / Overridden / status error - leave
                // as-is. Overridden belongs on this side of the line and not
                // with drift: our values are already exactly what connect would
                // write, so a re-apply is a no-op that would run on every pass.
                _ => false,
            };
            if !reapply {
                continue;
            }
            let input = ConnectInput {
                gateway_base_url: account.gateway_base_url.clone(),
                upstream_url: integ.default_upstream_url().to_string(),
                billing_mode: account.billing_mode,
                relay_base_url: relay_base_url.clone(),
                engine_proxy_url: crate::proxy::tool_proxy_url(),
            };
            if let Err(e) = integ.connect(&input) {
                crate::logging::failure(&format!(
                    "auto-configuring {} failed: {e:#}",
                    integ.display_name()
                ));
            }
        }
    }
    reconcile_unmapped_tools(&account, relay_base_url.as_deref())
}

/// Self-heal the registry tools no provider maps (OpenCode, OpenClaw, Hermes).
///
/// Unlike a provider tool, a standalone tool has no enabled-provider flag to
/// read as intent, so `Detected` (installed, no Gate config) is left alone -
/// nothing says the user wants it routed. Only *our own* stale write is
/// reasserted: `Drifted` plus [`Integration::config_is_managed`], the same test
/// the provider pass uses. That covers the case this exists for - the relay
/// came back on a different port, so the base URL we wrote is now dead - while
/// never clobbering a config the user set up out-of-app.
fn reconcile_unmapped_tools(
    account: &account::Account,
    relay_base_url: Option<&str>,
) -> Result<()> {
    let Some(relay_base_url) = relay_base_url else {
        return Ok(()); // no relay to point anything at; connect() would bail
    };
    let mapped: Vec<ToolId> = providers()
        .iter()
        .flat_map(|p| p.tool_ids.iter().copied())
        .collect();
    for integ in registry::registry() {
        if mapped.contains(&integ.id()) {
            continue; // covered by the provider pass above
        }
        if integ.requires_upstream_credential() {
            continue; // needs a stored key; not safe to auto-apply
        }
        if !matches!(integ.status(), Ok(Status::Drifted(_))) {
            continue;
        }
        if !integ.config_is_managed().unwrap_or(false) {
            continue; // drift in a config we didn't write - leave it alone
        }
        if !engine_up_if_needed(integ.as_ref()) {
            continue;
        }
        let input = ConnectInput {
            gateway_base_url: account.gateway_base_url.clone(),
            upstream_url: integ.default_upstream_url().to_string(),
            billing_mode: account.billing_mode,
            relay_base_url: Some(relay_base_url.to_string()),
            engine_proxy_url: crate::proxy::tool_proxy_url(),
        };
        if let Err(e) = integ.connect(&input) {
            crate::logging::failure(&format!(
                "re-applying {} failed: {e:#}",
                integ.display_name()
            ));
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
/// Member slugs (tool or proxy domain) that were already switched off inside a
/// provider the master-off snapshot recorded, so master-on brings the family
/// back without turning them on too.
///
/// [`PROVIDER_SNAPSHOT`] is provider-granularity while [`enable`] turns on
/// *every* member of a provider, and a provider counts as enabled when any one
/// member is on. So a family that was on because one member was on came back
/// with all of them on: switch Claude Desktop off while Claude Code stays on,
/// toggle routing, and Claude Desktop is routing again, with nothing anywhere
/// recording that it had been switched off. Master-off destroys the per-member
/// state on its way out (`disable` clears every domain flag and disconnects
/// every tool), so the distinction has to be written down before it goes.
///
/// Scoped to one master cycle, not a durable preference: it is written at
/// master-off and cleared once the restore completes.
const RESTORE_SKIP_MEMBERS: &str = "restore-skip-members.json";

fn snapshot_path(file: &str) -> Result<PathBuf> {
    Ok(crate::env::app_support_dir()?.join("provider").join(file))
}

fn save_snapshot(file: &str, slugs: &[String]) -> Result<()> {
    let path = snapshot_path(file)?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
    }
    let raw = serde_json::to_string(slugs).context("serializing provider snapshot")?;
    // Atomic, like every other file under app support: a torn snapshot reads
    // back as empty and leaves swept tools reverted with nothing to restore
    // them. 0o600 to match its neighbours; the contents are slugs, not secrets.
    crate::primitives::write_file(&path, raw.as_bytes(), 0o600)
        .with_context(|| format!("writing {}", path.display()))
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

/// [`master_flow_guard`] that gives up after `wait`, for the one caller that
/// must not block indefinitely: the quit path. A restore or a toggle mid-flight
/// when the user quits is unlikely and short, but "the app will not close" is
/// the worst outcome on that path, and quitting *without* the revert is only
/// the behaviour every release before this one had.
fn try_master_flow_guard(wait: std::time::Duration) -> Option<MutexGuard<'static, ()>> {
    let deadline = std::time::Instant::now() + wait;
    loop {
        match MASTER_FLOW_LOCK.try_lock() {
            Ok(g) => return Some(g),
            Err(std::sync::TryLockError::Poisoned(p)) => return Some(p.into_inner()),
            Err(std::sync::TryLockError::WouldBlock) if std::time::Instant::now() < deadline => {
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            Err(std::sync::TryLockError::WouldBlock) => return None,
        }
    }
}

/// Members of `p` that are not carrying traffic right now.
///
/// Not-installed tools are left out: absent is not "switched off", and
/// [`enable`] skips them anyway. A drifted tool counts as off, which is the
/// conservative reading - its config points somewhere that is not ours, and a
/// restore has no business overwriting that.
fn off_members(p: &Provider) -> Vec<String> {
    let mut out = Vec::new();
    for &id in p.tool_ids {
        if tool_detected(id) && !tool_connected(id) {
            out.push(id.slug().to_string());
        }
    }
    #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
    if let Ok(domains) = crate::proxy::config::load_domains() {
        // Hoisted, like the two other callers: `cascade_domains` rebuilds the
        // whole catalog to answer, and inside the loop that was once per
        // persisted domain.
        let cascaded = cascade_domains(p);
        for d in domains {
            if cascaded.contains(&d.slug.as_str()) && !d.enabled {
                out.push(d.slug);
            }
        }
    }
    out
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
fn snapshot_and_disable_all_locked(configs: ToolConfigs) -> Result<()> {
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

    // Which members were *already* off inside the families we just snapshotted.
    // Recorded before the disable loop below, because that loop is what destroys
    // the distinction.
    //
    // Only the providers in this pass, deliberately. A second off-flow (a quit
    // teardown right after a master-off) sees every provider off, so `enabled`
    // is empty and this loop does nothing. Iterating the whole catalog instead
    // would put every member of every family on the skip list and restore
    // nothing - the exact inverse of the bug being fixed, and a worse one.
    let mut skip = load_snapshot(RESTORE_SKIP_MEMBERS)?;
    for slug in &enabled {
        if let Some(p) = find(slug) {
            for member in off_members(&p) {
                if !skip.contains(&member) {
                    skip.push(member);
                }
            }
        }
    }
    if !skip.is_empty() {
        save_snapshot(RESTORE_SKIP_MEMBERS, &skip)?;
    }

    for slug in &enabled {
        // `audit: false`: the sweep is the master switch's doing, and that one
        // operator action already emits `proxy_disabled`.
        if let Err(e) = disable_inner(slug, false, configs) {
            // `{e:#}` rather than `{e}`: the chain is the reason, and the outer
            // context on its own routinely says only which step it was.
            crate::logging::failure(&format!(
                "disabling provider {slug:?} during master-off failed: {e:#}"
            ));
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
///
/// Returns the **display names of the tools it could not return to their own
/// settings**, empty when everything came back. Best-effort still means the call
/// succeeds when one tool fails, because the sweep must not abandon the remaining
/// tools; the difference is that the failure is now the caller's to report rather
/// than a line on stderr. A quit that leaves a config pointing at a relay about
/// to die is exactly what the user needs told, and the old signature could not
/// say it.
pub fn snapshot_and_disable_everything() -> Result<Vec<String>> {
    let _guard = master_flow_guard();
    snapshot_and_disable_all_locked(ToolConfigs::Reverted)?;
    let mut disconnected = Vec::new();
    let mut failed = Vec::new();
    for integ in registry::registry() {
        if !matches!(
            integ.status(),
            Ok(Status::Connected | Status::Drifted(_) | Status::Overridden(_))
        ) {
            continue;
        }
        match integ.disconnect() {
            Ok(()) => disconnected.push(integ.id().slug().to_string()),
            Err(e) => {
                // Kept on stderr for the log, *and* returned. It used to be only
                // the former, which meant a tool left pointing at a dead relay
                // was invisible to the caller and the quit reported success.
                crate::logging::failure(&format!(
                    "disconnecting {} during quit failed: {e:#}",
                    integ.display_name()
                ));
                failed.push(integ.display_name().to_string());
            }
        }
    }
    record_swept(disconnected)?;
    Ok(failed)
}

/// Add `slugs` to the swept-tools snapshot so the startup restore reconnects
/// them. A union, for the same reason the provider snapshot unions: an existing
/// file is a pending restore, and overwriting it would drop tools from it. No
/// write at all when there is nothing to add, so a no-op sweep leaves no empty
/// snapshot behind.
fn record_swept(slugs: Vec<String>) -> Result<()> {
    if slugs.is_empty() {
        return Ok(());
    }
    let mut snapshot = load_snapshot(SWEPT_TOOLS_SNAPSHOT)?;
    for slug in slugs {
        if !snapshot.contains(&slug) {
            snapshot.push(slug);
        }
    }
    save_snapshot(SWEPT_TOOLS_SNAPSHOT, &snapshot)
}

/// One thing a restore has recorded and not finished.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct PendingEntry {
    pub slug: String,
    /// What to call it on screen. Falls back to the slug for an entry whose
    /// provider or tool is no longer in the registry - an uninstall between the
    /// snapshot and now - because naming it is still better than dropping it from
    /// a list the user is being asked to act on.
    pub name: String,
}

/// Routing work that was written down and has not completed.
///
/// The snapshots have always been a record of unfinished work - [`restore_all`]
/// keeps failures in the file and only clears it once everything is back - but
/// nothing ever read them for display. So a restore that half-succeeded left the
/// user with some tools routing, some not, and no statement anywhere that Gate
/// knew about it.
///
/// Empty means there is nothing outstanding, which is the normal case.
#[derive(Debug, Clone, Serialize, PartialEq, Eq, Default)]
pub struct PendingRestore {
    /// Providers still waiting to be re-enabled.
    pub providers: Vec<PendingEntry>,
    /// Standalone tools (OpenCode and friends) still waiting to be reconnected.
    pub tools: Vec<PendingEntry>,
}

impl PendingRestore {
    pub fn is_empty(&self) -> bool {
        self.providers.is_empty() && self.tools.is_empty()
    }
}

/// What a restore still owes, read from the snapshots.
///
/// Read-only: it opens no config, starts nothing, and writes nothing. Safe to call
/// on a status refresh.
pub fn pending_restore() -> Result<PendingRestore> {
    let providers = load_snapshot(PROVIDER_SNAPSHOT)?
        .into_iter()
        .map(|slug| {
            let name = find(&slug)
                .map(|p| p.display_name.to_string())
                .unwrap_or_else(|| slug.clone());
            PendingEntry { slug, name }
        })
        .collect();
    let tools = load_snapshot(SWEPT_TOOLS_SNAPSHOT)?
        .into_iter()
        .map(|slug| {
            let name = ToolId::from_slug(&slug)
                .and_then(registry::find)
                .map(|integ| integ.display_name().to_string())
                .unwrap_or_else(|| slug.clone());
            PendingEntry { slug, name }
        })
        .collect();
    Ok(PendingRestore { providers, tools })
}

/// Master OFF via the routing switch: record what was on and turn the domains
/// off, and **leave every tool's configuration alone**.
///
/// The counterpart of [`snapshot_and_disable_everything`], which is what the
/// quit-and-disconnect choice still runs. The two used to be one function,
/// because they used to be the same event: the engine stopped either way, so a
/// config naming the loopback relay was about to point at nothing, and putting
/// it back was the only way to leave the tool working.
///
/// The engine parks now (`proxy::manager_core`, `Teardown::Dormant`). The ports
/// stay bound and forward straight through, so a config naming them still
/// works and still reaches the tool's own provider. Rewriting it moves no
/// traffic, and it costs something real: a tool reads its configuration once,
/// so the write tells every running process that it missed a change and has to
/// be reopened. Routing off is not a reason to restart somebody's editor.
///
/// Nothing is recorded in [`SWEPT_TOOLS_SNAPSHOT`], because nothing was swept.
/// [`restore_all`] still runs on master-on and still re-enables the providers;
/// the tool half of it finds an empty snapshot and does nothing, and the
/// provider half re-writes configs that already hold the right bytes, which
/// `primitives::write_file` declines to turn into a write.
pub fn snapshot_and_park_everything() -> Result<()> {
    let _guard = master_flow_guard();
    snapshot_and_disable_all_locked(ToolConfigs::Kept)
}

/// Is this tool pointed at an address that stops answering when the GUI quits?
///
/// Managed at all (its status is one the sweeps act on), and at least one
/// address its configuration names is hosted in the engine's process -
/// [`crate::proxy::address_dies_with_gui`], per address. Not the declared
/// [`registry::Mechanism`]: a forward-proxy tool whose install still names the
/// engine's own port dies exactly like a relay tool, and a relay tool the user
/// has repointed by hand dies not at all.
fn stranded_by_quit(integ: &dyn registry::Integration) -> bool {
    if !matches!(integ.status(), Ok(Status::Connected | Status::Drifted(_))) {
        return false;
    }
    integ
        .configured_addresses()
        .unwrap_or_default()
        .iter()
        .any(|a| crate::proxy::address_dies_with_gui(a))
}

/// Display names of the tools a plain quit would put back on their own
/// settings. Read-only, for the quit dialog: the same predicate the revert
/// applies, so what the dialog names is exactly what gets rewritten.
pub fn tools_stranded_by_quit() -> Vec<String> {
    registry::registry()
        .into_iter()
        .filter(|i| stranded_by_quit(i.as_ref()))
        .map(|i| i.display_name().to_string())
        .collect()
}

/// Plain quit's teardown, on the platforms where the engine lives in the GUI.
///
/// [`ToolConfigs::Kept`] is the routing toggle's rule: leave a config alone,
/// because the address it names keeps answering. A plain quit breaks that
/// premise for some addresses and not others, and the line between them is
/// not a tool boundary. Everything naming the forwarder keeps working, because
/// the forwarder is a separate process and is deliberately left running
/// (`proxy::forwarder::stop` is not called here). A config naming the relay, or
/// the engine's own port, names a listener inside this process, and nothing
/// fronts it: the tool cannot connect until Gate runs again, with an error
/// about a loopback port the user has never heard of.
///
/// So this reverts a config **if and only if an address it names dies with
/// this process** - [`stranded_by_quit`], which is also what the quit dialog
/// used to name these tools a moment ago. Reverted tools are recorded in
/// [`SWEPT_TOOLS_SNAPSHOT`] so the startup restore brings them back exactly as
/// it brings back the quit-and-disconnect sweep. No provider is snapshotted,
/// because no provider was turned off.
///
/// Not called on Linux, where the engine is a daemon and the GUI hosts none of
/// these addresses; the caller gates on platform. Not called from
/// `RunEvent::Exit` either, which also runs on an updater relaunch and a crash
/// restart, neither of which is the user choosing to leave Gate off.
///
/// Returns the display names of what it reverted, for the notification the
/// caller fires: the popover is gone by then, and a rewrite of somebody's
/// config file is worth a sentence. A failure to *record* what was reverted is
/// logged and does not hide the names - that is the one case the sentence
/// matters most, since nothing will restore those tools on the next start.
pub fn revert_stranded_configs_for_quit() -> Result<Vec<String>> {
    let Some(_guard) = try_master_flow_guard(std::time::Duration::from_secs(5)) else {
        anyhow::bail!(
            "another routing operation is still running; quitting without putting relay \
             tools back on their own settings"
        );
    };
    let mut reverted: Vec<(String, String)> = Vec::new();
    for integ in registry::registry() {
        if !stranded_by_quit(integ.as_ref()) {
            continue;
        }
        match integ.disconnect() {
            Ok(()) => reverted.push((
                integ.display_name().to_string(),
                integ.id().slug().to_string(),
            )),
            Err(e) => eprintln!(
                "[gate] reverting {} for quit failed: {e}",
                integ.display_name()
            ),
        }
    }
    let (names, slugs): (Vec<String>, Vec<String>) = reverted.into_iter().unzip();
    if let Err(e) = record_swept(slugs) {
        eprintln!(
            "[gate] recording reverted tools for the next start failed: {e:#}; they will need \
             reconnecting by hand"
        );
    }
    Ok(names)
}

/// Master ON: re-enable every provider that was on when routing was last
/// turned off, then reconnect any standalone tools the master-off sweep
/// disconnected. Entries that are not back yet stay in their snapshot so a
/// later call can retry them; each snapshot is cleared once everything in it
/// is back. Idempotent; a missing snapshot is a no-op. Callers run this twice
/// per master-on: once before the proxy comes up (config-based tools, and the
/// engine's "at least one provider" precondition) and once after (domain-only
/// providers, which have nothing to configure until the proxy is running).
///
/// **An entry leaves the snapshot only once it is actually routing again.**
/// The two passes exist because the first one *cannot* finish the job: a
/// domain-only provider (OpenRouter, or Anthropic on a machine with no Claude
/// app) has no tool to configure and no running engine to flip its domain in,
/// so the pre-enable pass gets [`Applied::NotYet`]. That used to arrive as a
/// bare `Ok`, indistinguishable from a completed restore, and clearing the
/// snapshot on it left the post-enable pass - the one that could actually do
/// the work - with nothing to restore, so every domain-only provider silently
/// stayed off for the rest of the session.
///
/// The [`Applied`] verdict is the primary test; the provider's own `enabled`
/// is checked too, so a call that believes it configured a route but did not
/// produce one is also held for retry. Belt and braces on purpose: this dance
/// is hand-copied across `routing::enable` and the Linux manager's startup
/// re-honor, and an invariant that reads the world protects those call sites
/// from each other in a way a "this is the final pass" flag could not.
pub fn restore_all() -> Result<()> {
    let _guard = master_flow_guard();
    // Members that were off before routing stopped stay off; the family around
    // them comes back. See [`RESTORE_SKIP_MEMBERS`].
    let skip = load_snapshot(RESTORE_SKIP_MEMBERS)?;
    let mut pending = Vec::new();
    let queued = load_snapshot(PROVIDER_SNAPSHOT)?;
    // One journal for the whole restore, seeded with BOTH passes before the first
    // attempt. Both, because a restore is one operation from the user's side and
    // two writers would each clobber the other's file. Seeded up front, because an
    // interruption has to leave the entries it never reached visibly Pending rather
    // than absent.
    //
    // Explanation only: the snapshots remain the state a resume actually works from.
    let mut journal = recovery::JournalWriter::begin(
        queued
            .iter()
            .map(|slug| {
                let name = find(slug)
                    .map(|p| p.display_name.to_string())
                    .unwrap_or_else(|| slug.clone());
                (slug.clone(), name, recovery::EntryKind::Provider)
            })
            .chain(
                load_snapshot(SWEPT_TOOLS_SNAPSHOT)?
                    .into_iter()
                    .map(|slug| {
                        let name = ToolId::from_slug(&slug)
                            .and_then(registry::find)
                            .map(|integ| integ.display_name().to_string())
                            .unwrap_or_else(|| slug.clone());
                        (slug, name, recovery::EntryKind::Tool)
                    }),
            )
            .collect(),
    );
    for slug in queued {
        if find(&slug).is_none() {
            // Written by an older build, or a provider since removed. The tool
            // pass below has had this guard since it was written; this loop
            // never got it, so an unresolvable slug took the `Err` arm, was
            // recorded as a failed write and pushed straight back into the
            // snapshot. That is a retry that cannot ever succeed: `enable_inner`
            // fails on `find` before touching a file, so every resume produced
            // the identical "unknown provider" and the entry outlived every
            // attempt to clear it. Observed in the wild as a permanent "Routing
            // didn't finish - google is still waiting" card whose Resume now
            // could not, even in principle, do anything.
            //
            // Dropped rather than retried, and recorded as settled, which is
            // exactly what `Outcome::Unknown` is for - `is_outstanding` already
            // excludes it, so the recovery card stops counting it.
            journal.record(&slug, recovery::Outcome::Unknown);
            continue;
        }
        match enable_skipping(&slug, &skip) {
            Ok((Applied::Enabled, state)) if state.enabled => {
                journal.record(&slug, recovery::Outcome::Restored);
            }
            // Nothing to do yet, which `enable_inner` reaches only with the
            // engine down: recorded as deferred rather than left `Pending`,
            // because "Not started" reads as an entry the operation never got
            // to and this one was reached and declined. Stays in the snapshot
            // either way, for the post-enable pass.
            Ok((Applied::NotYet, _)) => {
                journal.record(&slug, recovery::Outcome::DeferredEngineDown);
                pending.push(slug);
            }
            // Reached, and there was nothing here to restore: no installed
            // tool and no domain this pass may cascade to. Settled, so it is
            // journalled and dropped rather than re-queued - the same remedy
            // the unknown-slug guard above applies, for the same reason. It
            // used to fall into the arm below and sit at "Not started" for
            // ever. AG-885.
            //
            // `NotInstalled` rather than a new outcome: it is already
            // `is_complete` and already excluded from `is_outstanding`, and
            // the sentence the UI draws for a provider - "Nothing this
            // provider routes is on this machine any more" - is the true one.
            Ok((Applied::NothingRoutable, _)) => {
                journal.record(&slug, recovery::Outcome::NotInstalled);
                continue;
            }
            // A route that did not take: an attempt happened and produced
            // nothing. Not a failure worth reporting and not a completion, so
            // the seeded `Pending` stands rather than the journal being told a
            // story about it.
            //
            // Still reachable, and still deliberately silent: a provider whose
            // members exist but were all skipped lands here. That case is the
            // user's own earlier choice rather than a dead end, and labelling
            // it would need an outcome none of the current ones fit.
            Ok(_) => pending.push(slug),
            Err(e) => {
                // `{e:#}`, matching the string journalled two lines down: the
                // two accounts of one failure disagreeing on detail is how a
                // reader comes to think they are about different things.
                crate::logging::failure(&format!(
                    "restoring provider {slug:?} on master-on failed: {e:#}"
                ));
                // The message, not just the category: `Outcome::category` can
                // say which step failed and never why, and the summary's whole
                // job is the why.
                journal.record_failed(&slug, recovery::Outcome::WriteFailed, &format!("{e:#}"));
                pending.push(slug);
            }
        }
    }
    if pending.is_empty() {
        clear_snapshot(PROVIDER_SNAPSHOT)?;
        // The cycle is complete, so the skip list has done its job. Held until
        // now for the same reason the provider snapshot is: a partial restore
        // gets retried, and the retry needs to know what to leave alone.
        clear_snapshot(RESTORE_SKIP_MEMBERS)?;
    } else {
        save_snapshot(PROVIDER_SNAPSHOT, &pending)?;
    }
    // The same journal continues into the tool pass. Finished here rather than
    // there, and finished even when that pass errors: the journal is the record of
    // what happened, so a failure is exactly when it must survive.
    let swept = restore_swept_tools(&mut journal);
    journal.finish();
    swept
}

/// Reconnect the standalone tools the master-off sweep disconnected (see
/// [`snapshot_and_disable_everything`]). Same retry semantics as the
/// provider snapshot: failures stay recorded, the file clears once every tool
/// is back. Tools uninstalled (or slugs unknown) since the quit are dropped.
/// Signed out since the quit: leave the snapshot for a later signed-in
/// restore - there's no gateway to point the tools at.
fn restore_swept_tools(journal: &mut recovery::JournalWriter) -> Result<()> {
    let slugs = load_snapshot(SWEPT_TOOLS_SNAPSHOT)?;
    if slugs.is_empty() {
        return Ok(());
    }
    let Some(account) = account::load()? else {
        // Signed out: nothing is attempted and the snapshot is left for a later
        // signed-in restore. Recorded as deferred rather than failed - there is
        // nothing wrong with these tools, and calling it a failure would send the
        // user looking for a problem that is really a missing account.
        for slug in &slugs {
            journal.record(slug, recovery::Outcome::DeferredSignedOut);
        }
        return Ok(());
    };
    let relay_base_url = crate::proxy::relay_base_url();
    let engine_proxy_url = crate::proxy::engine_proxy_url();
    // Not `failed`: an entry stays recorded because it is unfinished, and two of
    // the branches below leave it here having found nothing wrong with it.
    let mut outstanding = Vec::new();
    for slug in slugs {
        let Some(integ) = ToolId::from_slug(&slug).and_then(registry::find) else {
            // Written by an older build, or a tool since removed from the registry.
            // Dropped from the snapshot deliberately, so it is recorded as settled
            // rather than left looking like unfinished work.
            journal.record(&slug, recovery::Outcome::Unknown);
            continue;
        };
        if !integ.detect().unwrap_or(false) {
            // Uninstalled since the snapshot. Also dropped: there is nothing to
            // restore, and retrying forever would be wrong.
            journal.record(&slug, recovery::Outcome::NotInstalled);
            continue;
        }
        // The engine is not up, and this tool's config is the engine's address.
        // The provider loop has had this early-out since `Applied::NotYet`
        // existed; this pass had none, so it called `connect`, got the hard
        // error both such integrations raise, and recorded a failed write for a
        // file it never opened. Declared by the integration rather than read off
        // the error - see `Integration::requires_engine`.
        if integ.requires_engine() && engine_proxy_url.is_none() {
            journal.record(&slug, recovery::Outcome::DeferredEngineDown);
            outstanding.push(slug);
            continue;
        }
        let input = ConnectInput {
            gateway_base_url: account.gateway_base_url.clone(),
            upstream_url: integ.default_upstream_url().to_string(),
            billing_mode: account.billing_mode,
            relay_base_url: relay_base_url.clone(),
            engine_proxy_url: crate::proxy::tool_proxy_url(),
        };
        if let Err(e) = integ.connect(&input) {
            crate::logging::failure(&format!(
                "restoring tool {slug:?} on master-on failed: {e:#}"
            ));
            journal.record_failed(&slug, recovery::Outcome::WriteFailed, &format!("{e:#}"));
            outstanding.push(slug);
        } else {
            journal.record(&slug, recovery::Outcome::Restored);
        }
    }
    if outstanding.is_empty() {
        clear_snapshot(SWEPT_TOOLS_SNAPSHOT)
    } else {
        save_snapshot(SWEPT_TOOLS_SNAPSHOT, &outstanding)
    }
}

/// Retry exactly one recorded entry, leaving every other entry's recorded work
/// alone.
///
/// [`restore_all`] is the batch: it walks both snapshots and re-attempts
/// everything in them. That is the right shape for "resume this operation", and
/// the wrong shape for two things the recovery summary needs. One is a retry of a
/// single failing tool, which must not re-enter the providers that already came
/// back. The other is progress: a caller that wants to say which tool it is
/// working on can only do that if it drives the entries itself.
///
/// The semantics are [`restore_all`]'s, narrowed to one slug and not otherwise
/// reinterpreted:
///
/// - **The snapshot is still the state.** The slug leaves its snapshot only once
///   it is actually back, so an unsuccessful retry is a no-op on disk and the
///   next one tries again.
/// - **[`Applied::NotYet`] is not a failure.** A domain-only provider with no
///   engine up yet stays recorded and stays `Pending`, exactly as the batch
///   leaves it. Nothing is journalled about an attempt that did not happen.
/// - **The skip list outlives a partial restore** and clears with the last
///   provider, because a later retry needs to know which members to leave off.
/// - **A slug in neither snapshot is `Ok`**, not an error: two windows can offer
///   the same retry, and the second one arrives to find the work already done.
///
/// Errors are the retry's own: a failed write returns `Err` *and* records
/// `WriteFailed`, so the caller can report the failure rather than infer it from
/// an unchanged pending list.
pub fn restore_one(slug: &str) -> Result<()> {
    let _guard = master_flow_guard();
    let providers = load_snapshot(PROVIDER_SNAPSHOT)?;
    if providers.iter().any(|s| s == slug) {
        return restore_one_provider(slug, providers);
    }
    let tools = load_snapshot(SWEPT_TOOLS_SNAPSHOT)?;
    if tools.iter().any(|s| s == slug) {
        return restore_one_tool(slug, tools);
    }
    Ok(())
}

/// [`restore_one`] for a provider slug, with the queue it was found in.
fn restore_one_provider(slug: &str, queued: Vec<String>) -> Result<()> {
    // Resolved once. It was looked up twice - for the display name and again for
    // the guard - and the first call already handles `None`.
    let provider = find(slug);
    let name = provider
        .as_ref()
        .map(|p| p.display_name.to_string())
        .unwrap_or_else(|| slug.to_string());
    let mut journal = recovery::JournalWriter::reopen(slug, &name, recovery::EntryKind::Provider);
    // One copy of the snapshot rewrite, for the two exits that need it: the
    // unknown-slug settle below and a successful restore at the end drop the
    // entry the same way, and the `RESTORE_SKIP_MEMBERS` clear has to ride along
    // in both. Two copies of a snapshot rewrite is how the two come to disagree,
    // which is why `restore_one_tool` factors its own out the same way.
    let drop_from_snapshot = || -> Result<()> {
        let remaining: Vec<String> = queued.iter().filter(|s| *s != slug).cloned().collect();
        if remaining.is_empty() {
            clear_snapshot(PROVIDER_SNAPSHOT)?;
            // Held until the provider queue empties, for the reason `restore_all`
            // gives: a partial restore gets retried, and the retry needs to know
            // what to leave alone.
            clear_snapshot(RESTORE_SKIP_MEMBERS)
        } else {
            save_snapshot(PROVIDER_SNAPSHOT, &remaining)
        }
    };
    if provider.is_none() {
        // The same guard the batch pass above now carries, and the same one
        // `restore_one_tool` has always had: a slug this build cannot resolve is
        // settled, not outstanding, because no retry can change the answer.
        // Without it the per-row Retry failed identically every time and left
        // the entry in the snapshot for the next one.
        journal.record(slug, recovery::Outcome::Unknown);
        journal.finish();
        return drop_from_snapshot();
    }
    let skip = load_snapshot(RESTORE_SKIP_MEMBERS)?;
    let outcome = match enable_skipping(slug, &skip) {
        Ok((Applied::Enabled, state)) if state.enabled => Ok(Some(true)),
        // Nothing to do yet, which means the engine is not up. Left in the
        // snapshot per the batch's own reasoning, and recorded as deferred for
        // the batch's own reason too: a retry that reports "Not started" claims
        // it never ran.
        Ok((Applied::NotYet, _)) => Ok(None),
        // Nothing here to restore, ever. The batch drops this and so does the
        // retry - and this arm is the one the Retry BUTTON needed: without it
        // a row with no routable member reported `Some(false)`, journalled
        // nothing, stayed in the snapshot and came back reading "Not started".
        // Pressing Retry again did the same. AG-885.
        Ok((Applied::NothingRoutable, _)) => {
            journal.record(slug, recovery::Outcome::NotInstalled);
            journal.finish();
            return drop_from_snapshot();
        }
        // Enabled, but no route came out of it. The batch leaves this `Pending`
        // and so does the retry.
        Ok(_) => Ok(Some(false)),
        Err(e) => Err(e),
    };
    let restored = match outcome {
        Ok(restored) => {
            match restored {
                Some(true) => journal.record(slug, recovery::Outcome::Restored),
                None => journal.record(slug, recovery::Outcome::DeferredEngineDown),
                Some(false) => {}
            }
            journal.finish();
            restored.unwrap_or(false)
        }
        Err(e) => {
            journal.record_failed(slug, recovery::Outcome::WriteFailed, &format!("{e:#}"));
            journal.finish();
            return Err(e).with_context(|| format!("retrying provider {slug:?}"));
        }
    };
    if !restored {
        return Ok(());
    }
    drop_from_snapshot()
}

/// [`restore_one`] for a swept tool slug, with the queue it was found in.
///
/// Mirrors [`restore_swept_tools`]'s per-entry branches - unknown slug, gone from
/// the machine, signed out, write failed - because they are the same four
/// conditions and a second reading of them would be a second set of outcomes.
fn restore_one_tool(slug: &str, queued: Vec<String>) -> Result<()> {
    let integ = ToolId::from_slug(slug).and_then(registry::find);
    let name = integ
        .as_ref()
        .map(|i| i.display_name().to_string())
        .unwrap_or_else(|| slug.to_string());
    let mut journal = recovery::JournalWriter::reopen(slug, &name, recovery::EntryKind::Tool);
    let drop_from_snapshot = |journal: recovery::JournalWriter| -> Result<()> {
        journal.finish();
        let remaining: Vec<String> = queued.iter().filter(|s| *s != slug).cloned().collect();
        if remaining.is_empty() {
            clear_snapshot(SWEPT_TOOLS_SNAPSHOT)
        } else {
            save_snapshot(SWEPT_TOOLS_SNAPSHOT, &remaining)
        }
    };
    let Some(integ) = integ else {
        journal.record(slug, recovery::Outcome::Unknown);
        return drop_from_snapshot(journal);
    };
    if !integ.detect().unwrap_or(false) {
        journal.record(slug, recovery::Outcome::NotInstalled);
        return drop_from_snapshot(journal);
    }
    let Some(account) = account::load()? else {
        // Left in the snapshot for a later signed-in retry, and recorded as
        // deferred rather than failed: there is nothing wrong with this tool.
        journal.record(slug, recovery::Outcome::DeferredSignedOut);
        journal.finish();
        return Ok(());
    };
    let engine_proxy_url = crate::proxy::engine_proxy_url();
    if integ.requires_engine() && engine_proxy_url.is_none() {
        // The batch's early-out, narrowed to one slug: left recorded, and left
        // saying it is waiting for the engine rather than that its write failed.
        // Not an `Err`, because nothing went wrong - a retry that returned one
        // would put an error banner over a tool that is simply next.
        journal.record(slug, recovery::Outcome::DeferredEngineDown);
        journal.finish();
        return Ok(());
    }
    let input = ConnectInput {
        gateway_base_url: account.gateway_base_url.clone(),
        upstream_url: integ.default_upstream_url().to_string(),
        billing_mode: account.billing_mode,
        relay_base_url: crate::proxy::relay_base_url(),
        engine_proxy_url: crate::proxy::tool_proxy_url(),
    };
    if let Err(e) = integ.connect(&input) {
        journal.record_failed(slug, recovery::Outcome::WriteFailed, &format!("{e:#}"));
        journal.finish();
        return Err(e).with_context(|| format!("retrying tool {slug:?}"));
    }
    journal.record(slug, recovery::Outcome::Restored);
    drop_from_snapshot(journal)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A slug the registry no longer knows - a provider or tool uninstalled between
    /// the snapshot and now - still gets named, because dropping it silently would
    /// shorten a list the user is being asked to act on.
    #[test]
    fn an_unknown_slug_still_names_itself() {
        let entry = PendingEntry {
            slug: "retired-provider".into(),
            name: "retired-provider".into(),
        };
        assert_eq!(entry.name, entry.slug);
    }

    #[test]
    fn nothing_outstanding_reads_as_empty() {
        assert!(PendingRestore::default().is_empty());
        assert!(!PendingRestore {
            providers: vec![PendingEntry {
                slug: "openai".into(),
                name: "OpenAI".into(),
            }],
            tools: Vec::new(),
        }
        .is_empty());
    }

    /// Tools alone count. The two snapshots are separate files and a restore can
    /// finish the providers and still owe the standalone tools.
    #[test]
    fn tools_alone_are_still_outstanding() {
        assert!(!PendingRestore {
            providers: Vec::new(),
            tools: vec![PendingEntry {
                slug: "opencode".into(),
                name: "OpenCode".into(),
            }],
        }
        .is_empty());
    }

    /// The shape of AG-885, pinned without needing a running engine.
    ///
    /// The reported symptom - `openai` stuck at "Not started", Retry and
    /// Resume both inert - is composed of two facts that are pure and can be
    /// asserted directly. Together they say the row cannot succeed: with Codex
    /// absent there is no tool to configure, and with an empty cascade there is
    /// no domain to enable, yet `enable_plan` still reports work to do because
    /// the engine is up.
    ///
    /// Asserted here rather than through `restore_all` because reproducing it
    /// end to end needs `proxy_running()` true, and a unit test cannot bind an
    /// engine. If either half ever changes, the guard in `enable_inner` is
    /// answering a question nobody is asking any more and this fails loudly.
    #[test]
    fn openai_has_no_route_of_its_own_when_codex_is_absent() {
        let openai = find("openai").expect("the openai provider is in the catalog");

        // Both its domains are `Credential::Additive`, so neither cascades.
        // This is the fact the provider's own comment states and the one that
        // makes the domain loop a no-op.
        assert!(
            cascade_domains(&openai).is_empty(),
            "openai's cascade should be empty, got {:?}",
            cascade_domains(&openai)
        );

        // And the engine being up is enough to make the plan claim work, which
        // is what skips the `NotYet` return. `nothing` is the only escape
        // `enable_inner` had before `NothingRoutable`.
        assert!(
            !enable_plan(false, true).nothing,
            "an undetected tool with the engine up must not read as nothing to do"
        );
        // The engine-down case keeps its old answer, so the new guard has not
        // swallowed the one that was already right.
        assert!(enable_plan(false, false).nothing);
    }

    /// The outcome the two restore paths now record for it.
    ///
    /// Pinned because the choice is load-bearing and invisible from the Rust
    /// side: `NotInstalled` has to be settled, or the entry goes straight back
    /// into the snapshot and AG-885 returns wearing a different label.
    #[test]
    fn the_outcome_for_nothing_routable_is_settled_not_outstanding() {
        assert!(!recovery::Outcome::NotInstalled.is_outstanding());
        assert!(recovery::Outcome::NotInstalled.is_complete());
        // The one it must not be confused with: `Pending` is what the bug left
        // behind, and it is outstanding, which is why the banner never cleared.
        assert!(recovery::Outcome::Pending.is_outstanding());
        assert!(!recovery::Outcome::Pending.is_complete());
    }

    /// `Applied::NotYet` is private, so the only place that can name it is a
    /// test in this module - and `restore_all`'s behavioural test cannot
    /// distinguish it, because after a master-off the provider reads off
    /// anyway and either half of that guard alone would hold the entry. This
    /// pins the half the integration test cannot see.
    ///
    /// Redirects the per-user paths, so it takes [`crate::env::path_env_lock`]:
    /// without it a test that depends on the *absence* of the seam can read
    /// this one's throwaway home instead of the real filesystem. The secrets
    /// seam goes with it, or `account::save` writes the key into the
    /// developer's own keyring.
    #[test]
    fn an_enable_with_nothing_to_do_yet_says_so() {
        let _lock = crate::env::path_env_lock();
        let home = std::env::temp_dir().join(format!(
            "gate-provider-notyet-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = fs::remove_dir_all(&home);
        fs::create_dir_all(home.join("secrets")).unwrap();
        let prev_home = std::env::var_os("GATE_CONNECT_TEST_HOME");
        let prev_secrets = std::env::var_os("GATE_CONNECT_TEST_SECRETS");
        std::env::set_var("GATE_CONNECT_TEST_HOME", &home);
        std::env::set_var("GATE_CONNECT_TEST_SECRETS", home.join("secrets"));

        let applied = (|| {
            account::save("https://gw.example.com", Some("sk-gw-testkey123"))?;
            // The shape the pre-engine restore pass meets: no engine running,
            // and `anthropic`'s only config tool skipped because the user had
            // switched it off before routing stopped. Skipping it also makes
            // the result independent of whether Claude Code happens to be
            // installed on the machine running this - `detect` consults real
            // binary paths, which no test home redirects.
            enable_skipping("anthropic", &["claude-code".to_string()]).map(|(applied, _)| applied)
        })();

        match prev_home {
            Some(v) => std::env::set_var("GATE_CONNECT_TEST_HOME", v),
            None => std::env::remove_var("GATE_CONNECT_TEST_HOME"),
        }
        match prev_secrets {
            Some(v) => std::env::set_var("GATE_CONNECT_TEST_SECRETS", v),
            None => std::env::remove_var("GATE_CONNECT_TEST_SECRETS"),
        }
        let _ = fs::remove_dir_all(&home);

        assert_eq!(
            applied.expect("a skipped family is not an error"),
            Applied::NotYet,
            "reporting this as Enabled is what let the pre-engine pass clear \
             the snapshot the post-engine pass needed"
        );
    }

    /// A restore pass with nothing to do yet says so **whatever the skip list
    /// holds**, which is the half `an_enable_with_nothing_to_do_yet_says_so`
    /// cannot see.
    ///
    /// That test supplies a skip list, and the guard it was testing read
    /// `!skip.is_empty()` - so the ordinary case, a restore where nothing had
    /// been switched off beforehand, fell through to the error written for a
    /// user who asked for the provider by name. `restore_all` recorded it as
    /// `WriteFailed`, and the recovery summary told somebody mid-master-on that
    /// Gate could not write a config file OpenRouter does not have, over a
    /// message advising them to turn on the routing they were turning on.
    ///
    /// `openrouter` is the sharpest case: `tool_ids` is empty, so there is never
    /// a tool to detect and the engine is the only thing that could give this
    /// call something to do.
    #[test]
    fn a_restore_with_nothing_to_do_yet_says_so_with_no_skip_list() {
        let _lock = crate::env::path_env_lock();
        let home = std::env::temp_dir().join(format!(
            "gate-provider-noskip-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = fs::remove_dir_all(&home);
        fs::create_dir_all(home.join("secrets")).unwrap();
        let prev_home = std::env::var_os("GATE_CONNECT_TEST_HOME");
        let prev_secrets = std::env::var_os("GATE_CONNECT_TEST_SECRETS");
        std::env::set_var("GATE_CONNECT_TEST_HOME", &home);
        std::env::set_var("GATE_CONNECT_TEST_SECRETS", home.join("secrets"));

        let applied = (|| {
            account::save("https://gw.example.com", Some("sk-gw-testkey123"))?;
            enable_skipping("openrouter", &[]).map(|(applied, _)| applied)
        })();

        match prev_home {
            Some(v) => std::env::set_var("GATE_CONNECT_TEST_HOME", v),
            None => std::env::remove_var("GATE_CONNECT_TEST_HOME"),
        }
        match prev_secrets {
            Some(v) => std::env::set_var("GATE_CONNECT_TEST_SECRETS", v),
            None => std::env::remove_var("GATE_CONNECT_TEST_SECRETS"),
        }
        let _ = fs::remove_dir_all(&home);

        assert_eq!(
            applied.expect("a restore pass with nothing to do yet is not an error"),
            Applied::NotYet,
            "an error here is journalled as WriteFailed, and the summary then \
             reports a failed config write that never happened"
        );
    }

    /// The by-name caller keeps its explanation. The fix must not turn the
    /// user's own click into a silent no-op: they asked for this provider, and
    /// nothing happening is a result that needs a sentence.
    #[test]
    fn a_by_name_enable_with_nothing_to_do_still_explains_itself() {
        let _lock = crate::env::path_env_lock();
        let home = std::env::temp_dir().join(format!(
            "gate-provider-byname-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = fs::remove_dir_all(&home);
        fs::create_dir_all(home.join("secrets")).unwrap();
        let prev_home = std::env::var_os("GATE_CONNECT_TEST_HOME");
        let prev_secrets = std::env::var_os("GATE_CONNECT_TEST_SECRETS");
        std::env::set_var("GATE_CONNECT_TEST_HOME", &home);
        std::env::set_var("GATE_CONNECT_TEST_SECRETS", home.join("secrets"));

        let out = (|| {
            account::save("https://gw.example.com", Some("sk-gw-testkey123"))?;
            enable("openrouter")
        })();

        match prev_home {
            Some(v) => std::env::set_var("GATE_CONNECT_TEST_HOME", v),
            None => std::env::remove_var("GATE_CONNECT_TEST_HOME"),
        }
        match prev_secrets {
            Some(v) => std::env::set_var("GATE_CONNECT_TEST_SECRETS", v),
            None => std::env::remove_var("GATE_CONNECT_TEST_SECRETS"),
        }
        let _ = fs::remove_dir_all(&home);

        let err = out.expect_err("nothing to configure is an error for a by-name enable");
        assert!(
            format!("{err:#}").contains("nothing to configure"),
            "got {err:#}"
        );
    }

    /// A snapshot entry naming a provider this build does not have is DROPPED,
    /// not retried.
    ///
    /// The regression this pins was permanent and self-sustaining. A stale
    /// `restore-snapshot.json` of `["google"]` - a provider an older build knew
    /// and this one does not - took the `Err` arm of `restore_all`'s loop,
    /// because `enable_inner` fails on `find` before it opens a file. That arm
    /// journalled `WriteFailed` and pushed the slug straight back into the
    /// snapshot, so the next resume produced the identical error, and so did
    /// every resume after it. On screen: a "Routing didn't finish - google is
    /// still waiting" card that no action could clear, with a Resume now that
    /// could not in principle succeed.
    ///
    /// `Outcome::Unknown` already existed for exactly this and the tool pass
    /// already used it; only the provider pass lacked the branch.
    #[test]
    fn a_snapshot_entry_for_an_unknown_provider_is_dropped_not_retried() {
        let _lock = crate::env::path_env_lock();
        let home = std::env::temp_dir().join(format!(
            "gate-provider-unknown-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = fs::remove_dir_all(&home);
        fs::create_dir_all(home.join("secrets")).unwrap();
        let prev_home = std::env::var_os("GATE_CONNECT_TEST_HOME");
        let prev_secrets = std::env::var_os("GATE_CONNECT_TEST_SECRETS");
        std::env::set_var("GATE_CONNECT_TEST_HOME", &home);
        std::env::set_var("GATE_CONNECT_TEST_SECRETS", home.join("secrets"));

        let outcome = (|| -> Result<(PendingRestore, PendingRestore)> {
            account::save("https://gw.example.com", Some("sk-gw-testkey123"))?;
            save_snapshot(PROVIDER_SNAPSHOT, &["google".to_string()])?;
            let before = pending_restore()?;
            // Best-effort like every caller: what matters is the snapshot after.
            let _ = restore_all();
            let after = pending_restore()?;
            Ok((before, after))
        })();

        match prev_home {
            Some(v) => std::env::set_var("GATE_CONNECT_TEST_HOME", v),
            None => std::env::remove_var("GATE_CONNECT_TEST_HOME"),
        }
        match prev_secrets {
            Some(v) => std::env::set_var("GATE_CONNECT_TEST_SECRETS", v),
            None => std::env::remove_var("GATE_CONNECT_TEST_SECRETS"),
        }
        let _ = fs::remove_dir_all(&home);

        let (before, after) = outcome.expect("the snapshot round-trip is not what is under test");
        assert!(
            before.providers.iter().any(|e| e.slug == "google"),
            "the test set this up wrong: google should start out pending, got {:?}",
            before.providers
        );
        assert!(
            !after.providers.iter().any(|e| e.slug == "google"),
            "an unresolvable slug survived the restore, so the recovery card is \
             permanent and Resume now can never clear it; got {:?}",
            after.providers
        );
    }

    /// The same slug through the **per-row Retry**, which is the other button.
    ///
    /// `restore_all` and `restore_one` reach the guard by different routes, and
    /// only the batch was covered. That matters here more than it usually would:
    /// the bug class this is about is "a card no action can clear", and Retry is
    /// half of what the user can press. `restore_one_provider` carries its own
    /// copy of the branch - it has to, because it has its own queue to rewrite -
    /// so a fix to one is not a fix to the other.
    #[test]
    fn the_per_row_retry_also_drops_an_unknown_provider() {
        let _lock = crate::env::path_env_lock();
        let home = std::env::temp_dir().join(format!(
            "gate-provider-unknown-retry-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = fs::remove_dir_all(&home);
        fs::create_dir_all(home.join("secrets")).unwrap();
        let prev_home = std::env::var_os("GATE_CONNECT_TEST_HOME");
        let prev_secrets = std::env::var_os("GATE_CONNECT_TEST_SECRETS");
        std::env::set_var("GATE_CONNECT_TEST_HOME", &home);
        std::env::set_var("GATE_CONNECT_TEST_SECRETS", home.join("secrets"));

        let outcome = (|| -> Result<(PendingRestore, Result<()>, PendingRestore)> {
            account::save("https://gw.example.com", Some("sk-gw-testkey123"))?;
            save_snapshot(PROVIDER_SNAPSHOT, &["google".to_string()])?;
            let before = pending_restore()?;
            let retry = restore_one("google");
            let after = pending_restore()?;
            Ok((before, retry, after))
        })();

        match prev_home {
            Some(v) => std::env::set_var("GATE_CONNECT_TEST_HOME", v),
            None => std::env::remove_var("GATE_CONNECT_TEST_HOME"),
        }
        match prev_secrets {
            Some(v) => std::env::set_var("GATE_CONNECT_TEST_SECRETS", v),
            None => std::env::remove_var("GATE_CONNECT_TEST_SECRETS"),
        }
        let _ = fs::remove_dir_all(&home);

        let (before, retry, after) =
            outcome.expect("the snapshot round-trip is not what is under test");
        assert!(
            before.providers.iter().any(|e| e.slug == "google"),
            "the test set this up wrong: google should start out pending, got {:?}",
            before.providers
        );
        // Not an `Err`: nothing failed, the answer is simply settled. A retry
        // that reported failure here would redraw the card it just cleared.
        assert!(
            retry.is_ok(),
            "an unresolvable slug is settled, not a failure; got {:?}",
            retry.err().map(|e| format!("{e:#}"))
        );
        assert!(
            !after.providers.iter().any(|e| e.slug == "google"),
            "the per-row Retry left an unresolvable slug in the snapshot, so the \
             row comes back and the button can never clear it; got {:?}",
            after.providers
        );
    }

    #[test]
    fn openai_provider_governs_codex_and_no_domain_at_all() {
        // The `openai` domain used to hang here. It is api.openai.com, and no
        // OpenAI tool Gate configures rides its switch: Codex routes through the
        // relay, which resolves against the whole catalog rather than the
        // enabled set, and the ChatGPT desktop app talks to chatgpt.com. What
        // the switch governs is generic interception of that host, whose real
        // dependants are whatever else on the machine talks to that host, which
        // is what `Client::AnyApp` now says in the catalog.
        //
        // Asserted against the DERIVED cascade rather than an array's contents:
        // the family lists two domains now, and what must stay empty is the set
        // the switch can flip.
        let p = find("openai").expect("openai provider present");
        assert_eq!(p.display_name, "OpenAI");
        assert!(p.tool_ids.contains(&ToolId::Codex));
        assert!(
            cascade_domains(&p).is_empty(),
            "a brokered domain here rejoins the family cascade, got {:?}",
            cascade_domains(&p)
        );
    }

    #[test]
    fn the_chatgpt_domains_are_not_reachable_by_enabling_the_openai_provider() {
        // Both chatgpt.com entries stay off this switch, because `enable` turns
        // on EVERY domain a provider lists: hanging them here would intercept
        // that host for every OpenAI user, including the API-key users who never
        // call it. Both are `Credential::Additive`, so the derived cascade
        // skips them while the family still lists them for the ledger - its own
        // row, its own switch. Codex needs neither slug enabled: its embedded
        // agent ignores the system proxy and routes via the relay, which
        // resolves slugs off the catalog rather than off the enabled flags.
        //
        // This is the test that would have caught the old failure mode. Under
        // two arrays it asserted a slug's absence from one of them; now it
        // asserts the consequence, so moving an entry between arrays cannot
        // pass it any more.
        let p = find("openai").expect("openai provider present");
        assert!(p.domain_slugs.contains(&"chatgpt"));
        assert!(p.domain_slugs.contains(&"chatgpt-apps"));
        assert!(!cascade_domains(&p).contains(&"chatgpt"));
        assert!(!cascade_domains(&p).contains(&"chatgpt-apps"));
    }

    #[test]
    fn anthropic_provider_maps_to_claude_code_and_anthropic_domain() {
        let p = find("anthropic").expect("anthropic provider present");
        assert_eq!(p.display_name, "Anthropic");
        assert!(p.tool_ids.contains(&ToolId::ClaudeCode));
        assert_eq!(p.domain_slugs, &["anthropic", "claude-web"]);
        assert_eq!(cascade_domains(&p), vec!["anthropic"]);
    }

    #[test]
    fn openrouter_provider_is_proxy_only() {
        let p = find("openrouter").expect("openrouter provider present");
        assert_eq!(p.display_name, "OpenRouter");
        assert!(
            p.tool_ids.is_empty(),
            "OpenRouter has no CLI integration - it's proxy-only"
        );
        assert_eq!(p.domain_slugs, &["openrouter"]);
        assert_eq!(cascade_domains(&p), vec!["openrouter"]);
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
        // `enable` flips every domain [`cascade_domains`] returns. If that ever
        // included the chat domain, enabling Claude would route the user's
        // claude.ai SESSION cookie as a side effect, bypassing the opt-in
        // default that is the only thing keeping it off.
        //
        // The family lists it - that is what puts it on the ledger - and the
        // credential is what keeps it out of the cascade. Both halves asserted,
        // because the bug this pins is exactly the two coming apart.
        let p = find("anthropic").expect("anthropic provider present");
        assert!(p.domain_slugs.contains(&"claude-web"));
        assert!(!cascade_domains(&p).contains(&"claude-web"));
        assert_eq!(cascade_domains(&p), vec!["anthropic"]);
    }

    #[test]
    fn session_domains_stay_listed_while_staying_out_of_the_cascade() {
        // The other half of the test above, and the half that keeps the fix in
        // place: a domain excluded from the cascade used to be excluded from
        // the family's only array, which is also what hid it from Home - so
        // "not cascaded" and "dropped" were indistinguishable. One array plus a
        // derived rule separates them by construction, and this asserts both
        // halves for every additive entry rather than for two named ones.
        let anthropic = find("anthropic").expect("anthropic provider present");
        assert!(anthropic.domain_slugs.contains(&"claude-web"));
        let openai = find("openai").expect("openai provider present");
        assert_eq!(openai.domain_slugs, &["chatgpt", "chatgpt-apps"]);
        assert!(cascade_domains(&openai).is_empty());

        let catalog = crate::proxy::default_domains();
        for p in providers() {
            let cascaded = cascade_domains(&p);
            for slug in p.domain_slugs {
                // Every slug named must exist in the catalog, or the row is
                // promised and never rendered.
                let entry = catalog
                    .iter()
                    .find(|d| d.slug == *slug)
                    .unwrap_or_else(|| panic!("{} names an unknown domain: {slug}", p.slug));
                // And the derived rule must agree with the entry, in both
                // directions: a brokered row the switch cannot reach is a dead
                // switch, an additive row it can reach is the bug above.
                assert_eq!(
                    cascaded.contains(slug),
                    entry.credential.cascades(),
                    "{}'s {slug} cascades={} but its credential says {:?}",
                    p.slug,
                    cascaded.contains(slug),
                    entry.credential
                );
            }
        }
    }
}
