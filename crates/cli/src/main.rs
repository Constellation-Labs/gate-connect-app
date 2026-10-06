//! `gate-connect` - prototype CLI for Gate Connect.
//!
//! Per the PRD this is one of three coordinated surfaces (desktop app,
//! web recipe pages, CLI). The Tauri desktop app and this CLI share the
//! same `gate-connect-core` crate, so anything testable here is testable
//! through the eventual GUI too.

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use gate_connect_core::{account, oauth, org, registry, ConnectInput, Status, ToolId};

// The built-in proxy is wired on the three desktop OSes (CA trust +
// system-proxy backends exist there); its subcommands are gated to match.
#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
use clap::ValueEnum;
#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
use gate_connect_core::proxy;

#[derive(Parser)]
#[command(
    name = "gate-connect",
    version,
    about = "Configure AI agent tools to route through Constellation Gate AI."
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Sign in to Gate AI. Stores the base URL on disk and the credential in
    /// the OS secret store (Keychain / Credential Manager / Secret Service).
    /// Re-run to update. With `--oauth`, signs in through the Constellation
    /// (Cognito) Hosted UI in the browser instead of a pasted API key.
    Login {
        #[arg(long, env = "GATE_BASE_URL")]
        base_url: String,
        #[arg(long)]
        api_key: Option<String>,
        /// Read the Gate API key from this file (first line) instead of
        /// passing it on the command line or typing it at the prompt.
        #[arg(long)]
        api_key_file: Option<std::path::PathBuf>,
        /// Sign in via the Constellation Hosted UI (OAuth) instead of an API
        /// key. Prints a URL to open in your browser and captures the redirect
        /// on a loopback listener. Cannot be combined with `--api-key` or
        /// `--api-key-file`.
        #[arg(long)]
        oauth: bool,
        /// With `--oauth`, preselect this organization (its UUID or slug)
        /// instead of prompting. Auto-selected when you belong to only one.
        #[arg(long)]
        org: Option<String>,
    },
    /// Sign out. Disconnects every tool Gate manages first (a failure there
    /// aborts the sign-out), then removes the stored base URL and the keychain
    /// entry.
    Logout,
    /// Show or change who pays the upstream provider.
    ///
    /// `byok` (the default) forwards each tool's own provider credential and
    /// the provider bills you directly. `payg` sends neither, so Gate routes
    /// through your workspace's provider accounts and debits its prepaid
    /// balance - top up in the dashboard first, since a funded balance is what
    /// activates it. Run with no argument to print the current mode.
    BillingMode {
        /// `byok` or `payg`. Omit to print the current mode.
        mode: Option<String>,
    },
    /// Show the currently signed-in gateway URL, if any.
    Whoami,
    /// List supported tools and their current state.
    List,
    /// Show detailed status for one tool.
    Status {
        /// Tool slug, e.g. `codex`.
        tool: String,
    },
    /// Point a tool at the Gate AI gateway.
    Connect { tool: String },
    /// Revert a tool back to its prior configuration.
    Disconnect { tool: String },
    /// Choose the model a tool runs on.
    ///
    /// `--gate` writes Gate models into the tool's own config: the first is its
    /// default, and the whole set is what its own model picker offers. They are
    /// served by Gate on your organization's credits, and a model outside the
    /// set is refused. `--app-default` puts the tool back on its own model. With
    /// neither, prints the current choice and what the tool's config holds.
    Model {
        /// Tool slug, e.g. `codex`.
        tool: String,
        /// Gate model ids, comma-separated, at most 4, e.g.
        /// `openai/gpt-5.6-luna,anthropic/claude-opus-5`.
        #[arg(long, value_delimiter = ',', conflicts_with = "app_default")]
        gate: Vec<String>,
        /// Go back to the tool's own model.
        #[arg(long)]
        app_default: bool,
        /// Accept that Gate models are billed to your organization's Gate
        /// credits. Needed once per install, the first time `--gate` is used.
        #[arg(long)]
        accept_paid: bool,
    },
    /// Manage the built-in MITM proxy that routes config-less apps
    /// (Claude Desktop, ChatGPT, …) and command-line tools through the Gate
    /// gateway. Enabling installs a local CA and points the system proxy at a
    /// loopback listener; only enabled provider domains are intercepted -
    /// every other host is tunnelled untouched.
    #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
    Proxy {
        #[command(subcommand)]
        command: ProxyCmd,
    },
}

#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
#[derive(Subcommand)]
enum ProxyCmd {
    /// Show whether the proxy is running, its port, CA trust, and the
    /// provider domains.
    Status,
    /// Turn the proxy on: trust the local CA and route the system proxy
    /// through the loopback engine. May prompt for elevation.
    ///
    /// On macOS and Windows the engine lives in this process, so the command
    /// stays in the foreground hosting it. Stopping it (Ctrl-C, closing the
    /// terminal, or SIGTERM on macOS) removes Gate from every tool's config, as
    /// quitting the app does, then stops routing and restores the prior
    /// system-proxy state. The next enable reconnects the tools. Returning instead
    /// would take the engine down with the process and leave the system proxy
    /// pointed at a port nothing answers.
    Enable {
        /// Linux only: stay in the foreground until Ctrl-C, SIGTERM or SIGHUP, then
        /// stop routing and restore the prior system-proxy state. Without it
        /// the command returns and the helper daemon keeps routing. For a
        /// systemd unit or a CI job that should own the routing lifetime.
        #[cfg(target_os = "linux")]
        #[arg(long)]
        foreground: bool,
    },
    /// Turn the proxy off and restore the prior system-proxy state.
    ///
    /// On macOS and Windows this is recovery for a host that stopped without
    /// restoring (killed, crashed): it refuses while a Gate process is still
    /// routing, since that one does the restore itself when it stops.
    Disable,
    /// Host ONLY the loopback reverse-proxy relay; blocks until killed.
    ///
    /// For environments with no desktop app (containers, servers, CI): CLI
    /// tools whose config points at the relay route through Gate with the live
    /// credential. No CA trust and no system-proxy changes, so nothing else on
    /// this machine is routed - `enable` is the one that does that, and it
    /// hosts this same relay, so the two are alternatives rather than steps.
    /// Sign in first.
    Relay,
    /// List routable provider domains and whether each is enabled.
    Domains,
    /// Enable or disable routing for one provider domain.
    Domain {
        /// Provider slug, e.g. `anthropic`.
        slug: String,
        /// `on` to route this provider through Gate, `off` to stop.
        state: Toggle,
    },
    /// Trust the local proxy CA without turning the proxy on.
    TrustCa {
        /// Install the CA machine-wide instead of for this user, with no
        /// dialog. For hosts where nobody can answer one: build agents,
        /// containers, headless servers. Needs root (macOS/Linux) or an
        /// elevated prompt (Windows), never prompts for it, and makes the CA a
        /// trusted TLS root for EVERY user on this machine. The default,
        /// per-user path and its confirmation dialog are what a desktop should
        /// use.
        #[arg(long)]
        system_trust: bool,
    },
    /// Remove the local proxy CA's trust. Requires the proxy to be off.
    UntrustCa {
        /// Remove a machine-wide install (the one `trust-ca --system-trust`
        /// makes) with no dialog. Same privileges, same non-interactive rule.
        #[arg(long)]
        system_trust: bool,
    },
}

#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
#[derive(Clone, Copy, ValueEnum)]
enum Toggle {
    On,
    Off,
}

fn main() -> Result<()> {
    // The proxy manager spawns the helper daemon as `<current-exe> --proxy-helper`
    // (Linux). When that current-exe is this CLI, handle the flag before clap
    // parses, so the daemon path works whether the proxy was enabled from the
    // app or the CLI. See `gate_connect_core::proxy::helper`.
    #[cfg(target_os = "linux")]
    if std::env::args().skip(1).any(|a| a == "--proxy-helper") {
        return gate_connect_core::proxy::helper::run_daemon();
    }

    let cli = Cli::parse();
    let result = match cli.command {
        Command::Login {
            base_url,
            api_key,
            api_key_file,
            oauth,
            org,
        } => cmd_login(base_url, api_key, api_key_file, oauth, org),
        Command::Logout => cmd_logout(),
        Command::Whoami => cmd_whoami(),
        Command::List => cmd_list(),
        Command::Status { tool } => cmd_status(&tool),
        Command::Connect { tool } => cmd_connect(&tool),
        Command::Disconnect { tool } => cmd_disconnect(&tool),
        Command::Model {
            tool,
            gate,
            app_default,
            accept_paid,
        } => cmd_model(&tool, gate, app_default, accept_paid),
        Command::BillingMode { mode } => cmd_billing_mode(mode),
        #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
        Command::Proxy { command } => cmd_proxy(command),
    };
    // Audit emits run on detached threads so they never stall a command; this
    // process ends when the command does, so wait for any still in flight
    // (bounded by the emit timeout) or they would be silently dropped.
    gate_connect_core::audit::flush();
    result
}

fn cmd_login(
    base_url: String,
    api_key: Option<String>,
    api_key_file: Option<std::path::PathBuf>,
    oauth: bool,
    org: Option<String>,
) -> Result<()> {
    if oauth {
        if api_key.is_some() || api_key_file.is_some() {
            anyhow::bail!("--oauth cannot be combined with --api-key / --api-key-file");
        }
        return cmd_login_oauth(base_url, org);
    }
    let api_key = resolve_secret(api_key, api_key_file, "Gate API key")?;
    account::save(&base_url, Some(&api_key))?;
    // Signing in with a key selects the legacy path explicitly, so a prior
    // `login --oauth` doesn't leave the account injecting a stale OAuth token
    // (the relay reads the mode via `access_token_for_injection`).
    account::set_auth_mode(account::AuthMode::ApiKey)?;
    // The proxy engine lives in whichever process enabled it (usually the
    // desktop app) - this process can't push the new key into it. The app's
    // refresh loop does, on its next tick, for an account not in OAuth mode.
    #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
    {
        if proxy::engine_likely_running() {
            println!(
                "note: the Gate proxy appears to be enabled in another process (likely the Gate Connect app). A running Gate Connect app switches it to this key within about 30 seconds; otherwise it keeps the previous credential until routing is restarted there."
            );
        }
    }
    println!("Signed in to {base_url}.");
    Ok(())
}

/// Sign in through the Constellation (Cognito) Hosted UI. Persists the gateway
/// first so the browser round-trip can record OAuth as the account's auth mode,
/// then prints the authorize URL and blocks on the loopback redirect. The token
/// bundle lands in the secret store; the relay / MITM engine inject it live, so
/// no credential is written to disk here.
fn cmd_login_oauth(base_url: String, org: Option<String>) -> Result<()> {
    let cfg = oauth::OAuthConfig::for_gateway(Some(&base_url)).context(
        "OAuth is not configured in this build for this gateway (GATE_COGNITO_HOSTED_DOMAIN / GATE_COGNITO_CLIENT_ID unset, or their _STAGING / _DEV variants for the staging and dev gateways)",
    )?;
    account::save(&base_url, None)?;
    let tokens = oauth::login(&cfg, oauth::REDIRECT_PORTS, |url| {
        println!("Open this URL in your browser to sign in:\n\n  {url}\n");
        Ok(())
    })?;
    account::set_auth_mode(account::AuthMode::OAuth)?;

    // The gateway requires an org on every OAuth request, so pick one now.
    let orgs = org::list(&base_url, &tokens.access_token)?;
    let chosen = select_org(&orgs, org.as_deref())?;
    account::set_org(&chosen.org_id, &chosen.name)?;

    match tokens.email() {
        Some(email) => println!("Signed in to {base_url} as {email} (org: {}).", chosen.name),
        None => println!("Signed in to {base_url} (org: {}).", chosen.name),
    }
    Ok(())
}

/// Resolve which org to use: an explicit `--org` (UUID or slug), the only org
/// when there's exactly one, or an interactive numbered prompt otherwise.
fn select_org<'a>(orgs: &'a [org::Org], preselect: Option<&str>) -> Result<&'a org::Org> {
    if orgs.is_empty() {
        anyhow::bail!(
            "no organizations are available for your account; ask an admin to add you to one"
        );
    }
    if let Some(sel) = preselect {
        return orgs
            .iter()
            .find(|o| o.org_id == sel || o.slug == sel)
            .with_context(|| {
                let available = orgs
                    .iter()
                    .map(|o| o.slug.as_str())
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("no org matches {sel:?} (available: {available})")
            });
    }
    if orgs.len() == 1 {
        return Ok(&orgs[0]);
    }
    println!("Select an organization:");
    for (i, o) in orgs.iter().enumerate() {
        println!("  {}. {} ({})", i + 1, o.name, o.slug);
    }
    print!("Enter number [1-{}]: ", orgs.len());
    use std::io::Write;
    std::io::stdout().flush().ok();
    let mut line = String::new();
    std::io::stdin()
        .read_line(&mut line)
        .context("reading org selection")?;
    let idx: usize = line
        .trim()
        .parse()
        .map_err(|_| anyhow::anyhow!("invalid selection {:?}", line.trim()))?;
    orgs.get(idx.wrapping_sub(1))
        .context("selection out of range")
}

/// Resolve a secret from, in order of precedence: an explicit value
/// (e.g. `--api-key`), a file whose first line holds the secret
/// (`--api-key-file`), or an interactive no-echo prompt. Reading from a
/// file or prompt keeps the secret out of the process environment and
/// `ps -E` output.
fn resolve_secret(
    value: Option<String>,
    file: Option<std::path::PathBuf>,
    label: &str,
) -> Result<String> {
    if let Some(v) = value {
        return Ok(v);
    }
    if let Some(path) = file {
        let contents = std::fs::read_to_string(&path)
            .with_context(|| format!("reading {}", path.display()))?;
        let line = contents.lines().next().unwrap_or("").trim();
        if line.is_empty() {
            anyhow::bail!("{} is empty", path.display());
        }
        return Ok(line.to_string());
    }
    let entered = rpassword::prompt_password(format!("{label}: "))
        .with_context(|| format!("reading {label} from prompt"))?;
    let entered = entered.trim().to_string();
    if entered.is_empty() {
        anyhow::bail!("no {label} provided");
    }
    Ok(entered)
}

fn cmd_logout() -> Result<()> {
    // Disconnect managed tools first: clearing the account while their
    // configs still embed the key would leave them routing to the gateway
    // with a dead credential on disk. A failure aborts the sign-out.
    registry::disconnect_all_managed()?;
    account::clear()?;
    // The proxy engine lives in whichever process enabled it (usually the
    // desktop app) - this process can't stop it or revoke its in-memory key.
    #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
    {
        if proxy::engine_likely_running() {
            println!(
                "note: the Gate proxy appears to be enabled in another process (likely the Gate Connect app); it keeps using the deleted key until routing stops there."
            );
        }
    }
    println!("Signed out.");
    Ok(())
}

fn cmd_whoami() -> Result<()> {
    match account::load_base_url()? {
        Some(url) => {
            println!("Signed in: {url}");
            // Who pays is not visible anywhere else on a headless machine, and
            // it decides whether traffic spends the workspace balance.
            println!(
                "Billing:   {}",
                billing_mode_label(account::billing_mode()?)
            );
        }
        None => println!("Not signed in. Run `gate-connect login --base-url … --api-key …`."),
    }
    Ok(())
}

fn billing_mode_label(mode: account::BillingMode) -> &'static str {
    match mode {
        account::BillingMode::Byok => "byok (your own provider keys)",
        account::BillingMode::Payg => "payg (billed to your Gate balance)",
    }
}

/// Print or switch the account's billing mode.
///
/// Switching rewrites nothing on its own beyond the account file: the relay and
/// the MITM engine read the mode per request, so routing follows immediately in
/// whichever process hosts them - except that Codex's provider block encodes
/// the mode, so it needs a reconnect, and this says so rather than silently
/// leaving it on the old shape.
fn cmd_billing_mode(mode: Option<String>) -> Result<()> {
    let Some(requested) = mode else {
        println!("{}", billing_mode_label(account::billing_mode()?));
        return Ok(());
    };
    let mode = match requested.to_ascii_lowercase().as_str() {
        "byok" => account::BillingMode::Byok,
        "payg" => account::BillingMode::Payg,
        other => anyhow::bail!("unknown billing mode {other:?} - expected `byok` or `payg`"),
    };
    account::set_billing_mode(mode)?;
    println!("Billing mode: {}", billing_mode_label(mode));

    // Codex is the one config integration whose file depends on the mode.
    if matches!(
        registry::find(ToolId::Codex).map(|i| i.status()),
        Some(Ok(Status::Connected))
            | Some(Ok(Status::Drifted(_)))
            | Some(Ok(Status::Overridden(_)))
    ) {
        println!(
            "note: run `gate-connect connect codex` to rewrite its provider block for this mode."
        );
    }
    #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
    {
        if proxy::engine_likely_running() {
            println!(
                "note: the Gate proxy appears to be enabled (likely in the menubar app); it keeps using the previous mode until it is toggled off and on."
            );
        }
    }
    Ok(())
}

fn cmd_list() -> Result<()> {
    println!("{:<14} {:<28} STATUS", "TOOL", "NAME");
    for integ in registry::registry() {
        let status = integ
            .status()
            .map(|s| s.to_string())
            .unwrap_or_else(|e| format!("error: {e}"));
        println!(
            "{:<14} {:<28} {}",
            integ.id().to_string(),
            integ.display_name(),
            status
        );
    }
    Ok(())
}

fn cmd_status(tool: &str) -> Result<()> {
    let integ = resolve(tool)?;
    let status = integ.status()?;
    println!("{}: {}", integ.display_name(), status);
    if matches!(status, Status::Connected) {
        match integ.id() {
            ToolId::ClaudeCode => {
                println!("Re-run `claude` to pick up the new settings.json env block.")
            }
            ToolId::Codex => {
                println!("Re-run `codex` to pick up the new config.toml provider block.")
            }
            ToolId::OpenCode => {
                println!("Re-run `opencode` to pick up the new opencode.json provider block.")
            }
            ToolId::OpenClaw => {
                println!("Re-run `openclaw` to pick up the new proxy setting in openclaw.json.")
            }
            ToolId::Hermes => {
                println!("Re-run `hermes` to pick up the new proxy settings in ~/.hermes/.env.")
            }
            ToolId::EnvProxy => {
                println!(
                    "Start a new shell (or relaunch your tools) - only processes started after the export see these variables."
                )
            }
        }
    }
    Ok(())
}

fn cmd_connect(tool: &str) -> Result<()> {
    let acct = account::load()?
        .context("Not signed in. Run `gate-connect login --base-url … --api-key …` first.")?;
    let integ = resolve(tool)?;
    let input = ConnectInput {
        gateway_base_url: acct.gateway_base_url,
        billing_mode: acct.billing_mode,
        relay_base_url: gate_connect_core::proxy::relay_base_url(),
        // `tool_proxy_url`, not the engine's own address: a config written here
        // has to name what the GUI writes, or the two disagree about the same
        // install and this one names a port that stops answering with its host.
        engine_proxy_url: gate_connect_core::proxy::tool_proxy_url(),
    };
    integ.connect(&input)?;
    println!("Connected {}.", integ.display_name());
    println!();
    println!("Next steps:");
    match integ.id() {
        ToolId::ClaudeCode => {
            println!(
                "  1. Quit any running `claude` sessions (they cache settings.json at launch)."
            );
            println!(
                "  2. Re-run `claude` - it picks up HTTPS_PROXY and NODE_EXTRA_CA_CERTS from ~/.claude/settings.json while keeping Anthropic's canonical base URL. If your shell already exports NODE_EXTRA_CA_CERTS, that one wins and has to trust Gate's CA too."
            );
            println!("  3. Select models normally: standard variants stay 200K and (1M) variants keep their 1M context through Gate.");
        }
        ToolId::Codex => {
            println!("  1. Quit any running `codex` sessions.");
            println!(
                "  2. Re-run `codex` - it reads ~/.codex/config.toml on launch and routes through the `gate` model provider. Gate handles upstream auth, no OPENAI_API_KEY needed."
            );
        }
        ToolId::OpenCode => {
            println!("  1. Quit any running `opencode` sessions.");
            println!(
                    "  2. Re-run `opencode` - your existing providers (anthropic / openai / openrouter) now route through Gate. Use the same model names you always have."
                );
            println!(
                    "  3. Your API keys from `opencode auth login <provider>` are untouched. Gate adds its headers and forwards each request to the original upstream."
                );
        }
        ToolId::OpenClaw => {
            println!("  1. Quit any running `openclaw` sessions.");
            println!(
                "  2. Re-run `openclaw` - it now sends its traffic through Gate's local proxy, so every provider you have configured routes, whichever one you use."
            );
            println!(
                "  3. Your provider credentials in ~/.openclaw/openclaw.json are untouched, and their base URLs are left exactly as you set them."
            );
        }
        ToolId::Hermes => {
            println!("  1. Quit any running `hermes` sessions.");
            println!(
                "  2. Re-run `hermes` - it reads ~/.hermes/.env on launch and sends its traffic through Gate's local proxy. Your providers in config.yaml are not repointed; Gate only names Hermes there, and adds its own provider if you choose Gate models (`gate-connect model hermes`)."
            );
            println!(
                "  3. Your upstream credentials are untouched. Gate injects its own in flight and forwards each request to the original upstream."
            );
        }
        ToolId::EnvProxy => {
            println!(
                "  1. Gate's proxy is now in your environment (HTTPS_PROXY, NO_PROXY, NODE_EXTRA_CA_CERTS)."
            );
            println!(
                "  2. Start a new shell, then re-run OpenCode or any other tool that reads HTTPS_PROXY. Already-running processes keep the old environment."
            );
            println!(
                "  3. This is machine-wide: git, curl and npm go through Gate's proxy too. It blind-tunnels anything Gate does not intercept, and `gate-connect disconnect env-proxy` takes it back out."
            );
        }
    }
    Ok(())
}

fn cmd_model(tool: &str, gate: Vec<String>, app_default: bool, accept_paid: bool) -> Result<()> {
    use gate_connect_core::preferences::{self, ModelSource};
    use gate_connect_core::registry::GateModelState;
    use gate_connect_core::tool_models;

    let integ = resolve(tool)?;
    if !integ.supports_gate_models() {
        anyhow::bail!("{} does not support Gate models yet", integ.display_name());
    }
    let slug = integ.id().slug();
    let name = integ.display_name();

    if gate.is_empty() && !app_default {
        let view = tool_models::states().remove(slug);
        if let Some(v) = view.as_ref().filter(|v| v.left_gate_models.is_some()) {
            let to = v.left_gate_models.clone().flatten();
            println!(
                "{name} was moved off Gate models from inside {name}{}; it is back on its own model.",
                to.map(|m| format!(" (to {m})")).unwrap_or_default()
            );
        }
        let prefs = preferences::load();
        match prefs.tool_models.get(slug) {
            Some(c) if c.source == ModelSource::Gate => {
                println!("Choice: Gate models {}", c.model_ids.join(", "))
            }
            Some(c) if !c.model_ids.is_empty() => println!(
                "Choice: App default (remembered Gate models: {})",
                c.model_ids.join(", ")
            ),
            _ => println!("Choice: App default"),
        }
        match view.map(|v| v.state) {
            Some(GateModelState::Applied { model }) => {
                println!("{name}'s config: on Gate models, starting on {model}")
            }
            Some(GateModelState::Drifted { model }) => println!(
                "{name}'s config: moved off Gate models{}",
                model.map(|m| format!(" (names {m})")).unwrap_or_default()
            ),
            _ => println!("{name}'s config: its own model"),
        }
        return Ok(());
    }

    let (source, ids) = if app_default {
        let kept = preferences::load()
            .tool_models
            .get(slug)
            .map(|c| c.model_ids.clone())
            .unwrap_or_default();
        (ModelSource::Tool, kept)
    } else {
        if preferences::load().gate_model_paid_ack_unix.is_none() && !accept_paid {
            anyhow::bail!(
                "Gate models are billed to your organization's Gate credits. Re-run with \
                 --accept-paid to confirm."
            );
        }
        (ModelSource::Gate, gate)
    };
    let meta = match source {
        ModelSource::Gate => gate_connect_core::gate_models::catalogue_json()
            .map(|json| tool_models::meta_from_catalogue(&json, &ids))
            .unwrap_or_default(),
        ModelSource::Tool => Vec::new(),
    };
    let applied = tool_models::choose(integ.id(), source, ids.clone(), accept_paid, meta)?;
    let what = match source {
        ModelSource::Gate => format!("Gate models {}", ids.join(", ")),
        ModelSource::Tool => "its own model".to_string(),
    };
    if applied {
        println!("{name} is set to {what}. Restart running {name} sessions to pick it up.");
    } else {
        println!(
            "{name} is set to {what}. Gate is not managing {name}'s config right now, so it \
             applies the next time you run `gate-connect connect {slug}`."
        );
    }
    Ok(())
}

fn cmd_disconnect(tool: &str) -> Result<()> {
    let integ = resolve(tool)?;
    integ.disconnect()?;
    println!("Disconnected {}.", integ.display_name());
    match integ.id() {
        ToolId::ClaudeCode => {
            println!("Restart any running `claude` sessions for the change to take effect.")
        }
        ToolId::Codex => {
            println!("Restart any running `codex` sessions for the change to take effect.")
        }
        ToolId::OpenCode => {
            println!("Restart any running `opencode` sessions for the change to take effect.")
        }
        ToolId::OpenClaw => {
            println!("Restart any running `openclaw` sessions for the change to take effect.")
        }
        ToolId::Hermes => {
            println!("Restart any running `hermes` sessions for the change to take effect.")
        }
        ToolId::EnvProxy => {
            println!(
                "Start a new shell for the change to take effect - already-running processes keep the variables until they are relaunched."
            )
        }
    }
    Ok(())
}

#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
fn cmd_proxy(command: ProxyCmd) -> Result<()> {
    // Inside another app's MSIX package (a terminal in Claude Desktop's Code
    // tab), Windows sends the CA and system-proxy writes to that app's private
    // registry, and `status` would read that copy back. The app relaunches
    // itself out; this command hosts the engine in the foreground, so it
    // refuses instead.
    #[cfg(target_os = "windows")]
    if gate_connect_core::env::in_foreign_package() {
        anyhow::bail!(
            "started inside another app's package (a terminal in Claude Desktop?). \
             Windows would send Gate's proxy and CA writes to that app's private \
             registry. Run it from a terminal outside the app."
        );
    }
    let mgr = proxy::manager();
    // This process exits the moment the command finishes, so its lifetime must
    // not bound the routing lifetime. Without this the Linux daemon reverted
    // the engine to pass-through as soon as `proxy enable` returned: it cleared
    // every rule, blind-tunnelled all traffic to the user's own provider, and
    // left `proxy status` reporting the domains as on. Set for the whole
    // subcommand rather than just Enable, because any command that reaches
    // `SetIntercept` (a domain toggle, for one) carries the same flag. No-op on
    // macOS and Windows, which run the engine in-process.
    mgr.set_detached(true);
    match command {
        ProxyCmd::Status => print_proxy_state(&mgr.status()?),
        ProxyCmd::Enable {
            #[cfg(target_os = "linux")]
            foreground,
        } => {
            // Only Linux has a daemon to outlive this process; elsewhere the
            // engine is ours, so returning would end routing on the way out.
            #[cfg(not(target_os = "linux"))]
            let foreground = true;
            // The same master-ON ceremony as the app (`routing::enable`):
            // persist the intent, restore providers around the engine start
            // (the all-off state would otherwise trip `enable`'s "at least
            // one provider" precondition), and keep best-effort hiccups as
            // notes.
            let (state, warnings) = gate_connect_core::routing::enable()?;
            for w in warnings {
                eprintln!("note: {} failed: {:#}", w.component, w.error);
            }
            println!("Proxy enabled.");
            print_proxy_state(&state);
            print_proxy_hint();
            if foreground {
                println!();
                #[cfg(target_os = "linux")]
                println!(
                    "Hosting the proxy engine. Press Ctrl-C to stop routing and restore the \
                     previous system-proxy settings."
                );
                #[cfg(not(target_os = "linux"))]
                println!(
                    "Hosting the proxy engine. Press Ctrl-C to stop: Gate is removed from \
                     tool configs, and the previous system-proxy settings are restored."
                );
                proxy::wait_for_shutdown().context(
                    "waiting for a stop signal failed with routing still on; run \
                     `gate-connect proxy disable` to restore the system proxy",
                )?;
                // Restoring here is the point of blocking: a service manager
                // sends SIGTERM, and an engine that vanished without reverting
                // would leave the machine pointed at a dead loopback port.
                println!();
                // Stopping this host is a quit, not a routing toggle.
                // `routing::disable` alone parks the engine so configs naming
                // its relay keep answering, which only helps a process that
                // stays alive; on macOS and Windows the relay lives here and
                // goes with us. So first take Gate out of every tool's config
                // and drain the forwarder, exactly as the app's quit does
                // (`quit_app`); the next enable or app launch reconnects them.
                // Linux skips it, as the app does: the daemon outlives us and
                // keeps answering.
                #[cfg(not(target_os = "linux"))]
                match gate_connect_core::provider::snapshot_and_disable_everything_for_exit() {
                    Ok(teardown) => {
                        gate_connect_core::proxy::forwarder::drain();
                        if !teardown.failed.is_empty() {
                            eprintln!(
                                "note: failed to remove Gate from the {} config(s); edit them by hand.",
                                teardown.failed.join(", ")
                            );
                        } else if teardown.managed > 0 {
                            println!(
                                "Removed Gate from tool configs; they reconnect the next time routing is enabled."
                            );
                        }
                    }
                    Err(e) => eprintln!(
                        "note: failed to remove Gate from tool configs ({e:#}); edit them by hand."
                    ),
                }
                disable_routing().context(
                    "restoring the system proxy failed; run `gate-connect proxy disable` to \
                     finish",
                )?;
            }
        }
        ProxyCmd::Disable => {
            // On macOS and Windows a live host - the app, or a foreground
            // `proxy enable` - restores the system proxy itself when it stops,
            // from the snapshot it took. Restoring from here underneath it
            // deletes that snapshot, so its own stop later finds none and
            // falls back to forcing every proxy setting off, the user's own
            // included. So only clean up after a host that is gone; the check
            // asks the relay to prove it is a routing Gate, which a stale
            // snapshot or a stranger on the port cannot fake.
            #[cfg(not(target_os = "linux"))]
            if let Some(port) = proxy::engine_hosted_elsewhere() {
                anyhow::bail!(
                    "the Gate proxy is being routed by another process on 127.0.0.1:{port}. Stop \
                     it there instead - press Ctrl-C in the terminal running `gate-connect proxy \
                     enable`, or quit the Gate Connect app - and it restores the system proxy \
                     itself. `proxy disable` is for cleaning up after one that stopped without \
                     restoring."
                );
            }
            // The app's master-OFF ceremony (`routing::disable`), not a bare
            // engine stop: the sweep turns the providers off and parks the
            // engine, leaving tool configs naming Gate so they pass straight
            // through while it is parked, and clearing the intent keeps a later
            // app launch from silently re-routing what the operator just
            // turned off.
            disable_routing()?;
        }
        ProxyCmd::Relay => {
            // Blocks until killed; hosts only the relay (no CA, no system proxy).
            proxy::serve_relay()?;
        }
        ProxyCmd::Domains => print_proxy_domains(&mgr.list_domains()?),
        ProxyCmd::Domain { slug, state } => {
            let enabled = matches!(state, Toggle::On);
            let st = mgr.set_domain(&slug, enabled)?;
            // Audited at the command layer, mirroring the app's
            // `proxy_set_domain`: `provider::enable` / `disable` drive
            // `set_domain` internally, so instrumenting the manager would turn
            // one operator action into N+1 events. This arm is the operator
            // toggling one domain by hand from the CLI.
            if let Ok(Some(base_url)) = account::load_base_url() {
                gate_connect_core::audit::domain_toggled(&base_url, None, &slug, enabled);
            }
            println!("{} {slug}.", if enabled { "Enabled" } else { "Disabled" });
            // The GUI raises a dialog before this exact act, because a row whose
            // credential does not cascade carries the session the operator is
            // already signed in with rather than a key Gate brokers. The CLI
            // cannot ask - the toggle has happened by the time anything could -
            // so it says what it did. The table below carries the same two facts
            // in its columns, and somebody toggling one domain by name never
            // reads it.
            if enabled {
                if let Some(d) = st.domains.iter().find(|d| d.slug == slug) {
                    if !d.credential.cascades() {
                        println!(
                            "note: {slug} carries the credential you are already signed in with, \
                             not a key Gate brokers. Gate now records and inspects that traffic \
                             on {}.",
                            d.hosts.join(", ")
                        );
                    }
                }
            }
            print_proxy_domains(&st.domains);
        }
        ProxyCmd::TrustCa { system_trust } => {
            if system_trust {
                // Said before it happens, not after. This is the one trust path
                // with no OS dialog to describe what is about to change, so the
                // description has to come from us.
                println!(
                    "Installing the proxy CA machine-wide. It becomes a trusted TLS root for every user on this host, and nothing will ask for confirmation."
                );
                let state = mgr.trust_ca_system()?;
                println!("Proxy CA trusted machine-wide.");
                print_browser_restart(&state);
                println!("Remove it with `gate-connect proxy untrust-ca --system-trust`.");
            } else {
                let state = mgr.trust_ca()?;
                println!("Proxy CA trusted.");
                print_browser_restart(&state);
            }
        }
        ProxyCmd::UntrustCa { system_trust } => {
            // Untrusting stops routing rather than refusing while it is on, so
            // say so: nothing else on this path would tell the user their
            // traffic stopped going through Gate.
            let was_routing = gate_connect_core::proxy::engine_likely_running();
            let state = if system_trust {
                let state = mgr.untrust_ca_system()?;
                println!("Machine-wide proxy CA trust removed.");
                state
            } else {
                let state = mgr.untrust_ca()?;
                println!("Proxy CA trust removed.");
                state
            };
            print_browser_removal(&state);
            if was_routing {
                println!(
                    "Routing was on and has been stopped: the engine signs with this CA, so it \
                     cannot run once the CA is untrusted. `gate-connect proxy enable` trusts a new \
                     one and turns routing back on."
                );
            }
        }
    }
    Ok(())
}

/// `routing::disable` and its notes, shared by `proxy disable` and the
/// foreground host's stop so the two report the same way.
#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
fn disable_routing() -> Result<()> {
    let (_, warnings) = gate_connect_core::routing::disable()?;
    for w in warnings {
        eprintln!("note: {} failed: {:#}", w.component, w.error);
    }
    println!("Proxy disabled; prior system-proxy state restored.");
    Ok(())
}

#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
fn print_proxy_state(state: &proxy::ProxyState) {
    let running = match (state.running, state.port) {
        (true, Some(p)) => format!("running on 127.0.0.1:{p}"),
        (true, None) => "running".to_string(),
        (false, _) => "stopped".to_string(),
    };
    println!("Proxy:    {running}");
    println!(
        "CA trust: {}",
        if state.ca_trusted {
            "trusted"
        } else {
            "not trusted"
        }
    );
    print_proxy_domains(&state.domains);
    print_browser_restart(state);
}

/// The CLI's counterpart of the window's "quit and reopen" note: a browser only
/// reads a newly added root at launch, and a write made in this process never
/// reaches an open window's counter. Silent when nothing was written (always,
/// off Linux).
#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
fn print_browser_restart(state: &proxy::ProxyState) {
    if state.ca_nss_written_at > 0 {
        println!("Certificate added to your browsers. {BROWSER_RESTART}");
    }
}

/// The sentence every certificate note ends on - `BROWSER_RESTART` in
/// `src/lib/groups.ts` - so the CLI and the window say the same thing.
#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
const BROWSER_RESTART: &str = "Quit and reopen any open browser so it trusts the certificate.";

/// The removal's counterpart, `BROWSER_REMOVED_RESTART` in `src/lib/groups.ts`.
#[cfg(target_os = "linux")]
const BROWSER_REMOVED_RESTART: &str =
    "Quit and reopen any open browser so it stops trusting the certificate.";

/// What a removal did to the browser stores, on Linux, where a running browser
/// keeps the stores it read at launch: one still open goes on trusting a root
/// just taken out of them. Says so only where a store actually lost it, and
/// says plainly when one would not let go - the warnings above name which.
/// Silent off Linux, where a running process re-evaluates trust.
#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
fn print_browser_removal(state: &proxy::ProxyState) {
    #[cfg(target_os = "linux")]
    {
        use gate_connect_core::proxy::NssTrust;
        match state.ca_nss_trust {
            Some(NssTrust::ToolsMissing) => println!(
                "A browser may still trust the certificate: certutil is not installed, so Gate \
                 could not remove it from the browsers' own stores. Remove the Gate Connect \
                 certificate in each browser's certificate settings."
            ),
            Some(NssTrust::WriteFailed | NssTrust::NotWritten) => println!(
                "A browser still trusts the certificate: one of the browsers' own stores would \
                 not let go of it (see the warnings above). Remove the Gate Connect certificate \
                 in that browser's certificate settings."
            ),
            Some(NssTrust::Trusted) | None => {
                if proxy::ca::nss_removals() > 0 {
                    println!("Certificate removed from your browsers. {BROWSER_REMOVED_RESTART}");
                }
            }
        }
    }
    #[cfg(not(target_os = "linux"))]
    let _ = state;
}

#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
fn print_proxy_domains(domains: &[proxy::ProxyDomain]) {
    // Client, scope and credential beside the flag, for the same reason the
    // diagnostics report carries them: on/off alone does not say who a row is
    // for, what else flipping it touches, or whether Gate supplies the key.
    // A `claude-web off` with none of that is what sent a support thread
    // looking in the wrong place.
    println!(
        "{:<14} {:<6} {:<15} {:<8} {:<10} NAME",
        "DOMAIN", "STATE", "CLIENT", "SCOPE", "CREDENTIAL"
    );
    for d in domains {
        let state = if !d.supported {
            "n/a"
        } else if d.enabled {
            "on"
        } else {
            "off"
        };
        println!(
            "{:<14} {:<6} {:<15} {:<8} {:<10} {}",
            d.slug,
            state,
            d.client.slug(),
            scope_word(d.scope),
            credential_word(d.credential),
            d.display_name
        );
    }
}

/// One word per [`Scope`], for the table above.
///
/// Spelled out here rather than derived from the serde name so the CLI's
/// vocabulary is a deliberate choice: "host" is the one a reader has to
/// understand, because it is the one that reaches past the row's own name.
fn scope_word(scope: gate_connect_core::taxonomy::Scope) -> &'static str {
    use gate_connect_core::taxonomy::Scope;
    match scope {
        Scope::Host => "host",
        Scope::Client => "client",
        Scope::Machine => "machine",
    }
}

/// One word per [`Credential`]. `brokered` is also the answer to "will a
/// provider switch turn this on".
fn credential_word(credential: gate_connect_core::taxonomy::Credential) -> &'static str {
    use gate_connect_core::taxonomy::Credential;
    match credential {
        Credential::Brokered => "brokered",
        Credential::Additive => "additive",
        Credential::Observed => "observed",
    }
}

/// Platform-specific reminder shown after enabling. On Linux the proxy is
/// delivered via a user systemd `environment.d` drop-in plus a live push into
/// the running session, so relaunching a tool picks it up without a logout.
#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
fn print_proxy_hint() {
    #[cfg(target_os = "linux")]
    println!(
        "\nNote: proxy variables were written to ~/.config/environment.d/gate-proxy.conf. Relaunch command-line tools and GUI apps for them to route through Gate."
    );
}

fn resolve(slug: &str) -> Result<Box<dyn gate_connect_core::Integration>> {
    let id = ToolId::from_slug(slug)
        .with_context(|| format!("unknown tool {slug:?}; try `gate-connect list`"))?;
    registry::find(id).context("integration missing from registry")
}
