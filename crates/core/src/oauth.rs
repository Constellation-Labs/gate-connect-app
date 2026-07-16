//! Cognito OAuth (authorization-code + PKCE) for Gate Connect's *own*
//! gateway auth - the replacement for the static `x-gate-api-key`.
//!
//! Mirrors the Gate web app's Hosted UI flow (Cognito `/oauth2/authorize`
//! and `/oauth2/token`, scopes `openid email profile
//! aws.cognito.signin.user.admin`) but as a native **public** client: we
//! open the Hosted UI in the user's browser and capture the redirect on a
//! loopback listener (RFC 8252). The gateway then receives the Cognito
//! access token on `x-gate-authorization`; the pasted API key
//! (`x-gate-api-key`) stays as the legacy fallback.
//!
//! The token bundle (access + refresh + id + expiry) lives in the OS secret
//! store as one JSON blob, under
//! `ai.constellation.gate-connect.account.oauth-tokens`. Non-secret status
//! (expiry, email) is mirrored into `account.json` by the account layer for
//! cheap UI reads. No client-side signature verification - the gateway
//! verifies the JWT; we only read `expires_in` to know when to refresh.

use anyhow::{bail, Context, Result};
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use time::OffsetDateTime;

use crate::env;
use crate::keychain;

/// Keychain label the token bundle is stored under (service
/// `ai.constellation.gate-connect.account.oauth-tokens`).
const KEYCHAIN_LABEL: &str = "oauth-tokens";

/// Refresh once the access token is within this many seconds of expiry, to
/// cover clock skew and requests already in flight.
const EXPIRY_SKEW_SECS: i64 = 60;

/// How often the silent-refresh drivers poll to keep the access token fresh -
/// the standalone CLI relay ([`crate::proxy::relay`]) and the desktop app's
/// background loop both tick on this interval, calling [`ensure_fresh`].
pub const REFRESH_INTERVAL_SECS: u64 = 30;

/// Static, build-time OAuth client configuration. Baked per environment the
/// same way the gateway base URL is (`VITE_GATE_DEFAULT_BASE_URL`): set
/// `GATE_COGNITO_HOSTED_DOMAIN`, `GATE_COGNITO_CLIENT_ID`, and
/// `GATE_COGNITO_SCOPES` (space-separated) at build time.
#[derive(Debug, Clone)]
pub struct OAuthConfig {
    /// Cognito Hosted UI domain - e.g. `auth.constellationgate.ai` or
    /// `<pool>.auth.<region>.amazoncognito.com`. No scheme, no trailing slash.
    pub hosted_domain: String,
    /// Public app client id (no secret - this is a native/public client).
    pub client_id: String,
    /// OAuth scopes, space-joined into the authorize URL's `scope` param.
    pub scopes: Vec<String>,
}

/// One build-time OAuth config value: the process env wins at runtime, else
/// the value baked in at build time. Empty env values are ignored so an
/// exported-but-blank var doesn't blank out a baked default.
fn config_value(name: &str, baked: Option<&str>) -> Option<String> {
    std::env::var(name)
        .ok()
        .filter(|s| !s.is_empty())
        .or_else(|| baked.map(str::to_string))
}

impl OAuthConfig {
    /// Read the OAuth client config. Each value comes from the process env at
    /// runtime if set (dev/staging override, and the CLI's hermetic tests),
    /// otherwise the value baked in at build time via `option_env!`. Returns
    /// `None` when neither supplies the domain/client id, so callers can fall
    /// back to the legacy API-key flow with a clear message instead of
    /// panicking. All three values are public client config (no secret), so a
    /// runtime override is safe.
    pub fn from_build_env() -> Option<Self> {
        let hosted_domain = config_value(
            "GATE_COGNITO_HOSTED_DOMAIN",
            option_env!("GATE_COGNITO_HOSTED_DOMAIN"),
        )?;
        let client_id = config_value(
            "GATE_COGNITO_CLIENT_ID",
            option_env!("GATE_COGNITO_CLIENT_ID"),
        )?;
        let scopes = config_value("GATE_COGNITO_SCOPES", option_env!("GATE_COGNITO_SCOPES"))
            .unwrap_or_else(|| "openid email profile aws.cognito.signin.user.admin".to_string())
            .split_whitespace()
            .map(str::to_string)
            .collect();
        Some(Self {
            hosted_domain,
            client_id,
            scopes,
        })
    }

    fn authorize_endpoint(&self) -> String {
        format!("https://{}/oauth2/authorize", self.hosted_domain)
    }

    fn token_endpoint(&self) -> String {
        // Test seam mirroring `GATE_CONNECT_TEST_*` elsewhere: point the token
        // exchange at a loopback mock. Unset in real builds.
        if let Some(o) = std::env::var_os("GATE_CONNECT_TEST_TOKEN_ENDPOINT") {
            return o.to_string_lossy().into_owned();
        }
        format!("https://{}/oauth2/token", self.hosted_domain)
    }
}

/// PKCE verifier/challenge pair (RFC 7636, S256).
#[derive(Debug, Clone)]
pub struct Pkce {
    /// High-entropy secret kept in-process and sent on the token exchange.
    pub verifier: String,
    /// `base64url(sha256(verifier))`, sent on the authorize request.
    pub challenge: String,
}

/// 32 random bytes, base64url-encoded (no padding).
fn random_b64url() -> String {
    let mut buf = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut buf);
    URL_SAFE_NO_PAD.encode(buf)
}

/// Derive a fresh PKCE pair.
pub fn generate_pkce() -> Pkce {
    let verifier = random_b64url();
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    Pkce {
        verifier,
        challenge,
    }
}

/// Everything the caller needs to drive one interactive login: the URL to
/// open in the browser, plus the `state` and PKCE `verifier` to validate
/// and redeem the callback.
#[derive(Debug, Clone)]
pub struct AuthorizationRequest {
    /// Open this in the system browser.
    pub authorize_url: String,
    /// CSRF token echoed back on the redirect; reject a mismatch.
    pub state: String,
    /// PKCE verifier to pass to [`complete_login`].
    pub verifier: String,
}

/// Build the Hosted UI authorize URL for a fresh login. `redirect_uri` is
/// the loopback callback the listener bound (`http://localhost:<port>/callback`).
pub fn begin_login(cfg: &OAuthConfig, redirect_uri: &str) -> Result<AuthorizationRequest> {
    let pkce = generate_pkce();
    let state = random_b64url();
    let scope = cfg.scopes.join(" ");
    let url = reqwest::Url::parse_with_params(
        &cfg.authorize_endpoint(),
        &[
            ("response_type", "code"),
            ("client_id", cfg.client_id.as_str()),
            ("redirect_uri", redirect_uri),
            ("scope", scope.as_str()),
            ("state", state.as_str()),
            ("code_challenge", pkce.challenge.as_str()),
            ("code_challenge_method", "S256"),
        ],
    )
    .context("building Cognito authorize URL")?;
    Ok(AuthorizationRequest {
        authorize_url: url.into(),
        state,
        verifier: pkce.verifier,
    })
}

/// Stored OAuth token bundle. Serialized as one JSON blob in the secret
/// store.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OAuthTokens {
    pub access_token: String,
    pub refresh_token: String,
    #[serde(default)]
    pub id_token: Option<String>,
    /// Absolute expiry as a Unix timestamp (seconds), computed from the
    /// token response's `expires_in` at fetch time.
    pub expires_at_unix: i64,
}

impl OAuthTokens {
    /// Is the access token expired (or within the skew margin) at `now_unix`?
    pub fn is_expired(&self, now_unix: i64) -> bool {
        now_unix + EXPIRY_SKEW_SECS >= self.expires_at_unix
    }

    /// Best-effort email from the id token's payload, for UI display only -
    /// no signature check (the gateway verifies). `None` if absent/unparseable.
    pub fn email(&self) -> Option<String> {
        let id = self.id_token.as_deref()?;
        let payload_b64 = id.split('.').nth(1)?;
        let payload = URL_SAFE_NO_PAD.decode(payload_b64).ok()?;
        let claims: serde_json::Value = serde_json::from_slice(&payload).ok()?;
        claims.get("email")?.as_str().map(str::to_string)
    }
}

/// Shape of Cognito's `/oauth2/token` JSON response.
#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    id_token: Option<String>,
    expires_in: i64,
}

/// Turn a token-endpoint body into an [`OAuthTokens`]. `fallback_refresh`
/// carries the prior refresh token forward when the response omits one (the
/// refresh grant reuses the same refresh token unless rotation is enabled).
fn parse_token_response(
    body: &str,
    now_unix: i64,
    fallback_refresh: Option<&str>,
) -> Result<OAuthTokens> {
    let tr: TokenResponse = serde_json::from_str(body).context("parsing Cognito token response")?;
    let refresh_token = tr
        .refresh_token
        .or_else(|| fallback_refresh.map(str::to_string))
        .context("token response carried no refresh_token and none to reuse")?;
    Ok(OAuthTokens {
        access_token: tr.access_token,
        refresh_token,
        id_token: tr.id_token,
        expires_at_unix: now_unix + tr.expires_in,
    })
}

/// POST a form-encoded grant to the token endpoint and parse the result.
fn post_token(
    cfg: &OAuthConfig,
    params: &[(&str, &str)],
    fallback_refresh: Option<&str>,
) -> Result<OAuthTokens> {
    // Build the `application/x-www-form-urlencoded` body via `Url`'s
    // serializer so keys/values are percent-encoded correctly.
    let mut serializer = reqwest::Url::parse("http://token.local/").expect("static base URL");
    serializer
        .query_pairs_mut()
        .extend_pairs(params.iter().copied());
    let body = serializer.query().unwrap_or("").to_string();

    // Control-plane call: reach Cognito directly, never through the app's own
    // data-plane proxy. `.no_proxy()` ignores any `HTTP(S)_PROXY` the app set.
    let client = reqwest::blocking::Client::builder()
        .no_proxy()
        .build()
        .context("building the Cognito token HTTP client")?;
    let resp = client
        .post(cfg.token_endpoint())
        .header(
            reqwest::header::CONTENT_TYPE,
            "application/x-www-form-urlencoded",
        )
        .body(body)
        .send()
        .context("calling Cognito token endpoint")?;
    let status = resp.status();
    let body = resp
        .text()
        .context("reading Cognito token endpoint response body")?;
    if !status.is_success() {
        bail!("Cognito token endpoint returned {status}: {body}");
    }
    parse_token_response(
        &body,
        OffsetDateTime::now_utc().unix_timestamp(),
        fallback_refresh,
    )
}

/// Redeem an authorization `code` for tokens (PKCE authorization-code grant).
pub fn complete_login(
    cfg: &OAuthConfig,
    code: &str,
    verifier: &str,
    redirect_uri: &str,
) -> Result<OAuthTokens> {
    post_token(
        cfg,
        &[
            ("grant_type", "authorization_code"),
            ("client_id", cfg.client_id.as_str()),
            ("code", code),
            ("code_verifier", verifier),
            ("redirect_uri", redirect_uri),
        ],
        None,
    )
}

/// Exchange a refresh token for a fresh access token.
pub fn refresh(cfg: &OAuthConfig, refresh_token: &str) -> Result<OAuthTokens> {
    post_token(
        cfg,
        &[
            ("grant_type", "refresh_token"),
            ("client_id", cfg.client_id.as_str()),
            ("refresh_token", refresh_token),
        ],
        Some(refresh_token),
    )
}

fn service() -> String {
    keychain::account_service(KEYCHAIN_LABEL)
}

/// Persist the token bundle to the OS secret store.
pub fn store(tokens: &OAuthTokens) -> Result<()> {
    let user = env::current_user()?;
    let json = serde_json::to_string(tokens).context("serializing oauth tokens")?;
    keychain::set(&service(), &user, &json)
}

/// Load the stored token bundle, if any.
pub fn current() -> Result<Option<OAuthTokens>> {
    let user = env::current_user()?;
    match keychain::get(&service(), &user)? {
        Some(raw) => Ok(Some(
            serde_json::from_str(&raw).context("parsing stored oauth tokens")?,
        )),
        None => Ok(None),
    }
}

/// Delete the stored token bundle. Idempotent.
pub fn clear() -> Result<()> {
    let user = env::current_user()?;
    keychain::delete(&service(), &user)?;
    Ok(())
}

/// The live OAuth session right now: the stored token if still valid, silently
/// refreshed via the refresh token if it's past its skew-adjusted expiry, or
/// `None` when there's no usable session - never signed in, signed out, or the
/// refresh token is dead / unreachable. Never errors: a failed refresh reports
/// `None` so callers drop to the sign-in prompt (status) or the legacy API key
/// (injection) rather than surfacing a transient error or riding a token that no
/// longer works.
///
/// Shared by [`access_token_for_injection`] and the Tauri `oauth_status`
/// command so the credential the engine actually sends and the signed-in state
/// the UI shows can't disagree - the divergence that let an expired session keep
/// reading as "signed in" while traffic had quietly reverted to the API key.
pub fn live_session() -> Option<OAuthTokens> {
    let cfg = OAuthConfig::from_build_env()?;
    ensure_fresh(&cfg).ok().flatten()
}

/// The access token to inject into gateway requests right now: the live session's
/// access token (refreshed if it had expired), else an empty string (meaning
/// "fall back to the legacy API key"). This is the single source of truth the
/// proxy managers seed the engine from.
pub fn access_token_for_injection() -> String {
    live_session().map(|t| t.access_token).unwrap_or_default()
}

/// Return the freshest token bundle, refreshing silently if the stored one is
/// past its skew-adjusted expiry. Used at startup and before injection so a
/// long-idle session comes back signed in without prompting.
///
/// - No stored bundle → `Ok(None)` (never signed in / signed out).
/// - Stored and still valid → `Ok(Some(unchanged))`.
/// - Stored but expired: exchange the refresh token, persist, and return the
///   new bundle. A failed refresh (revoked / expired refresh token) surfaces as
///   `Err` so the caller can drop to the interactive sign-in prompt.
pub fn ensure_fresh(cfg: &OAuthConfig) -> Result<Option<OAuthTokens>> {
    let Some(tokens) = current()? else {
        return Ok(None);
    };
    if !tokens.is_expired(OffsetDateTime::now_utc().unix_timestamp()) {
        return Ok(Some(tokens));
    }
    let refreshed =
        refresh(cfg, &tokens.refresh_token).context("refreshing expired access token")?;
    store(&refreshed)?;
    Ok(Some(refreshed))
}

/// Loopback ports the app tries, in order, for the OAuth redirect. Cognito
/// requires an exact callback-URL match, so **these must be registered as
/// allowed callback URLs on the app client** (`http://localhost:<port>/callback`).
/// Tests pass `&[0]` to bind an ephemeral port instead.
pub const REDIRECT_PORTS: &[u16] = &[8977, 8978, 8979];

/// How long to wait for the browser to hit the loopback callback before
/// giving up on an interactive login.
const LOGIN_TIMEOUT_SECS: u64 = 300;

const SUCCESS_HTML: &str = "<!doctype html><meta charset=utf-8><title>Signed in</title>\
<body style=\"font:15px system-ui;margin:4rem auto;max-width:24rem;text-align:center;color:#1a1a1a\">\
<h1 style=\"font-size:1.1rem\">You're signed in</h1>\
<p>You can close this tab and return to Gate Connect.</p>";

const ERROR_HTML: &str = "<!doctype html><meta charset=utf-8><title>Sign-in failed</title>\
<body style=\"font:15px system-ui;margin:4rem auto;max-width:24rem;text-align:center;color:#1a1a1a\">\
<h1 style=\"font-size:1.1rem\">Sign-in didn't complete</h1>\
<p>You can close this tab and try again from Gate Connect.</p>";

/// A bound loopback listener that catches the OAuth redirect. Its
/// `redirect_uri` must be passed to [`begin_login`] and [`complete_login`]
/// unchanged.
pub struct LoopbackListener {
    listeners: Vec<std::net::TcpListener>,
    redirect_uri: String,
}

impl LoopbackListener {
    /// Bind the first available port from `candidate_ports`. Pass
    /// [`REDIRECT_PORTS`] in production or `&[0]` for an ephemeral test port.
    pub fn bind(candidate_ports: &[u16]) -> Result<Self> {
        for &port in candidate_ports {
            // Bind IPv4 loopback first; it pins the concrete port (which
            // matters for the `0` ephemeral case) that the IPv6 bind reuses.
            let Ok(v4) = std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, port)) else {
                continue;
            };
            let bound = v4
                .local_addr()
                .context("reading loopback listener address")?
                .port();
            // The redirect URI advertises `localhost` (the only host Cognito
            // accepts over http), so the browser may connect over either IP
            // family. Also accept IPv6 loopback so the callback lands whichever
            // one `localhost` resolves to; best-effort, since a host may lack `::1`.
            let mut listeners = vec![v4];
            if let Ok(v6) = std::net::TcpListener::bind((std::net::Ipv6Addr::LOCALHOST, bound)) {
                listeners.push(v6);
            }
            return Ok(Self {
                listeners,
                redirect_uri: format!("http://localhost:{bound}/callback"),
            });
        }
        bail!("no loopback callback port available (tried {candidate_ports:?})");
    }

    pub fn redirect_uri(&self) -> &str {
        &self.redirect_uri
    }

    /// Block until the browser hits `/callback`, validate `state`, and return
    /// the authorization `code`. Ignores unrelated requests (e.g. favicon).
    pub fn wait_for_code(
        &self,
        expected_state: &str,
        timeout: std::time::Duration,
    ) -> Result<String> {
        for listener in &self.listeners {
            listener
                .set_nonblocking(true)
                .context("setting loopback listener non-blocking")?;
        }
        let deadline = std::time::Instant::now() + timeout;
        loop {
            for listener in &self.listeners {
                match listener.accept() {
                    Ok((stream, _)) => match self.handle_callback(stream, expected_state)? {
                        Some(code) => return Ok(code),
                        None => {} // not the callback (favicon, etc.); keep polling
                    },
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
                    Err(e) => return Err(e).context("accepting loopback callback"),
                }
            }
            if std::time::Instant::now() >= deadline {
                bail!("timed out waiting for the login redirect");
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
    }

    /// Parse one connection. `Ok(Some(code))` on a valid callback,
    /// `Ok(None)` for an unrelated path, `Err` on an OAuth error / bad state.
    fn handle_callback(
        &self,
        mut stream: std::net::TcpStream,
        expected_state: &str,
    ) -> Result<Option<String>> {
        use std::io::Read;

        stream
            .set_read_timeout(Some(std::time::Duration::from_secs(5)))
            .ok();
        let mut buf = Vec::new();
        let mut tmp = [0u8; 1024];
        // We only need the request line (GET target HTTP/1.1); stop at CRLF.
        loop {
            match stream.read(&mut tmp) {
                Ok(0) => break,
                Ok(n) => {
                    buf.extend_from_slice(&tmp[..n]);
                    if buf.windows(2).any(|w| w == b"\r\n") || buf.len() > 8192 {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
        let text = String::from_utf8_lossy(&buf);
        let target = text
            .lines()
            .next()
            .and_then(|line| line.split_whitespace().nth(1))
            .unwrap_or("");
        let url = reqwest::Url::parse(&format!("http://localhost{target}"))
            .context("parsing loopback callback URL")?;
        if url.path() != "/callback" {
            let _ = write_http(&mut stream, "404 Not Found", "");
            return Ok(None);
        }

        let params: std::collections::HashMap<_, _> = url.query_pairs().into_owned().collect();
        if let Some(err) = params.get("error") {
            let _ = write_http(&mut stream, "200 OK", ERROR_HTML);
            let desc = params
                .get("error_description")
                .map(|d| format!(": {d}"))
                .unwrap_or_default();
            bail!("authorization failed ({err}){desc}");
        }
        let state = params
            .get("state")
            .context("callback missing state parameter")?;
        if state != expected_state {
            let _ = write_http(&mut stream, "200 OK", ERROR_HTML);
            bail!("state mismatch on OAuth callback - possible CSRF, aborting");
        }
        let code = params
            .get("code")
            .context("callback missing code parameter")?
            .clone();
        let _ = write_http(&mut stream, "200 OK", SUCCESS_HTML);
        Ok(Some(code))
    }
}

fn write_http(stream: &mut std::net::TcpStream, status: &str, body: &str) -> std::io::Result<()> {
    use std::io::Write;
    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(response.as_bytes())?;
    stream.flush()
}

/// Run one full interactive login: bind a loopback callback, open the Hosted
/// UI via `open_url`, wait for the redirect, exchange the code, and persist
/// the tokens. `open_url` should launch the browser and return promptly (the
/// redirect arrives asynchronously). Returns the stored tokens.
pub fn login<F>(cfg: &OAuthConfig, candidate_ports: &[u16], open_url: F) -> Result<OAuthTokens>
where
    F: FnOnce(&str) -> Result<()>,
{
    let listener = LoopbackListener::bind(candidate_ports)?;
    let req = begin_login(cfg, listener.redirect_uri())?;
    open_url(&req.authorize_url).context("opening the sign-in page in the browser")?;
    let code = listener.wait_for_code(
        &req.state,
        std::time::Duration::from_secs(LOGIN_TIMEOUT_SECS),
    )?;
    let tokens = complete_login(cfg, &code, &req.verifier, listener.redirect_uri())?;
    store(&tokens)?;
    Ok(tokens)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> OAuthConfig {
        OAuthConfig {
            hosted_domain: "auth.example.test".to_string(),
            client_id: "client123".to_string(),
            scopes: vec!["openid".to_string(), "email".to_string()],
        }
    }

    #[test]
    fn pkce_challenge_is_s256_of_verifier() {
        let pkce = generate_pkce();
        let expected = URL_SAFE_NO_PAD.encode(Sha256::digest(pkce.verifier.as_bytes()));
        assert_eq!(pkce.challenge, expected);
        // base64url must not contain +, /, or padding.
        assert!(!pkce.challenge.contains(['+', '/', '=']));
        assert!(!pkce.verifier.contains(['+', '/', '=']));
    }

    #[test]
    fn pkce_pairs_are_unique() {
        assert_ne!(generate_pkce().verifier, generate_pkce().verifier);
    }

    #[test]
    fn authorize_url_carries_pkce_and_flow_params() {
        let req = begin_login(&cfg(), "http://localhost:52847/callback").unwrap();
        let url = reqwest::Url::parse(&req.authorize_url).unwrap();
        assert_eq!(url.host_str(), Some("auth.example.test"));
        assert_eq!(url.path(), "/oauth2/authorize");
        let q: std::collections::HashMap<_, _> = url.query_pairs().into_owned().collect();
        assert_eq!(q["response_type"], "code");
        assert_eq!(q["client_id"], "client123");
        assert_eq!(q["redirect_uri"], "http://localhost:52847/callback");
        assert_eq!(q["scope"], "openid email");
        assert_eq!(q["code_challenge_method"], "S256");
        assert_eq!(q["state"], req.state);
        assert!(!q["code_challenge"].is_empty());
    }

    #[test]
    fn parses_authorization_code_response() {
        let body = r#"{"access_token":"at","refresh_token":"rt","id_token":"it","expires_in":3600,"token_type":"Bearer"}"#;
        let t = parse_token_response(body, 1_000, None).unwrap();
        assert_eq!(t.access_token, "at");
        assert_eq!(t.refresh_token, "rt");
        assert_eq!(t.id_token.as_deref(), Some("it"));
        assert_eq!(t.expires_at_unix, 1_000 + 3600);
    }

    #[test]
    fn refresh_response_reuses_prior_refresh_token() {
        // Cognito omits refresh_token on the refresh grant unless rotation is on.
        let body = r#"{"access_token":"at2","expires_in":3600,"token_type":"Bearer"}"#;
        let t = parse_token_response(body, 2_000, Some("old-rt")).unwrap();
        assert_eq!(t.access_token, "at2");
        assert_eq!(t.refresh_token, "old-rt");
        assert_eq!(t.expires_at_unix, 2_000 + 3600);
    }

    #[test]
    fn missing_refresh_token_with_no_fallback_errors() {
        let body = r#"{"access_token":"at","expires_in":3600,"token_type":"Bearer"}"#;
        assert!(parse_token_response(body, 0, None).is_err());
    }

    #[test]
    fn expiry_respects_skew_margin() {
        let t = OAuthTokens {
            access_token: "a".into(),
            refresh_token: "r".into(),
            id_token: None,
            expires_at_unix: 1_000,
        };
        assert!(!t.is_expired(900)); // 900 + 60 < 1000
        assert!(t.is_expired(950)); // 950 + 60 >= 1000, within skew
        assert!(t.is_expired(1_000));
    }

    #[test]
    fn email_read_from_id_token_payload() {
        // header.payload.signature; only the payload matters, unsigned is fine.
        let payload = URL_SAFE_NO_PAD.encode(br#"{"email":"dev@example.test"}"#);
        let t = OAuthTokens {
            access_token: "a".into(),
            refresh_token: "r".into(),
            id_token: Some(format!("h.{payload}.s")),
            expires_at_unix: 0,
        };
        assert_eq!(t.email().as_deref(), Some("dev@example.test"));
    }
}
