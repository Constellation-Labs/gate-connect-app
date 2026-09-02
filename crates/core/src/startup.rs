//! Session policy - launch-time seeding and mid-session re-verification -
//! shared by any host that routes from a stored account. Extracted from the
//! desktop shell's startup thread so the sequencing lives where the
//! collaborators (`oauth`, `org`, `account`) do and can be exercised by the
//! same harnesses; the shell keeps only the tray/flag reaction to the verdict.

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

/// Verdict of [`reverify_session`]: what the gateway says about a session it
/// has just been seen to refuse.
pub enum Recheck {
    /// The session is alive after all. Carries the access token to re-seed
    /// routing with - it is a *new* one, so pushing it into the engine is what
    /// makes traffic work again.
    Recovered(String),
    /// The gateway refused the session even with a freshly minted access
    /// token. Recorded via [`oauth::mark_session_rejected`], so the whole app
    /// (status, injection, tray) already reads as signed out by the time this
    /// returns; the caller only has to react in its own UI.
    Dead,
    /// No verdict: not an OAuth account, nothing stored, or the gateway could
    /// not be reached. Nothing changed, and nothing should be concluded - an
    /// offline moment must never sign anyone out.
    Unchanged,
}

/// Re-verify a session the gateway has just refused mid-run, and recover it if
/// it can be recovered.
///
/// The launch-time [`refresh_session`] probe catches a session that died while
/// the app was closed, but it runs once. Everything after it trusts
/// [`OAuthTokens::is_expired`](oauth::OAuthTokens::is_expired), which compares
/// two readings of the local clock and therefore lies when the clock moves
/// between them - a suspended machine whose guest clock stops comes back
/// believing an hours-dead token is minutes old, refreshes nothing, and every
/// request 401s while the app shows "connected". The gateway's refusal is the
/// only signal that exists for that state, so this is what a refusal routes
/// to: force a refresh past the local expiry check ([`oauth::force_refresh`]),
/// then ask the gateway directly.
///
/// The probe, not the 401, is the authority. A 401 seen on the data plane is
/// only a *suspicion* - a rewritten request also carries the client's own
/// upstream credential, so the refusal may belong to that and not to us -
/// while [`org::probe_session`] is our own token, our own request, no client
/// credential involved. Only its verdict marks a session dead.
pub fn reverify_session() -> Recheck {
    if account::auth_mode().unwrap_or_default() != account::AuthMode::OAuth {
        return Recheck::Unchanged;
    }
    let Some(cfg) = oauth::OAuthConfig::from_build_env() else {
        return Recheck::Unchanged;
    };
    let tokens = match oauth::force_refresh(&cfg) {
        // No stored bundle: signed out, nothing to recover.
        Ok(None) => return Recheck::Unchanged,
        Ok(Some(tokens)) => tokens,
        // Cognito refused the grant: the refresh token is dead (revoked,
        // expired, or minted by another app client) and the session cannot
        // come back without an interactive sign-in. That is a verdict, and
        // the same one a rejected probe gives.
        Err(e) if e.is_refusal() => {
            eprintln!("[gate] re-verifying the session: the refresh was refused: {e}");
            oauth::mark_session_rejected();
            return Recheck::Dead;
        }
        // Cognito could not be reached at all. This is the likeliest thing
        // to happen at exactly the moment that brought us here - a machine
        // that just woke up 401s on the first requests out while its network
        // is still coming up - so it must stay a non-verdict. Signing out on
        // it would turn the failure this whole path exists to recover from
        // into a permanent one.
        Err(e) => {
            eprintln!("[gate] re-verifying the session: the refresh could not be reached: {e}");
            return Recheck::Unchanged;
        }
    };
    let Ok(Some(gateway)) = account::load_base_url() else {
        return Recheck::Unchanged;
    };
    match org::probe_session(&gateway, &tokens.access_token) {
        org::SessionProbe::Accepted(_) => Recheck::Recovered(tokens.access_token),
        org::SessionProbe::Rejected => {
            eprintln!(
                "[gate] the gateway rejected the session even after a forced refresh; \
                 sign-in required"
            );
            oauth::mark_session_rejected();
            Recheck::Dead
        }
        // Unreachable or a non-auth error: no verdict. The forced refresh
        // still happened and its token is stored, so a caller that re-seeds
        // routing on `Recovered` simply doesn't - the next 30s tick picks the
        // new token up through `live_session` anyway.
        org::SessionProbe::Unavailable => Recheck::Unchanged,
    }
}
