# Live UI e2e

The real frontend, driven by clicks, against the **real Rust command table**.

```
Playwright (Chromium, 1280x800)        cargo run --example ui-harness
  real bundle, real NewUiApp             tauri::test mock runtime
  __TAURI_INTERNALS__.invoke  ──POST──►  the app's own invoke_handler()
                              ◄──poll──  every event it emits
                                                  │
                                                  ▼
                                         gate-connect-core: the real
                                         engine, relay, config writes,
                                         on a throwaway $HOME
```

## Why it exists

The other two suites each cover one half and neither covers the join:

| | frontend | backend | the join |
|---|---|---|---|
| `e2e/*.spec.ts` | real | **fake** (`install.ts`) | asserts a command was *called* |
| `ci/e2e/run.sh` | **none** | real, real CLIs, real relay | asserts the gateway *received* it |
| this suite | real | real | asserts the click did it |

"Turning the switch on actually routes traffic" is the question a user asks,
and before this nothing answered it.

## Why not drive the shipped app

A WebDriver would be the obvious way, and macOS has none: WKWebView implements
no `webdriver` protocol, so a GUI-driving suite would cover Linux and Windows
and miss the platform with the most platform-specific behaviour. Driving the
real binary would also want a display, a tray icon, an updater, and an autostart
plugin that writes a real login item on the runner.

`tauri::test::mock_builder` takes the same `invoke_handler` the app registers
(`gate_connect_desktop_lib::invoke_handler`, shared rather than copied) and
answers IPC with no window and no plugins, on every platform. So `ui-e2e` in
`ci.yml` is a real three-OS matrix.

## What it does not cover

- **Rendering.** The browser is Chromium, never WKWebView / WebView2 /
  WebKitGTK. This proves interaction and wiring; how any of it paints is
  covered by nothing, same as the popover suite.
- **The tray, window lifecycle, OS trust dialogs, the updater.** No window
  exists. `quit-requested` and friends can still be exercised, because the
  backend emits them and `/events` replays them.
- **The keychain.** `GATE_CONNECT_TEST_SECRETS` is set, for the reason
  CLAUDE.md gives: a real keychain read would prompt per rebuild on macOS.
  Nothing green here says anything about `keychain.rs`.
- **`APP_HANDLE`.** It is a `OnceLock<AppHandle<Wry>>` in `lib.rs` that this
  runtime cannot fill, so the few backend-initiated emits routed through it are
  inert. Commands that emit through their own `app` argument work normally.

## The routing arc, and its one opt-in

`routing.spec.ts` drives the thing the harness exists for: switch an app on, a
real request reaching the mock gateway with the credential injected, switch it
off, nothing of ours left in its config, and what Gate leaves behind when it is
gone. It needs
`GATE_UI_HARNESS_ROUTING=1`, which means one thing - **this run may install a
root CA on this machine**, because `manager::enable()` calls
`ca::ensure_trusted()`. CI sets it and removes the root afterwards. Without it
the arc skips and the boot specs still run.

Two things it exploits, both worth knowing before writing another one:

- **Codex needs no install.** `detect()` falls back to `$HOME/.codex` existing
  and `connect()` reads `auth.json` for its auth mode, so two files in the
  throwaway home are a "logged-in, installed" Codex. It also routes through the
  loopback relay, so a plain `fetch` from Node is the exact shape of its own
  request. Claude Code would need `HTTPS_PROXY` plus a CA inside Node's bundle.
- **There is no off switch for the engine.** Since #317 routing runs for as
  long as the app is open, so the arc never asserts that a port closes or that
  a request stops being forwarded: it once clicked a master switch and asserted
  both, and that switch is gone. "Off" for one app is its config no longer
  naming the relay, read from disk and from `list_tools`. (For the record, the
  port measurement that used to live here: after an engine stop Linux's helper
  daemon keeps the listener and answers 502 in pass-through, while macOS and
  Windows refuse the connection.)

## Shape of a spec

One harness process per run, one throwaway home, wiped by
`ci/e2e/ui-harness.sh` at startup. State accumulates across tests on purpose -
the project is serial and specs read as a sequence, like `ci/e2e/run.sh` and
like a person. Arrange and read back through `harness.invoke`, which asks the
backend directly rather than trusting the UI's account of itself.

## Running

```sh
pnpm test:e2e:live                          # this suite
GATE_UI_HARNESS_ROUTING=1 pnpm test:e2e:live  # ...including the routing arc
```

`pnpm test:e2e` runs the popover suite and **not** this one. They are separate
config files (`playwright.live.config.ts`) rather than two projects in one,
because Playwright resolves `webServer` per config and never per project: as a
project, this suite's Rust harness started on every popover run too, including
the CI job that installs no Rust.

## The port is a remote control, and it is guarded

`/invoke` reaches every command the app has, several of which leave the
throwaway home: a real login item, a machine-wide certificate, the system proxy,
a process sweep that kills agents. Any page in the developer's browser can post
to loopback, so every route requires `x-gate-harness-token`, minted per run in
`playwright.live.config.ts`. The harness also refuses to start without
`GATE_CONNECT_TEST_HOME` and `GATE_CONNECT_TEST_SECRETS`, since without them
those same commands would drive the real machine.
