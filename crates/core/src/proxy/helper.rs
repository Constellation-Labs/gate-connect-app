//! The long-lived proxy helper daemon (Linux). Owns the loopback listener via
//! [`super::engine`] so the proxy outlives the GUI process: on GUI quit/crash it
//! drops to pass-through (MITM port blind-tunnels, relay forwards direct to the
//! real upstream) instead of stranding a frozen session's proxy pointer at a
//! dead port. Driven over a Unix-domain control
//! socket - see [`super::control`] for the protocol and the access-control
//! rationale (`0700` dir / `0600` socket, `SO_PEERCRED` UID check, per-run
//! token, catalog-constrained intercept).
//!
//! Spawned by the GUI as `<current-exe> --proxy-helper` (so there's no separate
//! binary to package or locate), detached from the GUI's lifetime. It runs
//! until logout (when the user session tears down) or an explicit `Shutdown`.

use std::os::fd::AsRawFd;
use std::os::unix::fs::PermissionsExt;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};

use crate::proxy::control::{self, Request, Response};
use crate::proxy::engine::{self, EngineConfig, RunningEngine};

/// Shared engine handle. Started lazily on the first `SetIntercept` (which
/// carries the CA the engine needs) and then kept for the daemon's whole life;
/// `SetPassthrough` / client-disconnect only drop it to pass-through
/// ([`set_passthrough`]), never stop it, so the ports stay bound.
type Shared = Arc<Mutex<Option<RunningEngine>>>;

/// Entry point invoked from the desktop binary when launched with
/// `--proxy-helper`. Builds a tokio runtime and serves the control socket until
/// shutdown. Never returns `Ok` while serving; returns `Err` only if it can't
/// come up (e.g. another daemon already owns the socket - then the caller
/// should just connect to that one).
pub fn run_daemon() -> Result<()> {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("building daemon tokio runtime")?;
    rt.block_on(serve())
}

async fn serve() -> Result<()> {
    let sock = control::socket_path()?;

    // Singleton guard: hold an exclusive flock for the daemon's whole life. If
    // another daemon holds it we defer to that one. flock releases on process
    // exit (even a crash), so this is race-free where the old connect-then-bind
    // check had a TOCTOU window (two daemons could both pass it and fight over
    // the socket). Held in `_singleton` for all of `serve`.
    let lock_path = control::singleton_lock_path()?;
    let _singleton = match crate::proxy::flock::FileLock::acquire(&lock_path, false)? {
        Some(lock) => lock,
        None => anyhow::bail!("a proxy helper is already running"),
    };

    // We own the singleton lock, so we're the only daemon - clear any stale
    // socket from a previous run and bind fresh.
    let _ = std::fs::remove_file(&sock);

    // Write our pid (0600) so a client that can't shut us down cleanly (e.g. a
    // future protocol we don't speak) can force-kill us and take over the
    // singleton lock. Held for the daemon's life; the next daemon overwrites it.
    let pid_path = control::pid_path()?;
    std::fs::write(&pid_path, std::process::id().to_string())
        .with_context(|| format!("writing {}", pid_path.display()))?;
    std::fs::set_permissions(&pid_path, std::fs::Permissions::from_mode(0o600))
        .with_context(|| format!("locking down {}", pid_path.display()))?;

    // Write the auth token (0600) the GUI must echo back in Hello.
    let token = control::random_token()?;
    let token_path = control::token_path()?;
    std::fs::write(&token_path, &token)
        .with_context(|| format!("writing {}", token_path.display()))?;
    std::fs::set_permissions(&token_path, std::fs::Permissions::from_mode(0o600))
        .with_context(|| format!("locking down {}", token_path.display()))?;

    let listener =
        UnixListener::bind(&sock).with_context(|| format!("binding {}", sock.display()))?;
    std::fs::set_permissions(&sock, std::fs::Permissions::from_mode(0o600))
        .with_context(|| format!("locking down {}", sock.display()))?;

    let engine: Shared = Arc::new(Mutex::new(None));
    // Set by a client that told us its lifetime is not the routing lifetime
    // (the CLI). Lives outside the connection loop on purpose: the whole point
    // is that it outlives the connection that set it.
    let detached = Arc::new(AtomicBool::new(false));
    // Control connections currently open. The disarm rule below is "the LAST
    // client left", not "a client left", which only becomes a distinction once
    // connections can overlap.
    let clients = Arc::new(AtomicUsize::new(0));
    // Shared with every connection task, so it outlives any one of them.
    let token = Arc::new(token);
    // SAFETY: geteuid never fails.
    let owner_uid = unsafe { libc::geteuid() };

    loop {
        let (stream, _) = match listener.accept().await {
            Ok(pair) => pair,
            Err(e) => {
                eprintln!("[gate-proxyd] accept failed: {e}");
                continue;
            }
        };
        // Counted here rather than inside the task: `accept` has returned, so
        // the peer is connected already, and a client that came and went in the
        // gap before its task started would otherwise read a count of zero and
        // disarm a proxy someone else is still minding.
        clients.fetch_add(1, Ordering::SeqCst);
        let engine = Arc::clone(&engine);
        let detached = Arc::clone(&detached);
        let clients = Arc::clone(&clients);
        let token = Arc::clone(&token);
        // One task per connection. Handling one client at a time meant the
        // GUI's held control connection blocked every other caller: a CLI
        // `proxy status` waited in the accept backlog until the GUI let go, hit
        // CONTROL_TIMEOUT, and reported the proxy off while it was on.
        tokio::spawn(async move {
            if let Err(e) = handle_conn(stream, owner_uid, &token, &engine, &detached).await {
                eprintln!("[gate-proxyd] connection ended: {e}");
            }
            // On disconnect, drop back to pass-through UNLESS the client asked
            // to be detached: a short-lived caller like `gate-connect proxy
            // enable` exits as soon as it has armed the engine, and reverting
            // on its exit silently un-routed the machine while status still
            // read "on". `fetch_sub` returns the count *before* the decrement,
            // so 1 means this task was the last one out; anyone else still
            // connected keeps the engine armed.
            let was_last = clients.fetch_sub(1, Ordering::SeqCst) == 1;
            if detached.load(Ordering::SeqCst) {
                eprintln!("[gate-proxyd] client disconnected; staying detached and intercepting");
            } else if was_last {
                set_passthrough(&engine);
            }
        });
    }
}

/// Read the connecting peer's UID via `SO_PEERCRED`. The kernel fills this in
/// at connect time, so it can't be spoofed by the peer.
fn peer_uid(stream: &UnixStream) -> Result<u32> {
    let fd = stream.as_raw_fd();
    let mut cred = libc::ucred {
        pid: 0,
        uid: 0,
        gid: 0,
    };
    let mut len = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
    // SAFETY: fd is a valid connected socket; cred/len are sized correctly.
    let rc = unsafe {
        libc::getsockopt(
            fd,
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            &mut cred as *mut _ as *mut libc::c_void,
            &mut len,
        )
    };
    if rc != 0 {
        return Err(std::io::Error::last_os_error()).context("getsockopt(SO_PEERCRED)");
    }
    Ok(cred.uid)
}

async fn handle_conn(
    stream: UnixStream,
    owner_uid: u32,
    token: &str,
    engine: &Shared,
    detached: &AtomicBool,
) -> Result<()> {
    // Access control #2: reject any peer that isn't us, before reading a byte.
    let uid = peer_uid(&stream)?;
    if uid != owner_uid {
        anyhow::bail!("rejecting control connection from uid {uid} (owner is {owner_uid})");
    }

    let (read_half, mut write_half) = stream.into_split();
    let mut lines = BufReader::new(read_half).lines();

    // Access control #3: first message must authenticate with the token.
    let first = lines
        .next_line()
        .await
        .context("reading Hello")?
        .context("connection closed before Hello")?;
    let authed = matches!(
        serde_json::from_str::<Request>(&first),
        Ok(Request::Hello { token: t, .. }) if t == token
    );
    write_response(
        &mut write_half,
        &Response::Hello {
            ok: authed,
            version: control::PROTOCOL_VERSION,
            fingerprint: control::BUILD_FINGERPRINT.into(),
        },
    )
    .await?;
    if !authed {
        anyhow::bail!("Hello failed authentication");
    }

    while let Some(line) = lines.next_line().await.context("reading request")? {
        if line.trim().is_empty() {
            continue;
        }
        let req: Request = match serde_json::from_str(&line) {
            Ok(r) => r,
            Err(e) => {
                write_response(
                    &mut write_half,
                    &Response::Error {
                        message: format!("bad request: {e}"),
                    },
                )
                .await?;
                continue;
            }
        };
        // Shutdown is special: ack, stop the engine, then exit the process so
        // the daemon (and its listener) goes away on explicit request.
        if matches!(req, Request::Shutdown) {
            write_response(&mut write_half, &Response::Ok).await?;
            if let Some(running) = engine.lock().expect("engine mutex poisoned").take() {
                running.stop();
            }
            std::process::exit(0);
        }
        let resp = handle_request(req, engine, detached);
        write_response(&mut write_half, &resp).await?;
    }
    Ok(())
}

/// Apply one request to the engine and produce the reply. Synchronous: engine
/// updates are cheap (watch-channel sends) and `start` returns once bound.
fn handle_request(req: Request, engine: &Shared, detached: &AtomicBool) -> Response {
    match req {
        Request::Hello { .. } => Response::Error {
            message: "unexpected Hello".into(),
        },
        Request::SetIntercept {
            gateway_base_url,
            api_key,
            oauth_token,
            org_id,
            billing_mode,
            ca_cert_pem,
            ca_key_pem,
            domains,
            detached: want_detached,
            preferred_port,
            preferred_relay_port,
        } => {
            // What the daemon actually received, as opposed to what the CLI
            // printed. The engine ends up with zero ENABLED domains while
            // `proxy enable` reports anthropic on, and every link in between
            // has been cleared: load_domains starts from default_domains and
            // cannot be empty, set_intercept sends `domains.to_vec()`,
            // validate_domains only validates, and ProxyDomain carries no serde
            // attributes on `enabled`. This is the one hop with no evidence.
            if crate::proxy::engine::debug_log() {
                let on: Vec<&str> = domains
                    .iter()
                    .filter(|d| d.enabled)
                    .map(|d| d.slug.as_str())
                    .collect();
                eprintln!(
                    "[gate-proxyd] SetIntercept received {} domains, enabled: [{}]",
                    domains.len(),
                    on.join(",")
                );
            }
            // Recorded before the engine is touched, so a client that arms and
            // exits immediately cannot lose the race with its own disconnect.
            detached.store(want_detached, Ordering::SeqCst);
            // Access control #4: only ever route catalog providers.
            if let Err(e) = control::validate_domains(&domains) {
                return Response::Error {
                    message: e.to_string(),
                };
            }
            let mut guard = engine.lock().expect("engine mutex poisoned");
            // A finished engine (crash) can't be updated - drop it and restart.
            if guard.as_ref().is_some_and(|e| e.is_finished()) {
                *guard = None;
            }
            match guard.as_ref() {
                Some(running) => {
                    running.update_api_key(&api_key);
                    running.update_token(&oauth_token);
                    running.update_org(&org_id);
                    running.update_mode(billing_mode);
                    running.update_domains(&domains);
                    running.set_relay_intercept(true);
                    Response::Intercepting {
                        port: running.port(),
                        relay_port: running.relay_port(),
                    }
                }
                None => {
                    match engine::start(
                        EngineConfig {
                            gateway_base_url,
                            api_key,
                            oauth_token,
                            org_id,
                            billing_mode,
                            domains,
                            ca_cert_pem,
                            ca_key_pem,
                            preferred_port,
                            // No PAC on Linux (env-var proxies wire clients
                            // straight at the engine port).
                            preferred_pac_port: None,
                            preferred_relay_port,
                            // The daemon runs as the owner; only intercept this
                            // user's own traffic. SAFETY: geteuid never fails.
                            owner_uid: Some(unsafe { libc::geteuid() }),
                            upstream_proxy: None,
                        },
                        // The daemon doesn't auto-revert on engine death; the
                        // GUI's drop-in lifecycle and a later re-enable handle
                        // recovery. Just note it.
                        || eprintln!("[gate-proxyd] engine exited unexpectedly"),
                    ) {
                        Ok(running) => {
                            let port = running.port();
                            let relay_port = running.relay_port();
                            *guard = Some(running);
                            Response::Intercepting { port, relay_port }
                        }
                        Err(e) => Response::Error {
                            message: format!("starting engine: {e}"),
                        },
                    }
                }
            }
        }
        Request::SetPassthrough => {
            // An explicit request always wins over a standing detach: this is
            // `proxy disable`, i.e. the user asking for routing off, not a
            // client going away.
            detached.store(false, Ordering::SeqCst);
            set_passthrough(engine);
            // Symmetric with the SetIntercept line above. Disarming is the one
            // state change here that left no trace, and its absence is what
            // separates "the disable landed" from "the CLI cleared the
            // snapshot and the drop-in but never reached the daemon, which
            // kept intercepting" - a case that otherwise looks identical from
            // outside, because only newly launched processes stop routing.
            eprintln!("[gate-proxyd] SetPassthrough received; engine disarmed");
            Response::Ok
        }
        Request::Status => {
            let guard = engine.lock().expect("engine mutex poisoned");
            match guard.as_ref().filter(|e| !e.is_finished()) {
                Some(running) => Response::Status {
                    running: true,
                    port: Some(running.port()),
                    intercepting: running.intercepting(),
                },
                None => Response::Status {
                    running: false,
                    port: None,
                    intercepting: 0,
                },
            }
        }
        Request::Shutdown => Response::Ok,
    }
}

/// Drop both listeners to their credential-free fallbacks, without stopping
/// the engine (the ports stay bound). MITM port: clear the domain set so it
/// blind-tunnels everything. Relay port: forward direct to the real upstream
/// under the tool's own credential instead of rewriting to the gateway - with
/// no GUI connected nothing keeps the injected token fresh, so routing to
/// Gate would quietly decay into 401s; going direct keeps CLI tools working,
/// just not through Gate. No-op if the engine isn't running.
fn set_passthrough(engine: &Shared) {
    if let Some(running) = engine
        .lock()
        .expect("engine mutex poisoned")
        .as_ref()
        .filter(|e| !e.is_finished())
    {
        running.update_domains(&[]);
        running.set_relay_intercept(false);
    }
}

async fn write_response(w: &mut (impl AsyncWriteExt + Unpin), resp: &Response) -> Result<()> {
    let mut line = serde_json::to_string(resp).context("serializing response")?;
    line.push('\n');
    w.write_all(line.as_bytes())
        .await
        .context("writing response")?;
    w.flush().await.context("flushing response")?;
    Ok(())
}
