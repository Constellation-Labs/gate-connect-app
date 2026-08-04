# Security Review: `quit-integrations-warning` (vs `main`)

Lens: security only. Scope: new Tauri command surface (`quit_app`,
`disconnect_tools_for_quit`, `request_quit`), the `quit-requested` event
payload rendered by `QuitConfirm`, and secret/credential leakage into logs,
analytics, notifications, and error strings.

## Summary

The change is low-risk. The two new webview-invokable commands add no
privilege beyond what the webview already holds: `disconnect_tools_for_quit`
is a strict subset of the already-exposed `proxy_disable` (which calls the
same `snapshot_and_disable_all` plus full proxy teardown,
`src-tauri/src/lib.rs:677`), and `quit_app` is DoS-equivalent to the
`process:allow-restart` permission already granted in
`src-tauri/capabilities/default.json:13`. The event payload rendered in the
DOM comes from compile-time `&'static str` registry names, not user or file
input. Nothing secret enters logs, analytics, notifications, or error
strings. `crates/core/src/proxy/manager.rs` and
`crates/core/src/proxy/system_proxy.rs` changes are doc comments only
(verified: no non-comment lines in the diff).

## Findings

### L1 (L): `quit_app` lets any script in the webview terminate the app unconditionally

`src-tauri/src/lib.rs:1082-1085` — `quit_app` is a plain
`app.exit(0)` with no state check, registered in both `invoke_handler`
lists. A compromised webview could kill the app silently, taking the local
relay down while CLI tool configs still point at it (availability impact
only).

Why this is acceptable as-is:

- The webview already holds `process:allow-restart`
  (`src-tauri/capabilities/default.json:13`) and
  `core:window:allow-close`, so process-lifecycle control from the webview
  is an existing, accepted capability; `quit_app` does not widen it.
- `app.exit(0)` in Tauri v2 runs the `RunEvent::Exit` handler
  (`src-tauri/src/lib.rs:1845-1848`), which calls
  `proxy::manager().disable_quiet()` — the OS system proxy is reverted, so
  an abusive quit cannot strand the machine's proxy settings pointing at a
  dead listener. This matches the pre-branch tray behavior (`app.exit(0)`
  at the old `"quit"` arm, now `request_quit(app)` at
  `src-tauri/src/lib.rs:1722`).
- The webview loads only bundled local content; there is no remote-content
  vector into this webview on this branch.

No action required. If the command surface is ever hardened wholesale,
gating lifecycle commands to the `main` window would be the place to do it,
but that is not specific to this branch.

### L2 (L): `disconnect_tools_for_quit` is callable outside the quit flow

`src-tauri/src/lib.rs:1094-1112` — the command is not tied to a pending
`request_quit`; the webview can invoke it any time and disable every
enabled provider (rewriting CLI tool config files via
`snapshot_and_disable_all`, `crates/core/src/provider.rs:406-420`).

Not a real widening: `provider_disable` (per-slug) and `proxy_disable`
(same snapshot+disable-all plus proxy stop and intent clearing,
`src-tauri/src/lib.rs:667-690`) are already exposed to the webview, so a
compromised webview could already do strictly more. The snapshot mechanism
also means the effect is reversible on next startup restore. Per-provider
failures inside `snapshot_and_disable_all` are `eprintln!`-only and do not
propagate into the returned error string
(`crates/core/src/provider.rs:415-418`). No action required.

### L3 (L): raw backend error chain rendered in the popover

`src/components/QuitConfirm.tsx:37` — on disconnect failure the raw
stringified error (`format!("{e:#}")` from
`src-tauri/src/lib.rs:1098-1101`, at most anyhow context with snapshot
file paths, e.g. a home-directory path) is set into local state and
rendered at `QuitConfirm.tsx:68` as a React text node.

- No XSS: React escapes text children; no `dangerouslySetInnerHTML`
  anywhere in the component.
- No exfiltration: the raw string is displayed locally only. Analytics get
  the *classified* title, not the raw string — `trackError`
  (`src/lib/analytics.ts:114-119`) sends `classifyError(...).title`
  ("Couldn't turn off integrations", `src/lib/errors.ts`) and event props
  pass the allowlist.
- Consistent with existing screens (`src/screens/Settings.tsx:146`,
  `src/screens/FirstRun.tsx:59`, `src/screens/OrgPicker.tsx:39` all render
  `String(err)` locally).

Nit only; no change requested.

### Verified non-findings

- **Event payload injection**: the `quit-requested` payload
  (`src-tauri/src/lib.rs:1064-1076`) is built from
  `integ.display_name()`, which is `&'static str` on every integration
  (`crates/core/src/integrations/{claude_code,codex,opencode,openclaw,hermes}.rs`)
  — compile-time constants, never file/user-derived. `App.tsx:151-156`
  passes it to `QuitConfirm`, where `joinNames` output is rendered as
  escaped React text. No injection path.
- **Analytics leakage**: new events `quit_warning_shown` (`tool_count`, a
  number, `src/App.tsx:302`) and `quit_confirmed`
  (`integrations_disabled`, a boolean, `QuitConfirm.tsx:31,44`) carry no
  tool names, hosts, or paths; both prop keys were added to
  `ALLOWED_PROP_KEYS` (`src/lib/analytics.ts:63-64`) and everything else is
  stripped by `sanitize`.
- **Notification content**: the quit notification body
  (`src-tauri/src/lib.rs:1102-1112`) is a fixed static string — no tool
  names, keys, hosts, or paths.
- **Logging**: no new log lines emit secrets; `snapshot_and_disable_all`
  logs provider slugs only.
- **request_quit trigger**: reachable only from the tray menu handler
  (`src-tauri/src/lib.rs:1722`), not from the webview; its config-file
  status probes reuse the existing `registry()` status path already used
  by the tools list.

## Verdict

**Low risk — no must-fix or should-fix findings.** The new command surface
is a subset of the webview's existing blast radius (`proxy_disable`,
`provider_disable`, `process:allow-restart`), the exit path still reverts
the system proxy via the `RunEvent::Exit` handler, DOM-rendered data is
compile-time constant and React-escaped, and the analytics/notification
additions carry no sensitive values. Ship as-is from a security
standpoint.
