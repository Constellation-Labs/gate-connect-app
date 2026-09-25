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
    /// terminal, or SIGTERM on macOS) puts tools whose config names this
    /// process's relay or engine back on their own settings, then stops
    /// routing and restores the prior system-proxy state. Returning instead
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
    // desktop app) - this process can't push the new key into it.
    #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
    {
        if proxy::engine_likely_running() {
            println!(
                "note: the Gate proxy appears to be enabled in another process (likely the Gate Connect app); it keeps using the previous key until routing is restarted there."
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

fn cmd_connect(tool: &str) -> Result<()> {
    let acct = account::load()?
        .context("Not signed in. Run `gate-connect login --base-url … --api-key …` first.")?;
    let integ = resolve(tool)?;
    let input = ConnectInput {
        gateway_base_url: acct.gateway_base_url,
        relay_base_url: gate_connect_core::proxy::relay_base_url(),
        // `tool_proxy_url`, not the engine's own address: a config written here
        // has to name what the GUI writes, or the two disagree about the same
        // install and a plain quit reverts whatever this wrote.
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
                    "Hosting the proxy engine. Press Ctrl-C to stop: tools pointed at this \
                     process go back on their own settings, and the previous system-proxy \
                     settings are restored."
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
                // goes with us. So first put those tools back on their own
                // settings, as the app's quit does (`quit_app`); the next
                // enable or app launch reconnects them. Linux skips it, as the
                // app does: the daemon outlives us and keeps answering.
                #[cfg(not(target_os = "linux"))]
                match gate_connect_core::provider::revert_stranded_configs_for_quit() {
                    Ok(names) if names.len() == 1 => println!(
                        "Put {} back on its own settings; it reconnects the next time routing is enabled.",
                        names[0]
                    ),
                    Ok(names) if !names.is_empty() => println!(
                        "Put {} back on their own settings; they reconnect the next time routing is enabled.",
                        names.join(", ")
                    ),
                    Ok(_) => {}
                    Err(e) => eprintln!("note: putting tools back on their own settings failed: {e:#}"),
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
                mgr.trust_ca_system()?;
                println!("Proxy CA trusted machine-wide.");
                println!("Remove it with `gate-connect proxy untrust-ca --system-trust`.");
            } else {
                mgr.trust_ca()?;
                println!("Proxy CA trusted.");
            }
        }
        ProxyCmd::UntrustCa { system_trust } => {
            // Untrusting stops routing rather than refusing while it is on, so
            // say so: nothing else on this path would tell the user their
            // traffic stopped going through Gate.
            let was_routing = gate_connect_core::proxy::engine_likely_running();
            if system_trust {
                mgr.untrust_ca_system()?;
                println!("Machine-wide proxy CA trust removed.");
            } else {
                mgr.untrust_ca()?;
                println!("Proxy CA trust removed.");
            }
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
