# Code-Quality / Maintainability Review - Gate Connect (main)

Scope: Rust core (`crates/core`, `crates/cli`), Tauri glue (`src-tauri`),
React/TS frontend (`src/`). Focus: duplication, simplification, Rust
idioms, consistency, testability. Correctness/security bugs excluded
(other reviewers cover those) unless incidental.

## Summary

The codebase is, on the whole, well-organized: the proxy domain catalog
(`proxy/mod.rs`) is cleanly data-driven, the `Integration` trait gives a
tidy per-tool boundary, and error handling via `anyhow` is consistent.
The highest-leverage issues are all **duplication that a small shared
helper or trait default would erase**, plus one **speculative
abstraction that is dead across all five integrations and four layers**.

Ranked leverage:

1. **High** - Dead "upstream credential" abstraction: 3 trait methods +
   1 default, reimplemented identically in all 5 integrations and wired
   through Tauri + FE, but no integration uses it and no FE screen calls it.
2. **High** - macOS and Windows proxy managers are ~70% byte-identical
   (211 shared lines); the OS-agnostic orchestration should be shared.
3. **High** - `state_path`/`load_state`/`save_state` sidecar helpers are
   copy-pasted identically across `opencode`, `openclaw`, `hermes`.
4. **Medium** - `map_err(|e| format!("{e:#}"))` repeated 39x in `lib.rs`;
   extract one helper / extension trait.
5. **Medium** - `App.tsx` re-inlines the same "refresh proxy+providers+tools"
   fetch trio in 4 places and `typeof e === "string" ? e : String(e)` twice.
6. **Medium** - Untested pure logic: `proxy/config.rs` (domain enable/merge),
   `registry::disconnect_all` aggregation, `ToolId::from_slug` round-trip.
7. **Low** - 26+ em-dash `—` usages violate the project's own CLAUDE.md
   rule ("never use —"), several in user-facing strings.

---

## Findings

### 1. [High] The "upstream credential" trait surface is dead across all implementations

`registry.rs:93-137` declares `requires_upstream_credential()` (default
`true`), `save_upstream_credential()`, `has_upstream_credential()`,
`clear_upstream_credential()`. Every one of the five integrations
implements the exact same trio:

- `requires_upstream_credential -> false` (claude_code:77, codex:191,
  opencode:229, hermes:84, openclaw:202)
- `has_upstream_credential -> Ok(true)`
- `clear_upstream_credential -> Ok(())`
- `save_upstream_credential -> anyhow::bail!("<Tool> does not need a
  separate upstream credential")`

(e.g. claude_code:275-289, codex:526-540, opencode:485-499,
hermes:299-313, openclaw:407-421).

Because `requires_upstream_credential()` is `false` everywhere, the CLI
branch that consumes it is unreachable (`crates/cli/src/main.rs:289`:
`if integ.requires_upstream_credential() && !integ.has_upstream_credential()?`),
and the three Tauri commands (`src-tauri/src/lib.rs:152,160,173`) plus
their `api.ts` bindings (`src/lib/api.ts:26-31`,
`hasUpstreamCredential`/`saveUpstreamApiKey`/`clearUpstreamCredential`)
are never called by any screen (`rg` in `src/` finds only the
definitions). ~75 lines of identical boilerplate + a dead 4-layer
command path.

**Suggested change:** Give the three methods no-op default bodies on the
trait (`has -> Ok(true)`, `clear -> Ok(())`, `save -> bail!("...")`) and
delete the five per-integration copies; a future credential-needing tool
overrides them. If the capability is not on the near roadmap, remove the
methods, the `requires_upstream_credential` default, the CLI branch, the
three Tauri commands, and the three `api.ts` exports outright. Either way
the "always-false" default should not read as `true`.

### 2. [High] macOS and Windows proxy managers duplicate ~70% of their body

`proxy/manager.rs` (296 lines, macOS) and `proxy/manager_windows.rs`
(278 lines) share 211 identical (whitespace-normalized) lines. The
process-global singleton, `list_domains`, `set_domain`, `refresh_api_key`,
`handle_engine_crash`, and most of the enable/disable/status engine
lifecycle are the same; both drive an **in-process** `engine`. The only
real divergence is macOS enumerating `active_services()` and passing them
to the system-proxy calls where Windows has a single global scope.
(`manager_linux.rs` genuinely differs - it drives an out-of-process
helper client - so it stays separate.)

**Suggested change:** Extract the OS-agnostic orchestration (engine
lifecycle, config toggling, singleton, `list_domains`/`set_domain`/
`refresh_api_key`/`handle_engine_crash`) into one shared struct, and put
the two OS-specific steps behind a tiny backend trait
(`trait SystemProxyBackend { fn enable(&self, port); fn force_off(&self);
fn snapshot(&self); }`) that macOS implements per-service and Windows
implements once. Removes ~200 duplicated lines and one entire drift risk
(the two files already have subtly different doc comments and cleanup
paths).

### 3. [High] Sidecar-state helpers copy-pasted across three integrations

`opencode.rs`, `openclaw.rs`, and `hermes.rs` each define their own
`const STATE_FILENAME`, a private `state_path()`, and `load_state()` /
`save_state()` that read/write `<app_support_dir>/<tool>-state.json` via
`serde_json`. The three `load_state`/`save_state`/`state_path` bodies are
byte-identical apart from the filename constant (opencode:71/STATE_FILENAME,
hermes:37, openclaw analogous).

**Suggested change:** Add a generic pair to `primitives.rs` (or a new
`integrations/sidecar_state.rs`):

```rust
pub fn load_state<T: DeserializeOwned>(filename: &str) -> Result<Option<T>>;
pub fn save_state<T: Serialize>(filename: &str, state: &T) -> Result<()>;
```

Each integration keeps only its `State` struct and `STATE_FILENAME`.
Centralizes the file-write convention (~20 lines x 3 removed).

### 4. [Medium] `map_err(|e| format!("{e:#}"))` repeated 39 times in lib.rs

`src-tauri/src/lib.rs` converts `anyhow::Error` to the `String` that
Tauri commands return via the identical closure `map_err(|e| format!("{e:#}"))`
in 39 places (116, 121, 131, 144, 156, 169, 177, 207, ...).

**Suggested change:** One free function `fn cmd_err(e: impl std::fmt::Display) -> String { format!("{e:#}") }`
used as `.map_err(cmd_err)`, or a private extension trait
`trait IntoCmd<T> { fn cmd(self) -> Result<T, String>; }` so call sites
read `integ.connect(&input).cmd()?`. Also note `tool_status`
(lib.rs:100-105) re-inlines the `ToolId::from_slug + registry::find`
lookup that the `resolve_integration` helper (lib.rs:180-183) already
encapsulates - call the helper.

### 5. [Medium] App.tsx re-inlines the same state-refresh fetch in 4 places

`src/App.tsx` fetches the `proxyStatus() / listProviders() / listTools()`
trio and sets state on initial load (116-126), the `proxy-state-changed`
listener (151-157), `toggleProxy` (213-236), and `setProvider`
(254-266). The post-failure re-sync `setProxy(await proxyStatus())`
appears at 232, 257, 283, 321, and the error coercion
`typeof e === "string" ? e : String(e)` at 229 and 262.

**Suggested change:** A single `refreshState()` callback (used by init,
the event listener, and after every mutation) plus an `errString(e)`
helper in `src/lib/`. `App.tsx` at 447 lines is the app's whole state
container; consolidating the refresh path is the cheap first step before
any larger split.

### 6. [Medium] Untested pure logic

Test density is good in `proxy/mod.rs` (14), `openclaw` (13), `provider`
(7), but several modules with pure, easily-testable logic have zero
tests:

- `proxy/config.rs` - `load_domains`/`set_enabled` merge built-in
  `default_domains` with the persisted per-slug enabled map; the
  "new built-in domain surfaces with its default" behavior described in
  the module doc is untested.
- `registry.rs` - `disconnect_all` aggregates per-tool failures into a
  joined bail message (registry.rs:~190-204); no test exercises the
  partial-failure path. `ToolId::slug`/`from_slug` round-trip is untested.
- `account.rs`, `env.rs`, `primitives.rs`, `keychain.rs` also have no
  in-file tests (some are OS-bound, but `env` path composition and
  `primitives` install-id formatting are portable and testable).

**Suggested change:** Add unit tests for `config::set_enabled` (toggle
persists, unknown default), the `disconnect_all` aggregation with a
failing stub integration, and a `from_slug(slug()) == Some(id)`
round-trip over all `ToolId` variants.

### 7. [Low] Em-dash usages violate project CLAUDE.md ("never use —")

`rg "—"` finds 26+ occurrences, concentrated in `openclaw.rs` (21) and
including user-facing strings: `openclaw.rs:237` ("not signed in — sign
in..."), `openclaw.rs:409` ("...credential — Gate Connect adds..."),
`cli/src/main.rs:333`. Most others are in doc comments across the
integration modules.

**Suggested change:** Replace `—` with `-` (or restructure the sentence)
project-wide; the user-facing strings are the priority since they surface
in the popover/CLI.

---

## Notes (not filed as findings)

- `proxy/mod.rs` domain catalog and `Decision` tunnelling logic are a
  good model - data-driven, well-tested. Keep new routing rules there.
- `env.rs` correctly delegates OS path differences to the `dirs` crate
  and uses a clean `GATE_CONNECT_TEST_HOME` test seam; no structural
  changes needed.
- Rust idiom check (per user preference to flag `match` on Option/Result
  where `map`/`unwrap_or`/`?` reads better): the integrations already use
  `let ... else`, `.map().unwrap_or_default()`, and `?` idiomatically -
  no notable offenders found. `if let ... else` here is idiomatic Rust,
  not the Scala anti-pattern.
- `registry::find(id)` is used consistently for slug lookup in both the
  CLI and Tauri layer; no lookup duplication there.
