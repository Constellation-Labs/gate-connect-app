# Plan: make the Gate proxy reach CLI tools on macOS (fix Gemini "not working")

## Problem

Gemini CLI is **proxy-only** in this app (`provider.rs:390`, `gemini_provider_is_proxy_only`):
it has no config integration because the Gemini CLI cannot carry a custom base URL
reliably, nor the `X-Gate-Api-Key` / `X-Gate-Upstream-Url` headers that Codex/Claude get
(header injection works only for MCP servers, not the model API; open upstream request
google-gemini/gemini-cli#1679). So everything depends on the local MITM proxy.

It works on Linux and fails on macOS because the two platforms expose the proxy to
processes differently:

| | Linux (works) | macOS (broken) |
|---|---|---|
| Proxy reach | `https_proxy` env var via `~/.config/environment.d/gate-proxy.conf` | `networksetup` system proxy only |
| Node CA trust | `NODE_EXTRA_CA_CERTS` env var | login-keychain root only |

The Gemini CLI runs on Node, and Node/undici ignore the macOS **system** proxy (they only
read `HTTP(S)_PROXY` env vars) and ignore the **keychain** (they need `NODE_EXTRA_CA_CERTS`
or `NODE_USE_SYSTEM_CA`, both read at process init). macOS sets neither env var for CLI
processes, so the Gemini CLI connects direct to Google, bypassing Gate. macOS is missing
the env-var half that `system_proxy_linux.rs` provides.

## Approach

Give macOS the same CLI env-var injection Linux already has, via a shell startup file.
There is no macOS equivalent of `environment.d` that the OS injects into every process; the
only reliable path to CLI (Node) processes is a shell rc file. Use `~/.zshenv` (zsh is the
macOS default, and `.zshenv` is sourced for all zsh invocations including non-interactive
CLI launches). Write a sentinel-delimited managed block on proxy-enable, strip it on
disable. This is a general macOS parity fix: it benefits every Node CLI, not just Gemini,
exactly like the Linux drop-in.

This does NOT touch the macOS `networksetup` system-proxy behavior (it still helps GUI apps)
and does NOT add a Gemini config integration (infeasible, see Problem).

Key differences from the Linux module the new one mirrors:
1. `~/.zshenv` is a **shared, shell-sourced** file (user content may exist), so edits must be
   **surgical block insert/replace/remove** using sentinels, never a whole-file
   overwrite/delete like the Linux dedicated drop-in.
2. Lines are shell `export KEY="VALUE"` with all values double-quoted (the CA path contains
   spaces: `~/Library/Application Support/...`). Linux uses raw `KEY=VALUE` (systemd format).
3. Same "only new shells pick it up" caveat as Linux; documented in the block comment.

## Files touched + new symbols

1. **NEW `crates/core/src/proxy/system_proxy_macos_env.rs`**
   Pure fs + string module (no macOS-only APIs). Public API mirrors the Linux enable/off pair:
   - `pub fn enable(port: u16) -> Result<()>` : upsert the managed block into `~/.zshenv`.
   - `pub fn disable() -> Result<()>` : strip the managed block; no-op if absent or file
     missing (maps `NotFound` to `Ok`, like Linux `force_off`).
   Private helpers:
   - `const BLOCK_BEGIN` / `BLOCK_END` : sentinel comment lines.
   - `fn zshenv_path() -> Result<PathBuf>` : `dirs::home_dir()`-based, so tests redirect via `$HOME`.
   - `fn ca_cert_path() -> Result<PathBuf>` : `env::app_support_dir()?.join("proxy").join("ca-cert.pem")`
     (mirrors Linux `system_proxy_linux.rs:99`; see Open decision 3 re: reuse vs duplicate).
   - `fn build_block(port: u16) -> Result<String>` : the export block below.
   - `fn upsert_block(existing: &str, block: &str) -> String` : replace between sentinels if
     present, else append (ensuring a separating newline so we never join a user line).
   - `fn strip_block(existing: &str) -> String` : remove the sentinel span (and its trailing
     newline) if present.
   Plus a `#[cfg(test)]` module (see Tests).

2. **`crates/core/src/proxy/mod.rs`** : declare the module in the macOS cfg block after the
   `system_proxy` group (near line 56). See Open decision 1 for gated vs ungated.

3. **`crates/core/src/proxy/manager.rs`** (macOS manager): add to `use super::{...}` (line 17)
   and wire the four revert-symmetric call sites:
   - `enable()` after `system_proxy::enable(port, &services)` (after `manager.rs:113`):
     `system_proxy_macos_env::enable(port)`.
   - `disable()` after restore/force_off, near `clear_snapshot` (`manager.rs:162-172`):
     `system_proxy_macos_env::disable()`.
   - `handle_engine_crash()` at the revert (`manager.rs:253-255`): `disable()` (best-effort).
   - `reconcile_on_startup()` after `system_proxy::force_off` (`manager.rs:283`): `disable()`
     (best-effort; strips a stale block from a crashed session so a dead-port proxy env does
     not break new shells).

No changes to Linux/Windows modules or to the Gemini/registry code.

## The managed block written to `~/.zshenv`

```
# >>> gate-connect proxy (managed, do not edit) >>>
# Written while the Gate proxy is ON so CLI tools (Node-based: Gemini, etc.)
# route through the local engine and trust its CA. Only NEW shells read this;
# restart open terminals. Removed automatically when the proxy is OFF.
export http_proxy="http://127.0.0.1:{port}"
export https_proxy="http://127.0.0.1:{port}"
export HTTP_PROXY="http://127.0.0.1:{port}"
export HTTPS_PROXY="http://127.0.0.1:{port}"
export no_proxy="localhost,127.0.0.1,::1"
export NO_PROXY="localhost,127.0.0.1,::1"
export NODE_EXTRA_CA_CERTS="{ca_cert_path}"
# <<< gate-connect proxy (managed, do not edit) <<<
```

`{port}` is `running.port()` (`manager.rs:110`; engine always binds `127.0.0.1`). Re-enable
with a new dynamic port replaces the block in place (single block invariant).

## Idempotency / safety

- Enable is an upsert: exactly one managed block regardless of how many times it runs; a new
  port rewrites the existing block, never appends a second.
- Disable removes only the sentinel span; all user content in `~/.zshenv` is preserved.
- Disable is a no-op when the block or the file is absent.
- Serialization: the macOS `ProxyManager` holds the engine mutex across enable/disable
  (`manager.rs:63`), so within-process writes are serialized. No new lock is added (the macOS
  `system_proxy` module has none either).

## Tests (in the new module)

Mirror the Linux `with_temp_env` harness (`system_proxy_linux.rs:197`): a `static
Mutex` guard to serialize, redirect `HOME` to a temp dir so `~/.zshenv` lands in the
sandbox, and `env::set_app_support_dir_for_tests(Some(...))` for the CA path; reset both on
teardown. Cases:
1. enable on a missing/empty file writes the block with sentinels, `export
   HTTPS_PROXY="http://127.0.0.1:{port}"`, quoted `NODE_EXTRA_CA_CERTS="`, and the no_proxy pair.
2. enable preserves pre-existing user content (seed `export FOO=bar\n`, assert it survives and
   the block is appended with a separating newline).
3. re-enable with a new port yields exactly one block containing the new port and not the old.
4. disable strips only the block, leaving user content intact.
5. disable is a no-op with no block, and with no file at all.
6. `build_block` format/quoting unit check.

## Verification

- `cargo test -p <core crate>` stays green on the Linux dev box (Linux build/tests untouched).
- New-module tests run per Open decision 1 (macOS CI if gated; Linux dev box if ungated).
- Manual end-to-end on the reporter's Mac: enable Gemini in the app, open a NEW terminal,
  `env | grep -iE 'proxy|node_extra_ca_certs'` shows the vars, then `gemini` transits Gate
  (confirm via the engine request log). Toggle off, open a new terminal, confirm vars gone
  and `~/.zshenv` user content intact.

## Open decisions (recommendations inline)

1. **Module gating.** Recommend `#[cfg(target_os = "macos")] pub mod system_proxy_macos_env;`
   to match every other platform module and keep it out of Linux/Win binaries; its tests then
   run on macOS CI. Alternative: leave it ungated (`pub mod`, pure Rust) so tests run on the
   Linux dev box too, at the cost of compiling unused code into non-macOS builds. Pick one.
2. **Enable failure semantics.** Recommend best-effort: log a warning (matching the logging
   macro already used in `manager.rs`) and keep the proxy "on" if the `~/.zshenv` write fails,
   since the system proxy still serves GUI/config tools. Alternative: treat it as fatal and
   roll back the whole enable like the `system_proxy::enable` failure branch at `manager.rs:113`.
3. **CA path source.** Recommend duplicating a private `ca_cert_path()` (matches the Linux
   module precedent, keeps the change local). Alternative: make macOS `ca::cert_path`
   (`ca.rs:58`) `pub` and call it, avoiding path drift but touching another file.
4. **Shell coverage.** Recommend zsh `~/.zshenv` only for the first cut. Add a bash file
   (`~/.bash_profile`) later only if a report warrants it.

## Out of scope

- No Gemini config/settings.json integration (infeasible: no base-URL/header support).
- No change to `networksetup` system-proxy behavior.
- No Linux/Windows changes.
- Bash/fish shell support (deferred, decision 4).
