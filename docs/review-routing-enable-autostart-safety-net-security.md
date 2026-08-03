# Security review — `feat/routing-enable-autostart-safety-net`

Range: `v0.1.1...HEAD`. Lens: **security only**.

## Summary

The diff is security-clean on its highest-risk surfaces. The helper daemon's
control channel keeps its real access controls intact (owner-UID check via
`SO_PEERCRED`, per-run `0600` token, `0700` socket dir, catalog-constrained
intercept); the new `BUILD_FINGERPRINT` is correctly scoped as a
change-detector, not an auth boundary (`crates/core/build.rs:11`,
`crates/core/src/proxy/control.rs:102`). The CA-handling change in `ca.rs` only
drops the `-t` flag from a System-keychain delete built entirely from
constants, so no new injection surface. The analytics seam preserves its
privacy posture: the new backend-error path buffers raw messages **locally**
and the frontend classifies them so only the vetted title/context ever leave
the machine (`src/lib/analytics.ts:110`, `src/App.tsx:61`,
`src/lib/errors.ts:41`). The new autostart/port-persistence markers are
non-secret files under the per-user support dir. No High findings. What
remains are a few Low-severity hardening notes; the most notable is the
name-only process matching in `close_running_agents` (mitigated by an inline
confirmation).

## Findings

### High
None.

### Medium
None.

### Low

- **`close_running_agents` matches processes by bare name across all users**
  — `src-tauri/src/lib.rs:319` / `src-tauri/src/lib.rs:329`. It iterates every
  process (`ProcessesToUpdate::All`), lowercases the name, strips `.exe`, and
  SIGTERMs (or hard-kills on Windows) anything named `claude`/`codex`/
  `opencode`. A user's own unrelated process that happens to share one of
  these names is killed too. Impact is bounded: the kernel prevents signalling
  other users' processes (best-effort, non-error), the action is user-initiated,
  and it is gated behind an inline "Anything they're working on will be
  interrupted" confirm (`src/components/StartupRoutingNotice.tsx:62`). It is a
  data-loss footgun rather than a privilege issue; matching on the executable
  path of the known CLIs would tighten it.

- **Untrusted release body flows into the Slack workflow payload** —
  `.github/workflows/release-notes-slack.yml:33`, interpolated at
  `.github/workflows/release-notes-slack.yml:96` (`payload: ${{ steps.payload.outputs.payload }}`).
  `release.body` (derived from auto-generated notes / PR titles, i.e.
  contributor-influenced) is transformed by `toMrkdwn` and emitted via
  `JSON.stringify`. The `&<>` escaping runs before link-markup substitution and
  the value is consumed by a JS action's input (not a shell), so this is not a
  command-injection vector; worst case is cosmetic Slack-mrkdwn mischief in the
  release-notes post. Workflow permissions are correctly minimized
  (`contents: read`, line 13) and the `workflow_dispatch` `tag` input is used
  only as an argument to `getReleaseByTag`. Low / informational.

- **Privileged-command stderr surfaced into the user-facing error string** —
  `crates/core/src/primitives.rs:154`. `run_as_admin` now captures osascript
  stderr and embeds it verbatim in the returned error
  (`osascript command exited non-zero: {stderr}`). That error becomes the
  `invoke` rejection whose `raw` is shown in the UI `<details>`
  (`src/lib/errors.ts:48`). Only the *classified* title is ever sent to
  analytics, so this is not a telemetry leak; it is a minor local
  information-exposure note (the underlying command is `security
  delete-certificate` on constants, so nothing sensitive is expected in stderr).

## Notes / non-findings (verified safe)

- **Helper control channel auth unchanged and intact** —
  `crates/core/src/proxy/helper.rs:137` (UID check before any byte),
  `helper.rs:151` (token match in `Hello`), `control.rs:9` (socket/dir modes).
  The added `fingerprint` field is compared client-side only to decide
  daemon reuse-vs-replace (`helper_client.rs:112`); it is not an
  authentication factor and its absence defaults to "incompatible".
- **`BUILD_FINGERPRINT` via `build.rs`** — FNV-1a over `src/` + crate version,
  explicitly documented as a change detector, not a security boundary
  (`crates/core/build.rs:11`). Deterministic over source, dependency-free. Fine.
- **Domain catalog validation preserved** — `control.rs:176`
  (`validate_domains`) still rejects any slug/hosts/upstream not exactly in the
  built-in catalog, so an authenticated caller still cannot point the MITM CA
  at an arbitrary host.
- **`ca.rs` System-keychain delete** — `crates/core/src/proxy/ca.rs:194`; the
  `security` command string is built from `CA_COMMON_NAME` and
  `SYSTEM_KEYCHAIN` constants via `sh_quote`, no user input. Dropping `-t` only
  changes which trust settings are removed, not the injection surface.
- **Backend-error analytics path** — raw `format!("{e:#}")` chains are buffered
  in `PENDING_BACKEND_ERRORS` (bounded to 32, `src-tauri/src/lib.rs:292`),
  handed to the frontend via `drain_backend_errors`, then run through
  `classifyError`; only `{context, title}` are captured
  (`src/lib/analytics.ts:113`). The `backendErrorContext` allowlist
  (`src/lib/errors.ts:32`) degrades an unknown backend-supplied context to
  `generic` rather than forwarding an unvetted label. Raw messages never leave
  the machine.
- **Analytics prop allowlist** — new keys (`routing_on`, `provider_count`,
  `launch_at_login`, `count`, `step`, …) are coarse/non-identifying and pass
  through the existing `sanitize` allowlist (`src/lib/analytics.ts:47`); no
  host/key/path key was added.
- **Port / PAC-port / autostart marker files** — non-secret integers and a
  `"1"` flag under the per-user `app_support_dir`, written `0644`
  (`system_proxy.rs`, `autostart_optout.rs`). Parsed as `u16` with graceful
  fallback; no deserialization or trust-boundary risk.
- **Release workflow secrets handling** — `.github/workflows/release.yml`
  passes `APPLE_*`, `TAURI_SIGNING_*`, `VITE_POSTHOG_KEY`, and
  `SLACK_WEBHOOK_URL` via `env:`/`with:` from `secrets.*`; none are echoed to
  logs. The `.p8` is base64-decoded to a `$RUNNER_TEMP` file (line 65). No
  change here weakens secret handling.
