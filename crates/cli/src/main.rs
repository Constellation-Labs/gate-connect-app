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
        /// on a loopback listener. Ignores `--api-key`.
        #[arg(long)]
        oauth: bool,
        /// With `--oauth`, preselect this organization (its UUID or slug)
        /// instead of prompting. Auto-selected when you belong to only one.
        #[arg(long)]
        org: Option<String>,
    },
    /// Sign out. Removes the stored base URL and the keychain entry.
    Logout,
    /// Show the currently signed-in gateway URL, if any.
    Whoami,
    /// List supported tools and their current state.
    List,
    /// Show detailed status for one tool.
    Status {
        /// Tool slug, e.g. `codex`.
        tool: String,
    },
    /// Point a tool at the Gate AI gateway. Requires an upstream
    /// credential - set one via `set-upstream` first.
    Connect {
        tool: String,
        /// Override the integration's default upstream URL. Sent via
        /// X-Gate-Upstream-Url.
        #[arg(long, env = "GATE_UPSTREAM_URL")]
        upstream_url: Option<String>,
    },
    /// Revert a tool back to its prior configuration.
    Disconnect { tool: String },
    /// Save the upstream provider credential for a tool: paste an API key.
    SetUpstream {
        tool: String,
        /// Paste the upstream provider API key.
        #[arg(long)]
        api_key: Option<String>,
        /// Read the upstream API key from this file (first line) instead
        /// of passing it on the command line or typing it at the prompt.
        #[arg(long)]
        api_key_file: Option<std::path::PathBuf>,
    },
    /// Forget the saved upstream credential for a tool.
    ClearUpstream { tool: String },
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
    Enable {
        /// Stay in the foreground hosting the engine; Ctrl-C (or SIGTERM)
        /// stops it and restores the prior system-proxy state.
        ///
        /// The engine runs inside this process, so without this the command
        /// returns and routing goes with it - fine while the menubar app is
        /// running, since it hosts its own, but it is why a machine with no
        /// app cannot route through the engine from the CLI. Use this to host
        /// it from launchd, systemd, or a CI job.
        #[arg(long)]
        foreground: bool,
    },
    /// Turn the proxy off and restore the prior system-proxy state.
    Disable,
    /// Host ONLY the loopback reverse-proxy relay; blocks until killed.
    ///
    /// For environments with no menubar app (containers, servers, CI): CLI
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
    TrustCa,
    /// Remove the local proxy CA's trust. Requires the proxy to be off.
    UntrustCa,
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
    match cli.command {
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
        Command::Connect { tool, upstream_url } => cmd_connect(&tool, upstream_url),
        Command::Disconnect { tool } => cmd_disconnect(&tool),
        Command::SetUpstream {
            tool,
            api_key,
            api_key_file,
        } => cmd_set_upstream(&tool, api_key, api_key_file),
        Command::ClearUpstream { tool } => cmd_clear_upstream(&tool),
        #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
        Command::Proxy { command } => cmd_proxy(command),
    }
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
    // menubar app) - this process can't push the new key into it.
    #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
    {
        if proxy::engine_likely_running() {
            println!(
                "note: the Gate proxy appears to be enabled (likely in the menubar app); it keeps using the previous key until it is toggled off and on."
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
    let cfg = oauth::OAuthConfig::from_build_env().context(
        "OAuth is not configured in this build (GATE_COGNITO_HOSTED_DOMAIN / GATE_COGNITO_CLIENT_ID unset)",
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
    // menubar app) - this process can't stop it or revoke its in-memory key.
    #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
    {
        if proxy::engine_likely_running() {
            println!(
                "note: the Gate proxy appears to be enabled (likely in the menubar app); it keeps using the deleted key until it is turned off there."
            );
        }
    }
    println!("Signed out.");
    Ok(())
}

fn cmd_whoami() -> Result<()> {
    match account::load_base_url()? {
        Some(url) => println!("Signed in: {url}"),
        None => println!("Not signed in. Run `gate-connect login --base-url … --api-key …`."),
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

fn cmd_connect(tool: &str, upstream_url: Option<String>) -> Result<()> {
    let acct = account::load()?
        .context("Not signed in. Run `gate-connect login --base-url … --api-key …` first.")?;
    let integ = resolve(tool)?;
    if integ.requires_upstream_credential() && !integ.has_upstream_credential()? {
        anyhow::bail!(
            "No upstream credential saved for {}. Run `gate-connect set-upstream {} --api-key …` or `--claude-oauth` first.",
            integ.display_name(),
            tool,
        );
    }
    let upstream_url = upstream_url.unwrap_or_else(|| integ.default_upstream_url().to_string());
    let input = ConnectInput {
        gateway_base_url: acct.gateway_base_url,
        upstream_url,
        relay_base_url: gate_connect_core::proxy::relay_base_url(),
        engine_proxy_url: gate_connect_core::proxy::engine_proxy_url(),
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
                "  2. Re-run `claude` - it picks up ANTHROPIC_BASE_URL and ANTHROPIC_CUSTOM_HEADERS from ~/.claude/settings.json."
            );
            println!("  3. Verify with `claude /status` (look for the gateway URL).");
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
                "  2. Re-run `hermes` - it reads ~/.hermes/.env on launch and sends its traffic through Gate's local proxy. Your config.yaml is not touched."
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

fn cmd_set_upstream(
    tool: &str,
    api_key: Option<String>,
    api_key_file: Option<std::path::PathBuf>,
) -> Result<()> {
    let integ = resolve(tool)?;
    if !integ.requires_upstream_credential() {
        anyhow::bail!(
            "{} brings its own upstream credentials - no separate key needed",
            integ.display_name()
        );
    }
    let credential = resolve_secret(api_key, api_key_file, "upstream API key")?;
    integ.save_upstream_credential(&credential)?;
    println!("Saved upstream credential for {}.", integ.display_name());
    println!("Next: `gate-connect connect {tool}`.");
    Ok(())
}

fn cmd_clear_upstream(tool: &str) -> Result<()> {
    let integ = resolve(tool)?;
    integ.clear_upstream_credential()?;
    println!("Cleared upstream credential for {}.", integ.display_name());
    Ok(())
}

#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
fn cmd_proxy(command: ProxyCmd) -> Result<()> {
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
        ProxyCmd::Enable { foreground } => {
            // Restore providers a prior master-off disabled, before enabling -
            // otherwise the all-off state trips `enable`'s "at least one
            // provider" precondition (mirrors the app's proxy_enable flow).
            if let Err(e) = gate_connect_core::provider::restore_all() {
                eprintln!("note: restoring providers failed: {e}");
            }
            let state = mgr.enable()?;
            // Second restore pass: domain-only providers have nothing to
            // configure until the proxy is running, so the pre-enable pass
            // leaves them in the snapshot.
            if let Err(e) = gate_connect_core::provider::restore_all() {
                eprintln!("note: restoring providers after enable failed: {e}");
            }
            println!("Proxy enabled.");
            print_proxy_state(&state);
            print_proxy_hint();
            if foreground {
                println!();
                println!("Hosting the proxy engine. Press Ctrl-C to stop routing and restore");
                println!("the previous system-proxy settings.");
                proxy::wait_for_shutdown()?;
                // Restoring here is the point of blocking: a service manager
                // sends SIGTERM, and an engine that vanished without reverting
                // would leave the machine pointed at a dead loopback port.
                println!();
                mgr.disable()?;
                println!("Proxy disabled; prior system-proxy state restored.");
            }
        }
        ProxyCmd::Disable => {
            mgr.disable()?;
            println!("Proxy disabled; prior system-proxy state restored.");
        }
        ProxyCmd::Relay => {
            // Blocks until killed; hosts only the relay (no CA, no system proxy).
            proxy::serve_relay()?;
        }
        ProxyCmd::Domains => print_proxy_domains(&mgr.list_domains()?),
        ProxyCmd::Domain { slug, state } => {
            let enabled = matches!(state, Toggle::On);
            let st = mgr.set_domain(&slug, enabled)?;
            println!("{} {slug}.", if enabled { "Enabled" } else { "Disabled" });
            print_proxy_domains(&st.domains);
        }
        ProxyCmd::TrustCa => {
            mgr.trust_ca()?;
            println!("Proxy CA trusted.");
        }
        ProxyCmd::UntrustCa => {
            mgr.untrust_ca()?;
            println!("Proxy CA trust removed.");
        }
    }
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
}

#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
fn print_proxy_domains(domains: &[proxy::ProxyDomain]) {
    println!("{:<12} {:<6} NAME", "PROVIDER", "STATE");
    for d in domains {
        let state = if !d.supported {
            "n/a"
        } else if d.enabled {
            "on"
        } else {
            "off"
        };
        println!("{:<12} {:<6} {}", d.slug, state, d.display_name);
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
