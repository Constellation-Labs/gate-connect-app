//! macOS Cowork implementation.
//!
//! Configures Claude Desktop's Cowork feature to route inference through
//! Constellation Gate (Cowork 3P, `inferenceProvider: gateway`).
//!
//! Mechanism: write a managed-preferences plist at
//!   /Library/Managed Preferences/<user>/com.anthropic.claudefordesktop.plist
//! plus a small helper script Cowork invokes at request time. The
//! helper reads two secrets from the macOS Keychain and emits JSON:
//!
//! - The **upstream provider credential** (per-tool — either a raw
//!   Anthropic API key `sk-ant-api03-…` or a Claude OAuth token
//!   `sk-ant-oat01-…`) is emitted as `token` AND as `X-Api-Key`.
//!   Cowork sends `Authorization: Bearer <token>` plus that header,
//!   mirroring Claude Code `apiKeyHelper`'s dual-emit. Gate forwards it
//!   to Anthropic.
//! - The **Gate API key** (account-level) is emitted as `X-Gate-Api-Key`
//!   so Gate's auth-guard can identify the workspace.
//!
//! Cowork reads the managed plist exactly once at launch — the user
//! must fully quit and relaunch Claude Desktop for changes to take
//! effect.

use anyhow::{Context, Result};
use plist::{Dictionary, Value};
use std::fs;
use std::path::{Path, PathBuf};

use crate::env;
use crate::keychain;
use crate::primitives;
use crate::registry::{ConnectInput, Status};

use super::upstream_service;

const HELPER_FILENAME: &str = "cowork-credential-helper.sh";
const COWORK_APP_PATH: &str = "/Applications/Claude.app";

/// Cached for 50 min by Cowork before re-invoking the helper.
const HELPER_TTL_SECONDS: i64 = 3000;

fn helper_path() -> Result<PathBuf> {
    Ok(env::app_support_dir()?.join(HELPER_FILENAME))
}

pub fn detect() -> Result<bool> {
    Ok(Path::new(COWORK_APP_PATH).exists())
}

pub fn status() -> Result<Status> {
    if !detect()? {
        return Ok(Status::NotInstalled);
    }
    let plist_path = env::cowork_managed_plist_path()?;
    if !plist_path.exists() {
        return Ok(Status::Detected);
    }
    let value = match plist::Value::from_file(&plist_path) {
        Ok(v) => v,
        Err(_) => {
            return Ok(Status::Drifted(format!(
                "managed plist at {} is unreadable",
                plist_path.display()
            )));
        }
    };
    let dict = value
        .as_dictionary()
        .context("managed plist top level is not a dictionary")?;
    let provider = dict
        .get("inferenceProvider")
        .and_then(|v| v.as_string())
        .unwrap_or("");
    if provider != "gateway" {
        return Ok(Status::Drifted(format!(
            "inferenceProvider is {:?}, expected \"gateway\"",
            provider
        )));
    }
    let helper = helper_path()?;
    if !helper.exists() {
        return Ok(Status::Drifted(format!(
            "credential helper missing at {}",
            helper.display()
        )));
    }
    let user = env::current_user()?;
    if keychain::get(&crate::account::service(), &user)?.is_none() {
        return Ok(Status::Drifted(
            "keychain entry for gate-api-key missing".into(),
        ));
    }
    if keychain::get(&upstream_service(), &user)?.is_none() {
        return Ok(Status::Drifted(
            "keychain entry for upstream credential missing".into(),
        ));
    }
    Ok(Status::Connected)
}

pub fn connect(input: &ConnectInput) -> Result<()> {
    if !detect()? {
        anyhow::bail!(
      "Claude Desktop is not installed at {COWORK_APP_PATH}; install from claude.com/download first"
    );
    }
    if !input.gateway_base_url.starts_with("https://") {
        anyhow::bail!("gateway base URL must be https://");
    }
    if !input.upstream_url.starts_with("https://") {
        anyhow::bail!("upstream URL must be https://");
    }
    let user = env::current_user()?;
    if keychain::get(&upstream_service(), &user)?.is_none() {
        anyhow::bail!(
      "no upstream Anthropic credential saved — paste an API key or run `claude setup-token` first"
    );
    }

    let helper = helper_path()?;
    let helper_script = render_helper_script(
        &crate::account::service(),
        &upstream_service(),
        &user,
        &input.upstream_url,
    );
    // Mode 0o700: helper script itself doesn't hold secrets, but it
    // runs `security find-generic-password` to fetch keychain-stored
    // credentials and prints them to stdout. Locking it down to
    // user-only stops other local users from reading the script
    // body and learning which keychain entries get queried.
    primitives::write_file(&helper, helper_script.as_bytes(), 0o700)
        .context("writing credential helper")?;

    let plist_bytes =
        primitives::plist_bytes(&build_managed_plist(&input.gateway_base_url, &helper)?)?;
    let plist_path = env::cowork_managed_plist_path()?;
    primitives::write_file_privileged(&plist_path, &plist_bytes, 0o644)
        .context("writing managed-preferences plist (requires sudo)")?;

    Ok(())
}

pub fn disconnect() -> Result<()> {
    // Only delete a plist we wrote: an MDM-pushed managed-preferences file
    // lives at the same path, and removing it with admin rights would
    // destroy configuration we don't own. Ours is identifiable by the
    // credential helper pointing at our per-user helper script.
    let plist_path = env::cowork_managed_plist_path()?;
    if plist_path.exists() {
        if plist_is_ours(&plist_path)? {
            primitives::remove_file_privileged(&plist_path)
                .context("removing managed-preferences plist (requires sudo)")?;
        } else {
            eprintln!(
                "gate-connect: leaving {} in place — it was not written by Gate Connect",
                plist_path.display()
            );
        }
    }

    let helper = helper_path()?;
    if helper.exists() {
        fs::remove_file(&helper).with_context(|| format!("removing {}", helper.display()))?;
    }
    // Keychain entries (Gate key + upstream credential) stay — use
    // Sign out or explicit clear_upstream_credential to wipe them.
    Ok(())
}

/// Does the managed plist at `plist_path` belong to Gate Connect? True
/// iff its `inferenceCredentialHelper` points at our helper script — an
/// MDM-pushed plist would reference its own tooling (or nothing). An
/// unreadable plist is treated as not ours: we can't prove ownership, so
/// we must not delete it.
fn plist_is_ours(plist_path: &Path) -> Result<bool> {
    let Ok(value) = plist::Value::from_file(plist_path) else {
        return Ok(false);
    };
    let helper = helper_path()?.display().to_string();
    Ok(value
        .as_dictionary()
        .and_then(|d| d.get("inferenceCredentialHelper"))
        .and_then(|v| v.as_string())
        .map(|s| s == helper)
        .unwrap_or(false))
}

/// Render the credential helper.
///
/// Two modes, distinguished by the value stashed in our per-tool
/// keychain entry:
///
/// - **Sentinel `@claude-code-session`** → live-read Claude Code's
///   own keychain entry on every invocation, parse out
///   `claudeAiOauth.accessToken`, and emit it as an OAuth bearer.
///   Refresh is handled entirely by Claude Code itself — we
///   trail whatever's current.
///
/// - **Anything else** → treat the stored value as a literal credential.
///   `sk-ant-oat*` → OAuth bearer + `anthropic-beta: oauth-2025-04-20`.
///   Anything else (API key) → dual-emit as Bearer + `X-Api-Key`,
///   matching Claude Code's `apiKeyHelper` convention.
///
/// Mixing token types confuses Anthropic — OAuth bearers sent with
/// `X-Api-Key` get validated against the api-key path and rejected
/// ("Invalid bearer token").
fn render_helper_script(
    gate_service: &str,
    upstream_service: &str,
    account: &str,
    upstream_url: &str,
) -> String {
    format!(
    "#!/bin/bash\n\
     # Written by Gate Connect. Do not edit by hand.\n\
     set -eu\n\
     GATE_KEY=\"$(/usr/bin/security find-generic-password -s {gate} -a {account} -w)\"\n\
     STORED=\"$(/usr/bin/security find-generic-password -s {upstream} -a {account} -w)\"\n\
     # If the stored value is our delegation sentinel, live-read the\n\
     # Claude Code session token. Otherwise treat as a literal credential.\n\
     if [ \"$STORED\" = '{sentinel}' ]; then\n\
       CC_JSON=\"$(/usr/bin/security find-generic-password -s 'Claude Code-credentials' -a {account} -w)\"\n\
       UPSTREAM=\"$(printf '%s' \"$CC_JSON\" | /usr/bin/sed -n 's/.*\"claudeAiOauth\"[^{{]*{{[^{{}}]*\"accessToken\":\"\\([^\"]*\\)\".*/\\1/p')\"\n\
       if [ -z \"$UPSTREAM\" ]; then\n\
         echo 'Gate Connect: could not extract accessToken from Claude Code-credentials' >&2\n\
         exit 1\n\
       fi\n\
     else\n\
       UPSTREAM=\"$STORED\"\n\
     fi\n\
     UPSTREAM_URL={upstream_url}\n\
     esc() {{ printf '%s' \"$1\" | /usr/bin/sed -e 's/\\\\/\\\\\\\\/g' -e 's/\"/\\\\\"/g'; }}\n\
     E_GATE=\"$(esc \"$GATE_KEY\")\"\n\
     E_UP=\"$(esc \"$UPSTREAM\")\"\n\
     E_UU=\"$(esc \"$UPSTREAM_URL\")\"\n\
     case \"$UPSTREAM\" in\n\
       sk-ant-oat*)\n\
         # OAuth token: Authorization Bearer + anthropic-beta only.\n\
         printf '{{\"token\":\"%s\",\"headers\":{{\"X-Gate-Api-Key\":\"%s\",\"X-Gate-Upstream-Url\":\"%s\",\"anthropic-beta\":\"oauth-2025-04-20\"}}}}' \\\n\
           \"$E_UP\" \"$E_GATE\" \"$E_UU\"\n\
         ;;\n\
       *)\n\
         # API key: dual-emit as Authorization Bearer + x-api-key.\n\
         printf '{{\"token\":\"%s\",\"headers\":{{\"X-Api-Key\":\"%s\",\"X-Gate-Api-Key\":\"%s\",\"X-Gate-Upstream-Url\":\"%s\"}}}}' \\\n\
           \"$E_UP\" \"$E_UP\" \"$E_GATE\" \"$E_UU\"\n\
         ;;\n\
     esac\n",
    gate = shell_escape(gate_service),
    upstream = shell_escape(upstream_service),
    account = shell_escape(account),
    sentinel = crate::claude_session_delegate::CLAUDE_CODE_SENTINEL,
    // shell_escape (not raw interpolation): a single quote in the URL must
    // not break out of the assignment — this value reaches us from IPC/CLI.
    upstream_url = shell_escape(upstream_url),
  )
}

fn shell_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('\'');
    for c in s.chars() {
        if c == '\'' {
            out.push_str("'\\''");
        } else {
            out.push(c);
        }
    }
    out.push('\'');
    out
}

fn build_managed_plist(base_url: &str, helper_path: &Path) -> Result<Value> {
    let mut dict = Dictionary::new();
    dict.insert("inferenceProvider".into(), Value::String("gateway".into()));
    dict.insert(
        "inferenceGatewayBaseUrl".into(),
        Value::String(base_url.to_string()),
    );
    dict.insert(
        "inferenceGatewayAuthScheme".into(),
        Value::String("bearer".into()),
    );
    dict.insert(
        "inferenceCredentialHelper".into(),
        Value::String(helper_path.display().to_string()),
    );
    dict.insert(
        "inferenceCredentialHelperTtlSec".into(),
        Value::Integer(HELPER_TTL_SECONDS.into()),
    );
    dict.insert(
        "deploymentOrganizationUuid".into(),
        Value::String(primitives::install_id()?),
    );

    // We deliberately do NOT write `inferenceGatewayHeaders` here.
    // Cowork's pre-load plist reader registers that key as a boolean
    // and rejects any map written there. Routing headers come from the
    // credential helper instead — Cowork merges them onto every
    // inference and /v1/models request.
    Ok(Value::Dictionary(dict))
}
