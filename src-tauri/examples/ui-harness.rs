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
//! Secrets, home and the gateway all come from the existing env seams
//! (`GATE_CONNECT_TEST_SECRETS`, `GATE_CONNECT_TEST_HOME`, `GATE_CONNECT_TEST_CA`,
//! `GATE_CONNECT_TEST_UPSTREAM`), so this touches no real keychain and no real
//! gateway. It is a dev-dependency example, never part of a shipped build.
//!
//! Wire protocol, all on loopback:
//!   POST /invoke   {"cmd": "...", "payload": {...}}  -> {"ok": <json>} | {"err": <json>}
//!   GET  /events?since=N                             -> {"next": M, "events": [...]}
//!   GET  /health                                     -> {"ok": true}
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
        .header("access-control-allow-headers", "content-type")
        .body(Full::new(Bytes::from(v.to_string())))
        .unwrap()
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
            tokio::spawn(async move {
                let service = service_fn(move |req| handle(req, webview.clone(), log.clone()));
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
) -> Result<Response<Full<Bytes>>, Infallible> {
    let path = req.uri().path().to_string();
    let query = req.uri().query().unwrap_or("").to_string();

    if req.method() == Method::OPTIONS {
        return Ok(json(StatusCode::NO_CONTENT, serde_json::Value::Null));
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
        Ok(Err(e)) => serde_json::json!({ "err": e }),
        Err(e) => serde_json::json!({ "err": format!("harness join error: {e}") }),
    };
    Ok(json(StatusCode::OK, response))
}
