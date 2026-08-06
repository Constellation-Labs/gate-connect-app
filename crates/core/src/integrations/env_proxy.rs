//! The environment channel, as something the user can decline.
//!
//! Not a tool. This entry represents the *mechanism* by which command-line
//! tools reach Gate: the proxy variables the system proxy exports into the
//! user's environment.
//!
//! ```text
//! HTTPS_PROXY / HTTP_PROXY (+ lower-case)  -> http://127.0.0.1:<engine-port>
//! NO_PROXY                                 -> localhost,127.0.0.1,::1
//! NODE_EXTRA_CA_CERTS                      -> <app-support>/proxy/ca-cert.pem
//! ```
//!
//! Why it is its own entry rather than per-tool code. Some tools cannot be
//! configured at all: OpenCode has no proxy or CA setting anywhere in its
//! config schema and loads no dotenv, so these variables are the *only* way to
//! route it. Writing an OpenCode-shaped integration for that would be a
//! fiction - nothing tool-specific happens. The same export simultaneously
//! covers anything else that reads `HTTPS_PROXY`, which is most of the
//! Node/Bun/Python ecosystem. One mechanism, many beneficiaries, so it is
//! modelled once.
//!
//! Why it is a *choice*. These variables are machine-wide: `HTTPS_PROXY`
//! redirects git, curl, npm and everything else, not just the AI tools. That is
//! a large enough change to deserve consent rather than arriving as a silent
//! side effect of the routing switch. [`crate::proxy::manager`] consults
//! [`crate::proxy::env_export_opted_in`] before exporting, so a user who
//! disconnects here does not get it back on the next enable.
//!
//! Defaults to connected, because that is what the routing switch has always
//! implied and turning it off would silently stop routing the CLI tools.
//!
//! **Linux is different, and honestly so.** There the `environment.d` drop-in
//! *is* the system proxy - there is no PAC - so the variables cannot be
//! declined without declining routing altogether. Disconnect still records the
//! choice (it must not fail: sign-out and the master-off sweep both call it),
//! but it cannot withdraw the variables, and `status` says exactly that rather
//! than claiming the tool is clean.
//!
//! Status is read back from the OS (`launchctl getenv`, the registry, the
//! drop-in), never from a record of what we last wrote - the failure this whole
//! branch exists to remove is a config we wrote reporting Connected while
//! something else decides the wire.

use anyhow::Result;

use crate::registry::{ConnectInput, Integration, Status, ToolId};

const DISPLAY_NAME: &str = "Environment proxy";
const UPSTREAM_PROVIDER_NAME: &str = "your existing providers";
const DEFAULT_UPSTREAM_URL: &str = "https://openrouter.ai/api/v1";

pub struct EnvProxy;

impl Integration for EnvProxy {
    fn id(&self) -> ToolId {
        ToolId::EnvProxy
    }

    fn display_name(&self) -> &'static str {
        DISPLAY_NAME
    }

    fn upstream_provider_name(&self) -> &'static str {
        UPSTREAM_PROVIDER_NAME
    }

    fn default_upstream_url(&self) -> &'static str {
        DEFAULT_UPSTREAM_URL
    }

    fn requires_upstream_credential(&self) -> bool {
        false
    }

    /// "Installed" means the platform can export at all. There is no binary to
    /// look for: the capability is the OS, not a tool.
    fn detect(&self) -> Result<bool> {
        Ok(supported())
    }

    fn status(&self) -> Result<Status> {
        if !supported() {
            return Ok(Status::NotInstalled);
        }
        Ok(compute_status(
            crate::proxy::env_export_opted_in(),
            crate::proxy::exported_proxy_url().as_deref(),
            crate::proxy::persisted_engine_proxy_url().as_deref(),
            crate::proxy::engine_proxy_url().is_some(),
            crate::proxy::env_export_is_separable(),
        ))
    }

    fn connect(&self, input: &ConnectInput) -> Result<()> {
        if !supported() {
            anyhow::bail!("Gate cannot export proxy environment variables on this platform");
        }
        // Same rule as the other proxy-routed integrations: handing tools a
        // proxy address with nothing behind it breaks them outright rather than
        // merely leaving them un-routed.
        input.engine_proxy_url.as_deref().context_missing()?;

        // Shared with the UI switch, so the two cannot drift on applying the
        // choice immediately rather than at the next routing toggle.
        crate::proxy::set_env_export(true)?;

        eprintln!(
            "note: only processes started from now on see these variables -- restart any tool \
             that is already running."
        );
        Ok(())
    }

    /// Record that the user does not want the export, and withdraw it where
    /// that is possible.
    ///
    /// Never fails on the inseparable platform, deliberately. This runs from
    /// [`crate::registry::disconnect_all_managed`] on sign-out and from the
    /// master-off sweep, and an error there aborts the whole operation - so
    /// refusing on Linux would make sign-out fail whenever routing was on. The
    /// choice is still recorded, and `status` then reports the truth: opted out,
    /// but the variables are still present because on Linux they *are* routing.
    fn disconnect(&self) -> Result<()> {
        if !supported() {
            return Ok(());
        }
        crate::proxy::set_env_export(false)
    }

    fn save_upstream_credential(&self, _credential: &str) -> Result<()> {
        anyhow::bail!(
            "the environment proxy needs no credential of its own -- Gate injects yours in flight and passes your provider credentials through untouched."
        )
    }

    /// Kept out of the *ledger*, which is not the same as kept out of the UI.
    ///
    /// Home groups by model family; this is a mechanism spanning every family,
    /// so it has no honest row there. It surfaces instead as a switch under the
    /// master one in the Routing card, fed by `ProxyState.env_export_opted_in`
    /// rather than by `list_tools` - which is the right shape, because it is a
    /// property of routing rather than a tool alongside Claude Code.
    fn hidden_in_ui(&self) -> bool {
        true
    }

    fn has_upstream_credential(&self) -> Result<bool> {
        Ok(true)
    }

    fn clear_upstream_credential(&self) -> Result<()> {
        Ok(())
    }
}

/// Platforms where Gate wires a system proxy at all.
fn supported() -> bool {
    cfg!(any(
        target_os = "macos",
        target_os = "windows",
        target_os = "linux"
    ))
}

/// Pure drift evaluation, so every state is testable without a live engine.
///
/// `exported` is what the OS actually reports, `expected` our proxy address
/// from the persisted port, and `running` whether the engine is up. "Exported
/// but the engine is down" is drift, not Connected: the variables are pointing
/// every CLI tool at a dead address, which is worse than not routing.
fn compute_status(
    opted_in: bool,
    exported: Option<&str>,
    expected: Option<&str>,
    running: bool,
    separable: bool,
) -> Status {
    if !opted_in {
        // The user declined. Anything still exported is worth surfacing rather
        // than reporting clean - but why it lingers differs by platform, and
        // telling a Linux user to "disconnect again" would be a dead end.
        return match exported {
            Some(v) if separable => Status::Drifted(format!(
                "the proxy environment export is turned off, but {v:?} is still in the \
                 environment -- reconnect and disconnect again to clear it"
            )),
            Some(v) => Status::Drifted(format!(
                "you turned the proxy environment export off, but on Linux those variables are \
                 how Gate routes at all, so {v:?} stays until you turn routing off"
            )),
            None => Status::Detected,
        };
    }
    let Some(exported) = exported else {
        return if running {
            Status::Drifted(
                "routing is on but no proxy variables are exported, so command-line tools are \
                 not going through Gate"
                    .into(),
            )
        } else {
            // Routing is off, so nothing is exported and nothing should be.
            Status::Detected
        };
    };
    let Some(expected) = expected else {
        return Status::Drifted(format!(
            "the environment points at {exported:?}, which is not an address Gate has bound"
        ));
    };
    if exported != expected {
        return Status::Drifted(format!(
            "the environment points at {exported:?}, not at Gate ({expected:?})"
        ));
    }
    if !running {
        return Status::Drifted(format!(
            "the Gate proxy is not running, so {exported:?} is a dead address for every tool \
             that reads it -- turn the proxy on, or disconnect this to clear the variables"
        ));
    }
    Status::Connected
}

/// Small helper so the missing-engine message is written once.
trait MissingEngine<T> {
    fn context_missing(self) -> Result<T>;
}

impl<T> MissingEngine<T> for Option<T> {
    fn context_missing(self) -> Result<T> {
        self.ok_or_else(|| {
            anyhow::anyhow!(
                "the Gate proxy is not running -- turn routing on before connecting the \
                 environment proxy, which points your tools at it"
            )
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const OURS: &str = "http://127.0.0.1:9977";

    #[test]
    fn connected_only_when_exported_matching_and_live() {
        assert_eq!(
            compute_status(true, Some(OURS), Some(OURS), true, true),
            Status::Connected
        );
    }

    #[test]
    fn exported_at_a_dead_engine_is_drift_not_connected() {
        // The dangerous state: every CLI tool is aimed at a port nothing
        // listens on, which fails closed rather than falling back to direct.
        match compute_status(true, Some(OURS), Some(OURS), false, true) {
            Status::Drifted(m) => {
                assert!(m.contains("not running"), "unexpected: {m}");
                assert!(m.contains("disconnect"), "must offer a way out: {m}");
            }
            other => panic!("expected drift, got {other:?}"),
        }
    }

    #[test]
    fn a_users_own_proxy_is_not_ours() {
        match compute_status(
            true,
            Some("http://proxy.corp.example:3128"),
            Some(OURS),
            true,
            true,
        ) {
            Status::Drifted(m) => assert!(m.contains("not at Gate"), "unexpected: {m}"),
            other => panic!("expected drift, got {other:?}"),
        }
    }

    #[test]
    fn opted_out_and_clean_is_simply_detected() {
        assert_eq!(
            compute_status(false, None, Some(OURS), true, true),
            Status::Detected
        );
    }

    #[test]
    fn opted_out_but_still_exported_is_drift() {
        // A failed withdrawal must not read as clean, or the leftover variables
        // outlive the user's decision - on Windows, across a reboot.
        match compute_status(false, Some(OURS), Some(OURS), true, true) {
            Status::Drifted(m) => {
                assert!(m.contains("still in the environment"), "unexpected: {m}")
            }
            other => panic!("expected drift, got {other:?}"),
        }
    }

    #[test]
    fn routing_on_with_nothing_exported_is_drift() {
        match compute_status(true, None, Some(OURS), true, true) {
            Status::Drifted(m) => assert!(m.contains("not going through Gate"), "unexpected: {m}"),
            other => panic!("expected drift, got {other:?}"),
        }
    }

    #[test]
    fn routing_off_with_nothing_exported_is_clean() {
        assert_eq!(
            compute_status(true, None, Some(OURS), false, true),
            Status::Detected
        );
    }

    /// On Linux the variables *are* the routing mechanism, so opting out cannot
    /// withdraw them. The message must say so instead of sending the user round
    /// a reconnect/disconnect loop that can never clear it.
    #[test]
    fn opting_out_where_the_channel_is_inseparable_explains_itself() {
        match compute_status(false, Some(OURS), Some(OURS), true, false) {
            Status::Drifted(m) => {
                assert!(
                    m.contains("turn routing off"),
                    "must point at the real lever: {m}"
                );
                assert!(
                    !m.contains("disconnect again"),
                    "must not suggest a loop that cannot work: {m}"
                );
            }
            other => panic!("expected drift, got {other:?}"),
        }
    }
}
