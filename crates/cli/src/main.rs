//! `gate-connect` — prototype CLI for Gate Connect.
//!
//! Per the PRD this is one of three coordinated surfaces (desktop app,
//! web recipe pages, CLI). The Tauri desktop app and this CLI share the
//! same `gate-connect-core` crate, so anything testable here is testable
//! through the eventual GUI too.

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use gate_connect_core::{account, registry, ConnectInput, Status, ToolId};

// `claude_session_delegate` and `migrate` are macOS-only (they back Cowork-specific
// flows: Claude Code-session delegation and standard-mode -> 3P-mode
// userData migration). On other targets those subcommands are gated off
// entirely.
#[cfg(target_os = "macos")]
use clap::ArgGroup;
#[cfg(target_os = "macos")]
use gate_connect_core::{claude_session_delegate, migrate};

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
    /// Sign in to Gate AI. Stores the base URL on disk and the API key
    /// in the OS secret store (Keychain / Credential Manager / Secret
    /// Service). Re-run to update.
    Login {
        #[arg(long, env = "GATE_BASE_URL")]
        base_url: String,
        #[arg(long)]
        api_key: Option<String>,
        /// Read the Gate API key from this file (first line) instead of
        /// passing it on the command line or typing it at the prompt.
        #[arg(long)]
        api_key_file: Option<std::path::PathBuf>,
    },
    /// Sign out. Removes the stored base URL and the keychain entry.
    Logout,
    /// Show the currently signed-in gateway URL, if any.
    Whoami,
    /// List supported tools and their current state.
    List,
    /// Show detailed status for one tool.
    Status {
        /// Tool slug, e.g. `cowork`.
        tool: String,
    },
    /// Point a tool at the Gate AI gateway. Requires an upstream
    /// credential — set one via `set-upstream` first.
    Connect {
        tool: String,
        /// Override the integration's default upstream URL. Sent via
        /// X-Gate-Upstream-Url.
        #[arg(long, env = "GATE_UPSTREAM_URL")]
        upstream_url: Option<String>,
    },
    /// Revert a tool back to its prior configuration.
    Disconnect { tool: String },
    /// Save the upstream provider credential for a tool: either paste
    /// an Anthropic API key, or (macOS only) reuse the active Claude
    /// Code session via OAuth delegation.
    #[cfg_attr(
        target_os = "macos",
        command(group(ArgGroup::new("source").required(true).args(["api_key", "api_key_file", "claude_oauth"])))
    )]
    SetUpstream {
        tool: String,
        /// Paste an Anthropic API key (`sk-ant-api03-…`).
        #[arg(long)]
        api_key: Option<String>,
        /// Read the upstream API key from this file (first line) instead
        /// of passing it on the command line or typing it at the prompt.
        #[arg(long)]
        api_key_file: Option<std::path::PathBuf>,
        /// Delegate to the active Claude Code session (macOS only —
        /// reads from the Claude Code keychain entry on every Cowork
        /// request).
        #[cfg(target_os = "macos")]
        #[arg(long)]
        claude_oauth: bool,
    },
    /// Forget the saved upstream credential for a tool.
    ClearUpstream { tool: String },
    /// Manage the built-in MITM proxy that routes config-less apps
    /// (Claude Desktop, ChatGPT, …) and command-line tools through the Gate
    /// gateway. Enabling installs a local CA and points the system proxy at a
    /// loopback listener; only enabled provider domains are intercepted —
    /// every other host is tunnelled untouched.
    #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
    Proxy {
        #[command(subcommand)]
        command: ProxyCmd,
    },
    /// Copy standard-mode Cowork data (scheduled tasks, conversations,
    /// org plugins, memory, prefs) into the 3P/gateway-mode userData dir.
    /// Only Cowork is supported today; the `tool` argument keeps the
    /// surface ready for future integrations.
    ///
    /// macOS-only: backs the Cowork integration, which is itself macOS-only.
    #[cfg(target_os = "macos")]
    Migrate {
        tool: String,
        #[command(subcommand)]
        command: MigrateCmd,
    },
}

#[cfg(target_os = "macos")]
#[derive(Subcommand)]
enum MigrateCmd {
    /// Scan source and destination and print what's available.
    Discover,
    /// Show what would happen without writing anything.
    Preview {
        /// Comma-separated category list. Defaults to all categories.
        #[arg(long, value_delimiter = ',')]
        include: Option<Vec<String>>,
    },
    /// Run the migration.
    Execute {
        /// Comma-separated category list. Defaults to all categories.
        #[arg(long, value_delimiter = ',')]
        include: Option<Vec<String>>,
        /// Print the plan but do not touch any files.
        #[arg(long)]
        dry_run: bool,
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
    Enable,
    /// Turn the proxy off and restore the prior system-proxy state.
    Disable,
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
    let cli = Cli::parse();
    match cli.command {
        Command::Login {
            base_url,
            api_key,
            api_key_file,
        } => cmd_login(base_url, api_key, api_key_file),
        Command::Logout => cmd_logout(),
        Command::Whoami => cmd_whoami(),
        Command::List => cmd_list(),
        Command::Status { tool } => cmd_status(&tool),
        Command::Connect { tool, upstream_url } => cmd_connect(&tool, upstream_url),
        Command::Disconnect { tool } => cmd_disconnect(&tool),
        #[cfg(target_os = "macos")]
        Command::SetUpstream {
            tool,
            api_key,
            api_key_file,
            claude_oauth,
        } => {
            let api_key = if claude_oauth {
                None
            } else {
                Some(resolve_secret(api_key, api_key_file, "upstream API key")?)
            };
            cmd_set_upstream(&tool, api_key, claude_oauth)
        }
        #[cfg(not(target_os = "macos"))]
        Command::SetUpstream {
            tool,
            api_key,
            api_key_file,
        } => cmd_set_upstream(&tool, api_key, api_key_file),
        Command::ClearUpstream { tool } => cmd_clear_upstream(&tool),
        #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
        Command::Proxy { command } => cmd_proxy(command),
        #[cfg(target_os = "macos")]
        Command::Migrate { tool, command } => cmd_migrate(&tool, command),
    }
}

fn cmd_login(
    base_url: String,
    api_key: Option<String>,
    api_key_file: Option<std::path::PathBuf>,
) -> Result<()> {
    let api_key = resolve_secret(api_key, api_key_file, "Gate API key")?;
    account::save(&base_url, Some(&api_key))?;
    // The key is copied into tool configs at connect time — push the new
    // one into any config that still embeds an old key.
    registry::refresh_gate_key_everywhere(&api_key)?;
    // The proxy engine lives in whichever process enabled it (usually the
    // menubar app) — this process can't push the new key into it.
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
    // menubar app) — this process can't stop it or revoke its in-memory key.
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
            #[cfg(any(target_os = "macos", target_os = "windows"))]
            ToolId::Cowork => {
                println!("Fully quit and relaunch Claude Desktop for changes to take effect.")
            }
            ToolId::ClaudeCode => {
                println!("Re-run `claude` to pick up the new settings.json env block.")
            }
            ToolId::Codex => {
                println!("Re-run `codex` to pick up the new config.toml provider block.")
            }
            ToolId::OpenCode => {
                println!("Re-run `opencode` to pick up the new opencode.json provider block.")
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
    };
    integ.connect(&input)?;
    println!("Connected {}.", integ.display_name());
    println!();
    println!("Next steps:");
    match integ.id() {
        #[cfg(any(target_os = "macos", target_os = "windows"))]
        ToolId::Cowork => {
            println!("  1. Fully quit Claude Desktop (do not just close the window).");
            println!("  2. Relaunch Claude Desktop.");
            println!(
                "  3. On the sign-in screen, choose \"skip Anthropic authentication\" — Cowork should now use your gateway."
            );
            println!(
                "  4. Verify with: Help → Troubleshooting → Copy Managed Configuration Report"
            );
        }
        ToolId::ClaudeCode => {
            println!(
                "  1. Quit any running `claude` sessions (they cache settings.json at launch)."
            );
            println!(
                "  2. Re-run `claude` — it picks up ANTHROPIC_BASE_URL and ANTHROPIC_CUSTOM_HEADERS from ~/.claude/settings.json."
            );
            println!("  3. Verify with `claude /status` (look for the gateway URL).");
        }
        ToolId::Codex => {
            println!("  1. Quit any running `codex` sessions.");
            println!(
                "  2. Re-run `codex` — it reads ~/.codex/config.toml on launch and routes through the `gate` model provider. Gate handles upstream auth, no OPENAI_API_KEY needed."
            );
        }
        ToolId::OpenCode => {
            println!("  1. Quit any running `opencode` sessions.");
            println!(
                    "  2. Re-run `opencode` — your existing providers (anthropic / openai / openrouter) now route through Gate. Use the same model names you always have."
                );
            println!(
                    "  3. Your API keys from `opencode auth login <provider>` are untouched. Gate adds its headers and forwards each request to the original upstream."
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
        #[cfg(any(target_os = "macos", target_os = "windows"))]
        ToolId::Cowork => println!("Restart Claude Desktop for the change to take effect."),
        ToolId::ClaudeCode => {
            println!("Restart any running `claude` sessions for the change to take effect.")
        }
        ToolId::Codex => {
            println!("Restart any running `codex` sessions for the change to take effect.")
        }
        ToolId::OpenCode => {
            println!("Restart any running `opencode` sessions for the change to take effect.")
        }
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn cmd_set_upstream(tool: &str, api_key: Option<String>, claude_oauth: bool) -> Result<()> {
    let integ = resolve(tool)?;
    if !integ.requires_upstream_credential() {
        anyhow::bail!(
            "{} brings its own upstream credentials — no separate key needed",
            integ.display_name()
        );
    }
    let credential: String = if claude_oauth {
        println!("Verifying Claude Code session and wiring delegation…");
        claude_session_delegate::verify_claude_code_session()?.to_string()
    } else {
        api_key.context("internal: no credential source provided")?
    };
    integ.save_upstream_credential(&credential)?;
    println!("Saved upstream credential for {}.", integ.display_name());
    println!("Next: `gate-connect connect {tool}`.");
    Ok(())
}

#[cfg(not(target_os = "macos"))]
fn cmd_set_upstream(
    tool: &str,
    api_key: Option<String>,
    api_key_file: Option<std::path::PathBuf>,
) -> Result<()> {
    let integ = resolve(tool)?;
    if !integ.requires_upstream_credential() {
        anyhow::bail!(
            "{} brings its own upstream credentials — no separate key needed",
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
    match command {
        ProxyCmd::Status => print_proxy_state(&mgr.status()?),
        ProxyCmd::Enable => {
            let state = mgr.enable()?;
            println!("Proxy enabled.");
            print_proxy_state(&state);
            print_proxy_hint();
        }
        ProxyCmd::Disable => {
            mgr.disable()?;
            println!("Proxy disabled; prior system-proxy state restored.");
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
/// delivered via `/etc/environment`, which only new sessions read.
#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
fn print_proxy_hint() {
    #[cfg(target_os = "linux")]
    println!(
        "\nNote: proxy variables were written to /etc/environment — open a new shell (or re-login) for command-line tools to pick them up."
    );
}

fn resolve(slug: &str) -> Result<Box<dyn gate_connect_core::Integration>> {
    let id = ToolId::from_slug(slug)
        .with_context(|| format!("unknown tool {slug:?}; try `gate-connect list`"))?;
    registry::find(id).context("integration missing from registry")
}

#[cfg(target_os = "macos")]
fn cmd_migrate(tool: &str, command: MigrateCmd) -> Result<()> {
    if tool != "cowork" {
        anyhow::bail!("migrate is only supported for `cowork` today");
    }
    match command {
        MigrateCmd::Discover => {
            let info = migrate::discover()?;
            print_discover(&info);
        }
        MigrateCmd::Preview { include } => {
            let mut opts = migrate::MigrateOptions::all();
            opts.dry_run = true;
            if let Some(cats) = include {
                apply_include(&mut opts, &cats)?;
            }
            let report = migrate::execute(&opts)?;
            print_report(&report);
        }
        MigrateCmd::Execute { include, dry_run } => {
            let mut opts = migrate::MigrateOptions::all();
            opts.dry_run = dry_run;
            if let Some(cats) = include {
                apply_include(&mut opts, &cats)?;
            }
            let report = migrate::execute(&opts)?;
            print_report(&report);
        }
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn apply_include(opts: &mut migrate::MigrateOptions, cats: &[String]) -> Result<()> {
    // When --include is supplied, start from "all off" and turn on the
    // requested categories. Unknown names error out so typos surface.
    opts.include_plugins = false;
    opts.include_scheduled = false;
    opts.include_conversations = false;
    opts.include_memory = false;
    opts.include_enabled_plugins = false;
    opts.include_preferences = false;
    opts.include_artifacts = false;
    for c in cats {
        match c.as_str() {
            "plugins" => opts.include_plugins = true,
            "scheduled" => opts.include_scheduled = true,
            "conversations" => opts.include_conversations = true,
            "memory" => opts.include_memory = true,
            "enabled_plugins" => opts.include_enabled_plugins = true,
            "preferences" => opts.include_preferences = true,
            "artifacts" => opts.include_artifacts = true,
            other => anyhow::bail!(
                "unknown migrate category {other:?}; valid: plugins, scheduled, conversations, memory, enabled_plugins, preferences, artifacts"
            ),
        }
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn print_discover(info: &migrate::MigrateDiscover) {
    if let Some(r) = &info.roots {
        println!("Source: {}", r.source_dir.display());
        println!("Dest:   {}", r.dest_dir.display());
    } else {
        println!("Source/dest not both discoverable.");
    }
    println!();
    println!("Available:");
    println!("  plugins:         {}", info.plugins);
    println!("  scheduled tasks: {}", info.scheduled);
    println!("  conversations:   {}", info.conversations);
    println!("  memory:          {}", info.has_memory);
    println!("  enabled plugins: {}", info.has_enabled_plugins);
    println!("  preferences:     {}", info.has_preferences);
    println!("  artifacts:       {}", info.artifacts);
    println!("  estimated size:  {} bytes", info.bytes_estimated);
    println!();
    println!("Ready: {}", info.ready);
    if !info.blockers.is_empty() {
        println!("Blockers:");
        for b in &info.blockers {
            println!("  - {b:?}");
        }
    }
}

#[cfg(target_os = "macos")]
fn print_report(report: &migrate::MigrateReport) {
    println!("dry_run: {}", report.dry_run);
    let mut keys: Vec<&String> = report.per_category.keys().collect();
    keys.sort();
    for k in keys {
        let r = &report.per_category[k];
        println!(
            "  {:<18} copied={:>4}  skipped={:>4}  failed={:>4}",
            k, r.copied, r.skipped, r.failed
        );
        for err in &r.errors {
            println!("    ! {err}");
        }
    }
}
