//! Launch-time session policy, shared by any host that seeds routing from a
//! stored account. Extracted from the desktop shell's startup thread so the
//! sequencing lives where the collaborators (`oauth`, `org`, `account`) do
//! and can be exercised by the same harnesses; the shell keeps only the
//! tray/flag reaction to the verdict.

use crate::{account, oauth, org};

/// What the launch-time refresh concluded about the stored OAuth session.
pub enum SessionVerdict {
    /// Not an OAuth account (API-key mode, or no OAuth config in this build):
    /// nothing to refresh and nothing to alarm about - the caller leaves its
    /// sign-in signal untouched.
    NotOauth,
    /// The token is fresh and, where the gateway answered, the session was
    /// accepted; a stored org that dropped out of the membership list has
    /// been cleared so the UI routes to the org picker.
    Healthy,
    /// The stored session could not refresh, or the gateway rejected it:
    /// the user must sign in again.
    NeedsSignIn,
}

/// Refresh a stale Cognito access token before the engine seeds itself, so
/// re-honor / auto-enable inject a live token (`enable()` re-reads the stored
/// token via `access_token_for_injection`). Never opens the browser - a
/// failed refresh just yields [`SessionVerdict::NeedsSignIn`] and the UI's
/// "sign in" state. Best-effort and offline-safe: a gateway that cannot be
/// reached gives no verdict and changes nothing.
pub fn refresh_session() -> SessionVerdict {
    if account::auth_mode().unwrap_or_default() != account::AuthMode::OAuth {
        return SessionVerdict::NotOauth;
    }
    let Some(cfg) = oauth::OAuthConfig::from_build_env() else {
        return SessionVerdict::NotOauth;
    };
    // Only a refresh *failure* (Err: a stored session that can't refresh) is
    // an alarm; `Ok(None)` is a signed-out / never-signed-in state and must
    // stay quiet.
    let session = match oauth::ensure_fresh(&cfg) {
        Ok(session) => session,
        Err(e) => {
            eprintln!("[gate] startup OAuth token refresh failed: {e}");
            return SessionVerdict::NeedsSignIn;
        }
    };
    // A locally-fresh session can still be dead at the gateway (upgrade /
    // server-side drift: revoked user, reseeded org data) - the token looks
    // fine here, so only the gateway can say. Ask it once, before the engine
    // seeds itself from this session. Offline starts get no verdict and
    // change nothing.
    if let (Some(tokens), Ok(Some(gateway))) = (session, account::load_base_url()) {
        match org::probe_session(&gateway, &tokens.access_token) {
            org::SessionProbe::Rejected => {
                eprintln!("[gate] gateway rejected the stored OAuth session; prompting sign-in");
                oauth::mark_session_rejected();
                return SessionVerdict::NeedsSignIn;
            }
            org::SessionProbe::Accepted(orgs) => {
                // Session is live, but a stored org that dropped out of the
                // membership list would doom every request; clear it so the
                // UI routes to the org picker (`needsOrg`).
                let stale = account::selected_org()
                    .ok()
                    .flatten()
                    .is_some_and(|(id, _)| !orgs.iter().any(|o| o.org_id == id));
                if stale {
                    eprintln!(
                        "[gate] stored org is no longer a membership; clearing it for re-pick"
                    );
                    if let Err(e) = account::clear_org() {
                        eprintln!("[gate] clearing the stale org failed: {e:#}");
                    }
                }
            }
            org::SessionProbe::Unavailable => {}
        }
    }
    SessionVerdict::Healthy
}
