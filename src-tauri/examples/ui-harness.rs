//! The UI end-to-end harness: the real command table, hosted with no window.
//!
//! What it is for. `e2e/*.spec.ts` drives the real frontend against a *fake*
//! backend (`e2e/install.ts`), so it proves the UI's own orchestration and
//! nothing below it; `ci/e2e/run.sh` drives the real relay with no UI at all.
//! Neither one can answer "does clicking the switch actually route traffic",
//! which is the question a user asks. This closes that gap: Playwright drives
//! the real frontend, every `invoke` lands on the real Rust command, and the
//! engine those commands start is the one that ships.
//!
//! Why a mock runtime rather than the real app. Driving the shipped binary
//! would need a WebDriver, and macOS has none - WKWebView implements no
//! `webdriver` protocol, so a GUI-driving suite would cover two of the three
//! platforms and miss the one with the most platform-specific behaviour. It
//! would also need a display, a tray icon, an updater and an autostart plugin
//! that writes a real login item on the runner. `tauri::test::mock_builder`
//! takes the same `invoke_handler` the app registers and answers IPC with no
//! window, no event loop and no plugins, on every platform, so the matrix in
//! `ci.yml` is the real one.
//!
//! What that costs, stated plainly. The webview here is Playwright's Chromium,
//! not WKWebView / WebView2 / WebKitGTK, so this proves interaction and
//! backend wiring, never per-engine rendering. The tray, real window lifecycle,
//! OS trust dialogs and the updater are out of reach, and `APP_HANDLE` in
//! `lib.rs` is a `OnceLock<AppHandle<Wry>>` that this runtime cannot fill - so
//! the few backend-initiated emits that go through it are inert here. Commands
//! that emit through their own `app` argument work normally, and `/events`
//! below replays them.
//!
//! ## What this port is, and why it is guarded
//!
//! `/invoke` reaches EVERY command the app has, including ones that leave the
//! throwaway home: `set_launch_at_login` writes a real login item,
//! `close_running_agents` scans and kills processes machine-wide, `proxy_enable`
//! sets the system proxy and installs a certificate. A page in the developer's
//! browser can make cross-origin requests to loopback, so an unauthenticated
//! port here would hand any web page that whole surface for as long as a test
//! run lasts. Hence `GATE_UI_HARNESS_TOKEN`: required on every route, supplied
//! by `playwright.live.config.ts`, and unguessable. The permissive CORS header
//! stays, because with the token the browser's same-origin policy is not what
//! is protecting this.
//!
//! It refuses to start without the file seams for the same reason. Without
//! them these commands drive the developer's REAL home, keychain, gateway and
//! system proxy, which is precisely the configuration nobody wants behind a
//! loopback port. `GATE_CONNECT_TEST_HOME` roots every per-user path,
//! `GATE_CONNECT_TEST_SECRETS` replaces the OS secret store with files, and
//! `GATE_CONNECT_TEST_CA` is what lets the relay trust the mock gateway's
//! throwaway CA. All three are inert in release builds (`crates/core/src/env.rs`).
//! It is a dev-dependency example and never part of a shipped build.
//!
//! ## One limitation worth knowing before writing a spec
//!
//! `app.run()` is never called, so the mock runtime only ENQUEUES what
//! `run_on_main_thread` is given and never runs it (`tauri::test`'s
//! `mock_runtime`). `lib.rs` uses that hop for window work on macOS and Linux,
//! so those call sites are silent no-ops here. Nothing in the routing path
//! depends on one; a command that WAITED on such a task would park its blocking
//! thread for the life of the run, so do not add one without checking.
//!
//! Wire protocol, all on loopback, every route requiring
//! `x-gate-harness-token`:
//!   POST /invoke   {"cmd": "...", "payload": {...}}  -> {"ok": <json>} | {"err": <json>}
//!   GET  /events?since=N                             -> {"next": M, "events": [...]}
//!   GET  /health                                     -> {"ok": true, "home": …, …}
//!
//! `/events` long-polls: it parks for up to a second when there is nothing new,
//! so the page's loop costs one idle request per second instead of a spin. SSE
//! would stream, but a streaming body is more harness than this needs.

use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Method, Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use tauri::ipc::{CallbackFn, InvokeBody, InvokeResponseBody};
use tauri::test::{mock_builder, mock_context, noop_assets, MockRuntime, INVOKE_KEY};
use tauri::webview::InvokeRequest;
use tauri::{Listener, WebviewWindowBuilder};

/// Every event the app emits, in order, with the sequence number `/events`
/// pages through. Kept unbounded: a run is seconds long and dropping an event
/// would make a test flake rather than fail.
#[derive(Default)]
struct EventLog {
    events: Vec<serde_json::Value>,
}

/// The event names `lib.rs` emits. Registered explicitly rather than with a
/// wildcard because `listen_any` takes one name: a missing entry here shows up
/// as a UI that never refreshes, so keep it in step with the `emit` calls.
const EVENTS: &[&str] = &[
    "proxy-state-changed",
    "tools-changed",
    "traffic-observed",
    "session-changed",
    "session-signin-required",
    "switch-org-requested",
    "security-feed-state",
    "security-feed-history",
    "security-events-requested",
    "security-event",
    "recovery-details-requested",
    "quit-requested",
    "cf-challenge-required",
    "backend-error-pending",
];

fn json(status: StatusCode, v: serde_json::Value) -> Response<Full<Bytes>> {
    Response::builder()
        .status(status)
        .header("content-type", "application/json")
        // The page is served from Vite on another port, so every call is
        // cross-origin. Loopback-only and test-only, hence the blanket allow.
        .header("access-control-allow-origin", "*")
        .header(
            "access-control-allow-headers",
            "content-type, x-gate-harness-token",
        )
        .body(Full::new(Bytes::from(v.to_string())))
        .unwrap()
}

/// Refuse to run without the seams that keep this off the developer's real
/// machine, and without the token that keeps other pages off this port.
///
/// A panic rather than a warning: every one of these is the difference between
/// a hermetic test and a loopback port that drives the real thing, and a
/// harness that started anyway would be found out by what it broke.
fn required_env(name: &str) -> String {
    match std::env::var(name) {
        Ok(v) if !v.is_empty() => v,
        _ => {
            eprintln!(
                "ui-harness: {name} is required. This example is started by \
                 `ci/e2e/ui-harness.sh`, which sets it along with a throwaway \
                 home; running it by hand would point every command at your \
                 real home, keychain and gateway."
            );
            std::process::exit(2);
        }
    }
}

/// Follow the process that started this one down, when asked to.
///
/// `GATE_UI_HARNESS_EXIT_WITH_PID` is set by `ci/e2e/ui-harness.sh` on Windows
/// only, to the script's own Windows PID. Playwright ends a webServer with
/// `taskkill /T /F` on the shell it spawned and then waits for the child's
/// stdout/stderr pipes to close, and `/T` walks parent PIDs. The script now
/// `exec`s this binary so it is created as the shell's child and inside that
/// tree; this is the second line, for the case where it still is not - a
/// Cygwin exec that leaves the harness with a dead parent - because the
/// alternative was measured: the harness outlived the kill holding the pipe,
/// and Playwright waited on it for 19 minutes until the job's own limit ended
/// the run. Unset, nothing is watched.
///
/// The PID is polled rather than the parent read, because on Windows the
/// parent recorded for this process may already be a dead stub at start.
fn exit_with_pid() {
    let Some(pid) = std::env::var("GATE_UI_HARNESS_EXIT_WITH_PID")
        .ok()
        .and_then(|v| v.parse::<u32>().ok())
        .filter(|p| *p > 0)
    else {
        return;
    };
    std::thread::spawn(move || {
        use sysinfo::{Pid, ProcessesToUpdate, System};
        let pid = Pid::from_u32(pid);
        let mut sys = System::new();
        loop {
            std::thread::sleep(std::time::Duration::from_secs(1));
            sys.refresh_processes(ProcessesToUpdate::Some(&[pid]), true);
            if sys.process(pid).is_none() {
                eprintln!("ui-harness: process {pid} is gone; exiting with it");
                std::process::exit(0);
            }
        }
    });
}

fn main() {
    // On Linux the engine does not live in this process: `manager_linux` spawns
    // `<current-exe> --proxy-helper` as a detached daemon that owns the loopback
    // listener and outlives the GUI. `src-tauri/src/main.rs` dispatches that for
    // the app, and the harness has to do the same or `current_exe` re-runs THIS
    // file - a second HTTP server on a taken port, and a helper that never comes
    // up. The symptom is `connect_tool` failing with "proxy helper did not come
    // up within ~2s", which names the daemon and not the binary behind it.
    #[cfg(target_os = "linux")]
    if std::env::args().skip(1).any(|a| a == "--proxy-helper") {
        if let Err(e) = gate_connect_core::proxy::helper::run_daemon() {
            eprintln!("gate proxy helper exited: {e}");
            std::process::exit(1);
        }
        return;
    }

    let app = mock_builder()
        // The one plugin a *command* reaches: `launch_at_login_status` and
        // `set_launch_at_login` go through `ManagerExt`, and Tauri panics with
        // "state() called before manage()" if it is absent - which surfaced as
        // an unexplained rejection on the app's very first boot read. The
        // others the frontend touches (updater, opener, process) are answered
        // in the page by `e2e/live/install.ts` and are not registered here.
        //
        // Note what this makes possible: `set_launch_at_login` would write a
        // REAL login item on the machine running the harness. Nothing drives it
        // today; a test that wants to should assert against the plugin's own
        // read rather than clicking that switch.
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            Some(vec!["--silent"]),
        ))
        // The one seam that makes this worth building: the app's own table,
        // not a copy of it.
        .invoke_handler(gate_connect_desktop_lib::invoke_handler())
        .build(mock_context(noop_assets()))
        .expect("mock app");

    // `drain_backend_errors` takes a `Window`, and several commands ask the
    // manager for one by label, so the harness needs a webview to dispatch
    // against. It renders nothing.
    let webview = WebviewWindowBuilder::new(&app, "main", Default::default())
        .build()
        .expect("mock webview");

    let log = Arc::new(Mutex::new(EventLog::default()));
    for name in EVENTS {
        let log = log.clone();
        let name = *name;
        app.listen_any(name, move |event| {
            let payload: serde_json::Value =
                serde_json::from_str(event.payload()).unwrap_or(serde_json::Value::Null);
            log.lock().unwrap().events.push(serde_json::json!({
                "event": name,
                "payload": payload,
            }));
        });
    }

    let token = required_env("GATE_UI_HARNESS_TOKEN");
    for seam in ["GATE_CONNECT_TEST_HOME", "GATE_CONNECT_TEST_SECRETS"] {
        let _ = required_env(seam);
    }
    exit_with_pid();

    let port: u16 = std::env::var("GATE_UI_HARNESS_PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(5610);

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("runtime");

    rt.block_on(async move {
        let addr = SocketAddr::from(([127, 0, 0, 1], port));
        let listener = tokio::net::TcpListener::bind(addr).await.expect("bind");
        // Playwright's `webServer` waits for this line's port to answer; the
        // print is what a failed run has to read.
        println!("ui-harness listening on http://{addr}");

        loop {
            let (stream, _) = match listener.accept().await {
                Ok(v) => v,
                Err(e) => {
                    eprintln!("accept failed: {e}");
                    continue;
                }
            };
            let webview = webview.clone();
            let log = log.clone();
            let token = token.clone();
            tokio::spawn(async move {
                let service =
                    service_fn(move |req| handle(req, webview.clone(), log.clone(), token.clone()));
                if let Err(e) = http1::Builder::new()
                    .serve_connection(TokioIo::new(stream), service)
                    .await
                {
                    // A page that navigates or closes drops its in-flight
                    // `/events` long-poll, which is this error and is not worth
                    // printing three times per test. Anything else is.
                    if !e.is_incomplete_message() {
                        eprintln!("connection error: {e}");
                    }
                }
            });
        }
    });
}

async fn handle(
    req: Request<hyper::body::Incoming>,
    webview: tauri::WebviewWindow<MockRuntime>,
    log: Arc<Mutex<EventLog>>,
    token: String,
) -> Result<Response<Full<Bytes>>, Infallible> {
    let path = req.uri().path().to_string();
    let query = req.uri().query().unwrap_or("").to_string();

    // The preflight carries no custom header by definition, so it is answered
    // before the check; it reveals nothing and grants nothing.
    if req.method() == Method::OPTIONS {
        return Ok(json(StatusCode::NO_CONTENT, serde_json::Value::Null));
    }

    // Every other route, including /health, which reports where the seams put
    // the developer's throwaway home.
    let presented = req
        .headers()
        .get("x-gate-harness-token")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default();
    if presented != token {
        return Ok(json(
            StatusCode::FORBIDDEN,
            serde_json::json!({ "err": "bad or missing x-gate-harness-token" }),
        ));
    }

    if path == "/health" {
        // The seam paths come from here rather than being recomputed by the
        // test, because they must agree exactly and one of them is length-
        // critical: the Linux helper daemon binds `$HOME/run/gate-connect/
        // proxyd.sock`, and a Unix socket path over ~108 bytes fails to bind
        // with nothing but "binding <path>" to say so. Letting the harness
        // report where it actually put things is what keeps a longer checkout
        // path from silently breaking the suite.
        let env = |k: &str| std::env::var(k).unwrap_or_default();
        return Ok(json(
            StatusCode::OK,
            serde_json::json!({
                "ok": true,
                "home": env("GATE_CONNECT_TEST_HOME"),
                "secrets": env("GATE_CONNECT_TEST_SECRETS"),
                "capture": env("GATE_UI_HARNESS_CAPTURE"),
            }),
        ));
    }

    if path == "/events" {
        let since: usize = query
            .split('&')
            .find_map(|kv| kv.strip_prefix("since="))
            .and_then(|v| v.parse().ok())
            .unwrap_or(0);
        // A `since` past the end can only come from a client that is out of
        // step with a restarted harness. Answering with the current offset
        // resynchronises it; parking for the full second on every poll, which
        // is what the loop below would do, looks like a hang.
        {
            let l = log.lock().unwrap();
            if since > l.events.len() {
                return Ok(json(
                    StatusCode::OK,
                    serde_json::json!({ "next": l.events.len(), "events": [] }),
                ));
            }
        }
        // Long-poll: up to ~1s in 20ms slices.
        for _ in 0..50 {
            let out = {
                let l = log.lock().unwrap();
                (l.events.len() > since).then(|| l.events[since..].to_vec())
            };
            if let Some(events) = out {
                return Ok(json(
                    StatusCode::OK,
                    serde_json::json!({ "next": since + events.len(), "events": events }),
                ));
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        return Ok(json(
            StatusCode::OK,
            serde_json::json!({ "next": since, "events": [] }),
        ));
    }

    if path != "/invoke" || req.method() != Method::POST {
        return Ok(json(
            StatusCode::NOT_FOUND,
            serde_json::json!({ "err": "no such route" }),
        ));
    }

    let body = match req.into_body().collect().await {
        Ok(b) => b.to_bytes(),
        Err(e) => {
            return Ok(json(
                StatusCode::BAD_REQUEST,
                serde_json::json!({ "err": format!("body: {e}") }),
            ))
        }
    };
    let parsed: serde_json::Value = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(e) => {
            return Ok(json(
                StatusCode::BAD_REQUEST,
                serde_json::json!({ "err": format!("json: {e}") }),
            ))
        }
    };
    let Some(cmd) = parsed.get("cmd").and_then(|c| c.as_str()).map(String::from) else {
        return Ok(json(
            StatusCode::BAD_REQUEST,
            serde_json::json!({ "err": "missing cmd" }),
        ));
    };
    let payload = parsed
        .get("payload")
        .cloned()
        .unwrap_or(serde_json::Value::Object(Default::default()));
    // For the log line below: the response consumes `cmd`.
    let cmd_name = cmd.clone();

    // `get_ipc_response` parks the calling thread on a channel until the
    // command resolves, so it cannot run on a reactor thread: a blocking
    // command would otherwise stall every other connection, including the
    // `/events` poll the page is waiting on.
    let result = tokio::task::spawn_blocking(move || {
        tauri::test::get_ipc_response(
            &webview,
            InvokeRequest {
                cmd,
                callback: CallbackFn(0),
                error: CallbackFn(1),
                url: if cfg!(windows) {
                    "http://tauri.localhost"
                } else {
                    "tauri://localhost"
                }
                .parse()
                .unwrap(),
                body: InvokeBody::Json(payload),
                headers: Default::default(),
                invoke_key: INVOKE_KEY.to_string(),
            },
        )
    })
    .await;

    let response = match result {
        Ok(Ok(body)) => {
            let v = match body {
                InvokeResponseBody::Json(s) => {
                    serde_json::from_str(&s).unwrap_or(serde_json::Value::Null)
                }
                InvokeResponseBody::Raw(b) => serde_json::json!(b),
            };
            serde_json::json!({ "ok": v })
        }
        // A command that returned `Err` is a normal outcome the UI renders, so
        // it comes back as a 200 with `err` rather than an HTTP error: the page
        // has to see the same rejection shape Tauri gives it.
        //
        // Said on stderr as well, because the app swallows most of these by
        // design - a failed `proxy_status` read becomes a missing master
        // switch, and nothing on the page says why. Playwright pipes this
        // process's stderr into the run log, so this is the one place a spec's
        // reader can learn what the backend actually answered.
        Ok(Err(e)) => {
            eprintln!("ui-harness: {cmd_name} rejected: {e}");
            serde_json::json!({ "err": e })
        }
        // A command that PANICKED lands here, not above: `get_ipc_response`
        // itself panics when the responder is dropped, which `spawn_blocking`
        // reports as a join error. Saying so matters because the causes are
        // different - a panic is usually a Tauri plugin this harness does not
        // register (`state() called before manage()`), and reporting it as an
        // ordinary rejection sends the reader looking in the command instead.
        Err(e) if e.is_panic() => serde_json::json!({
            "err": format!(
                "command panicked (often a plugin the harness does not register): {e}"
            ),
        }),
        Err(e) => serde_json::json!({ "err": format!("harness join error: {e}") }),
    };
    Ok(json(StatusCode::OK, response))
}
