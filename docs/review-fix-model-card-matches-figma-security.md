# Security review: PR #208 `fix/model-card-matches-figma`

Base `feat/new-app-ui`, head `fix/model-card-matches-figma` (+2343/-280).
Lens: security. Reviewed with `git diff feat/new-app-ui...HEAD`.

## Verdict

No high-severity findings. The proxy change is the only part of this diff with
real security weight, and its core decisions are made the safe way: `serve_path`
is an exact-match allowlist that returns `&'static str`, so no caller-supplied
bytes reach the rewritten path; `GATE_MODEL_HEADER` is removed unconditionally
before it is stamped, so a local process cannot elect itself into served
(org-billed) mode by sending the header; and on a served rewrite the upstream
hint is *removed* rather than merely left unwritten, so BYOK cannot be smuggled
back in to aim the gateway at a host of the caller's choosing. The client's own
provider bearer is stripped on served requests in both the MITM and relay paths,
and the new integration tests assert exactly that.

Two things should be tightened. First, this PR creates a wire combination that
could not previously exist: a request that carries `x-gate-model` ("Gate serves
this, bill the org") *and* `x-gate-upstream-url` ("forward this to my own
provider under my own key"), because a Gate model on an unservable path now
falls back to BYOK instead of always being served. Second, `strip_client_auth`
covers only `authorization` / `x-api-key`, so a served ChatGPT-route request
re-aimed at the gateway still carries the client's `cookie` header, and for the
ChatGPT app class a Gate-injected `cf_clearance` is added *after* the rewrite,
keyed off the original host. That route previously hung, so the exposure was
inert; this PR makes it complete.

The dev-secrets surface is clean: `GATE_CONNECT_TEST_SECRETS` is honoured only
in debug builds, the store lives outside the repo, and the `.gitignore`
additions add no secret path. No new Tauri commands and no new devtools globals
are introduced; the `src-tauri/src/lib.rs` change is documentation only.

## Findings

### M - `x-gate-model` survives the fallback to BYOK, so one request carries two contradictory intents

`crates/core/src/proxy/engine.rs:1206-1237`, `crates/core/src/proxy/relay.rs:577-589`

When the user has put a tool on a Gate model but the request path is not
servable, `model_serve_path` is `None`, so both paths take the BYOK branch and
set `x-gate-upstream-url`. Nothing removes `x-gate-model`, which
`inject_attribution` has already stamped. The header's own contract
(`crates/core/src/proxy/mod.rs:855-860`) is that it is not a label: "it
**changes what the gateway serves** - so the gateway rewrites the body's `model`
to the first entry". A request that says both "rewrite my model to
`openai/gpt-5`" and "forward me to `https://api.anthropic.com`" is asking the
gateway to send the user's prompt to the user's own provider account under the
user's own key, at a model id that provider never offered.

Before this PR the combination was unreachable: presence of `x-gate-model` was
the whole serve switch, so the upstream hint was always removed when it was set.
The new path condition introduces it.

Reachable in normal use, not only in edge cases: `serve_path` deliberately
returns `None` for `/v1/messages/count_tokens`
(`crates/core/tests/model_choice_injection.rs:454`), which Claude Code calls
routinely, and for OpenCode's `/zen/v1/…` shapes.

The new tests cover served (`…:440-445`) and no-Gate-model
(`…:447-473`) but not "Gate model set, path unservable", which is the case that
produces this pairing.

Fix shape: in the BYOK branch, remove `GATE_MODEL_HEADER` alongside setting the
upstream hint, so the wire carries one intent. Both call sites need it, since
the two paths are documented as having to agree.

### M - served requests keep the client's chatgpt.com cookies, and a `cf_clearance` is injected after the rewrite

`crates/core/src/proxy/mod.rs:1152-1176`, `crates/core/src/proxy/engine.rs:820-900`

`strip_client_auth` removes exactly two headers: `CLIENT_AUTH_HEADERS =
["authorization", "x-api-key"]` (`mod.rs:1158`). Cookies are not in that list.
On a served rewrite the request's authority has already been swapped to the
gateway (`engine.rs:1155-1160`, `1212-1228`), and any `cookie` header the client
sent for the provider host travels with it.

Worse, the chatgpt.com block runs *after* `apply_rewrite` and keys off the host
captured before the rewrite (`engine.rs:838`), so for
`client == ClientClass::App` a captured clearance is merged into the cookie
header of a request that is now addressed to the gateway
(`engine.rs:897-900`, injector at `engine.rs:1105-1129`). `cf_clearance` is
session material minted for a third party (Cloudflare/OpenAI) under the user's
own UA; the gateway has no use for it, and it lands wherever the gateway logs
request headers.

This ordering pre-dates the PR, but the exposure was inert because
`/backend-api/codex/responses` with the hint withheld hung and never completed -
that is the bug this PR fixes (`mod.rs:958-979`). Making the route work makes the
cookie forwarding live.

Exposure is to the first-party gateway, not a third party, which is why this is
M and not H.

Fix shape: either add `cookie` to the served-request strip, or gate the
`cf_clearance` injection on the request not having been served (the block
already computes `let rewritten = action == "rewrite->gateway"`, so the
information is at hand).

### L - the serve rewrite widens the UA-spoofable spend surface by one route

`crates/core/src/proxy/mod.rs:1066-1080`

`client_tool` identifies the sending tool from a substring match on
`User-Agent`, and that identity is what selects which stored model set is
stamped, hence whether the request is served on the org's balance. Any local
process can present `codex` in its UA. This is not new and is the accepted
loopback posture (`docs/security-notes-loopback.md`); noted only because
`serve_path` adds `/codex/responses` and `/backend-api/codex/responses` to the
set of paths on which that spoof now results in a completed, org-billed request
rather than a hang.

Note also that the ChatGPT desktop app's UA is documented in this same file as
the browser UA with `CodexBrowser ` prefixed (`engine.rs:862-870`), so it
matches the `codex` needle: app traffic is attributed to, and takes the model
choice of, the `codex` slug. Its own paths are not in `serve_path`, so it is not
served today - but it is one `serve_path` entry away from being served under a
model the user chose for a different tool.

### L - `adaptBillingUrl` accepts `http:` as well as `https:`

`src/lib/toolModels.ts:381-391`

The scheme check is the right instinct and the tests cover `javascript:`,
`file:` and garbage (`src/lib/toolModels.test.ts:261-267`). It still admits
plain `http:`, so a gateway response can send the user to a cleartext host
through the system opener. Tightening to `https:` costs nothing real.

### L - dev secret files are mode 0644 inside the 0700 directory

`ci/dev-app.sh:46-47`, `crates/core/src/keychain.rs:82-88`

The script chmods the store directory to 700 unconditionally, including a
custom `GATE_CONNECT_DEV_SECRETS` path, so the practical protection is right.
The files themselves are written with `std::fs::write` defaults (0644), so they
rely entirely on the directory mode. Worth a `set_permissions(0o600)` on the
seam write if this script becomes the standard dev loop.

### L - the dev script will point the app at any URL, including plain http

`ci/dev-app.sh:43,53-54`

`$1` flows into `VITE_GATE_DEFAULT_BASE_URL` unvalidated, and the Gate key in
the file-backed store is then sent to whatever that names. The default is
staging and the header comment is explicit about using a staging key, which is
the correct handling for a dev-only script; recorded for completeness rather
than as a change request.

## Surfaces checked and found clean

- **`serve_path` injection safety** (`mod.rs:969-982`): exact `match` on five
  literals returning `&'static str`. No traversal (`/v1/messages/../…` simply
  fails to match and falls back to BYOK, the fail-safe direction), no CRLF, no
  attacker bytes in the output. The preserved query comes from a
  hyper-validated `PathAndQuery` and is re-parsed before being installed
  (`engine.rs:1216-1228`); in the relay it is re-parsed by reqwest when the
  target URL is built (`relay.rs:602-607`).
- **Client cannot force served mode**: `inject_model_choice` removes
  `GATE_MODEL_HEADER` before stamping it, and only stored user intent can
  populate it (`mod.rs:919-931`). Both paths read the header back *after*
  injection (`engine.rs:1206`, `relay.rs:577`), including on the
  caller-brought-its-own-Gate-key short-circuit, which happens after
  `inject_attribution` (`mod.rs:1114-1124`).
- **BYOK cannot be smuggled onto a served request**: the upstream hint is
  explicitly removed, not merely left unset (`engine.rs:1237-1245`,
  `relay.rs:590-593`), so a local process cannot aim the gateway at an
  arbitrary host on a served rewrite.
- **Cross-provider bearer forwarding**: the tool's own bearer /`x-api-key` are
  stripped on served rewrites in both paths, and the new tests assert both the
  strip and its absence on the BYOK case
  (`crates/core/tests/model_choice_injection.rs:440-473`).
- **Dev-secrets leaking into shipped builds**: `crate::env::test_seam` returns
  `None` in release builds and says so on stderr
  (`crates/core/src/env.rs:18-25`), so `GATE_CONNECT_TEST_SECRETS` is inert in
  a release binary. No `[profile.release]` override re-enables
  `debug-assertions` anywhere in the workspace.
- **Secrets reachable by `git add`**: the store is `~/.gate-connect-dev/secrets`,
  outside the tree (`ci/dev-app.sh:44`). The `.gitignore` additions are
  `feature-plans/` and a blank line only - no secret path is newly ignored, and
  nothing secret is newly trackable.
- **New Tauri IPC surface**: none. The `src-tauri/src/lib.rs` diff is 13 lines
  of doc comment on `running_agents`; the `only: Option<Vec<String>>` parameter
  already existed. Slugs are filtered through the static `AGENT_PROCESSES`
  allowlist (`src-tauri/src/lib.rs:1551-1568`), so no caller-supplied string
  reaches a process name that `close_running_agents` would signal, and an
  unknown slug narrows to nothing rather than widening to all.
- **New devtools globals**: none. `src/main.tsx` adds only `window`
  `error` / `unhandledrejection` listeners.
- **Secret leakage into logs and diagnostics**: the new listeners forward error
  text to PostHog (consent- and key-gated, `src/lib/analytics.ts:231-234`) and
  to the local log, which is off against a production gateway
  (`crates/core/src/logging.rs:77-92`) and carries an `sk-gw-` / `sk-ant-`
  scrub backstop (`logging.rs:100-115`). `describe` emits message + stack only
  (`src/lib/log.ts:42-52`). No credential, request body or prompt is logged
  anywhere in the diff, and no new `eprintln!` prints header values.
- **Untrusted gateway JSON crossing into the UI**: `adaptToolShapes`,
  `adaptModels` and `adaptCredits` type-check every field and degrade an
  unrecognised verdict to `unknown` rather than to a denial
  (`src/lib/toolModels.ts:135-200`, `:361-391`). No `dangerouslySetInnerHTML`,
  `innerHTML`, `eval` or `new Function` in the diff.
- **Test fixtures**: `e2e/backend.ts` and `e2e/new-ui-model-picker.spec.ts` add
  only synthetic values (`sk-gw-test`, `Bearer chatgpt-oauth-token`, a fixed
  install id). No real credentials committed.
