# Correctness review - `main` (Gate Connect)

Scope: provider state machine + reconcile, proxy intent persistence,
per-OS proxy managers, teardown, and the React state layer. Method: read
+ trace, then rate CONFIRMED vs PLAUSIBLE with a concrete failure path.

## Summary (by severity)

| Severity | Count |
|----------|-------|
| High     | 0 |
| Medium-High | 1 |
| Medium   | 1 |
| Low      | 3 |
| Info / considered-safe | 2 |

Two findings are genuine logic bugs that clobber the user's explicit
per-provider choices; the rest are robustness gaps and per-OS parity
issues. No memory-unsafety or data-corruption bugs found (config writes
go through an atomic temp+rename in `primitives::write_file`).

Ranked most-severe first:

1. [Medium-High, CONFIRMED] Master proxy toggle re-enables providers the user turned off, and defeats `restore_all`
2. [Medium, CONFIRMED] `enable`/`disable` persisted-intent asymmetry breaks `reconcile_enabled` for the proxy-off case
3. [Low, PLAUSIBLE] Windows startup reconcile has no `clear_stranded_loopback` sweep (macOS parity gap)
4. [Low, PLAUSIBLE] `toggleProxy` / `setProvider` double-submit through a stale `proxyBusy` closure
5. [Low, PLAUSIBLE] Codex reports a misleading `Drifted` after the user logs out of Codex
6. [Info] Considered and judged safe: concurrent reconcile writers; non-atomic intent write

---

## 1. [Medium-High, CONFIRMED] Master proxy toggle re-enables providers the user turned off, and defeats `restore_all`

Files:
- `src/App.tsx:207-224` (the enable-all loop)
- `crates/core/src/provider.rs:189-204` (`state`: `available = any_detected || proxy_running`)
- `crates/core/src/provider.rs:399-434` (`snapshot_and_disable_all` / `restore_all`)
- `src-tauri/src/lib.rs:366` (`proxy_enable` calls `restore_all` first)

When the user flips the master "Route through Gate" switch on, the
frontend does this after `proxyEnable()` returns:

```ts
let current = await listProviders()...
for (const p of current.filter((p) => p.available && !p.enabled)) {
  await providerEnable(p.slug);
}
```

`available` is computed as `any_detected || proxy_running()`
(`provider.rs:193-200`). Once the proxy is running, **every** provider is
`available`, so the loop calls `providerEnable` on every provider that
isn't already on.

Two concrete misbehaviors:

- **Clobbers an explicit off-choice.** State: user uses only Claude, has
  Codex installed, and deliberately left "OpenAI / Codex" off
  (`enabled=false`). They enable the master proxy. The loop calls
  `providerEnable("openai")`, which edits `~/.codex/config.toml`
  (`[model_providers.gate]` + `model_provider = "gate"`) - a config
  mutation the user never asked for. The OpenRouter proxy domain is
  turned on the same way.

- **Makes `restore_all` dead code.** The backend `proxy_enable` runs
  `restore_all()` (`lib.rs:366`), which re-enables *exactly* the
  providers that were on when the master was last turned off (the
  `snapshot_and_disable_all` snapshot). The frontend then immediately
  turns on *all remaining available* providers, i.e. precisely the ones
  `restore_all` intentionally left off. The careful snapshot/restore
  design in `provider.rs:399-434` never has observable effect through the
  GUI path.

This directly contradicts the recent "default anthropic to opt-in" /
per-provider off-persistence work: a provider the user opted out of is
switched back on by an unrelated action (enabling the proxy). Fix
direction: on master-on, rely on `restore_all` (don't force-enable in the
frontend), or only auto-enable providers that were part of the restored
snapshot.

---

## 2. [Medium, CONFIRMED] `enable`/`disable` persisted-intent asymmetry breaks `reconcile_enabled` when the proxy is off

Files:
- `crates/core/src/provider.rs:225-262` (`enable` / `disable` bodies)
- `crates/core/src/provider.rs:272-276` (disable's "record off durably" comment)
- `crates/core/src/provider.rs:296-304` (`domains_enabled_persisted`)
- `crates/core/src/provider.rs:321-349` (`reconcile_enabled`)

`reconcile_enabled` is the startup/focus sweep that wires up a tool
installed *after* its provider was enabled. It decides "is this provider
on?" purely from the persisted domain-config file, via
`domains_enabled_persisted` (`provider.rs:326`).

The two writers of that persisted flag are asymmetric:

- `disable()` **always** records the off-state: when the engine is live
  it routes through `manager().set_domain(slug,false)` (which persists),
  and otherwise it persists directly via `config::set_enabled` - see the
  comment at `provider.rs:272-276`.
- `enable()` only persists the on-state when `plan.enable_domain` is true,
  i.e. only when `proxy_running()`. In the proxy-off path (the common
  case, since the proxy is opt-in) `enable()` configures the tool but
  **never** calls `config::set_enabled(slug, true)`.

So enabling a provider while the proxy is off leaves
`domains_enabled_persisted` reading its default/last value, not `true`.

Failure scenario: user has Claude, turns Anthropic **off** (persists
`anthropic=false`), then turns it back **on** with the proxy off (Claude
reconnected, but persisted stays `false`). Later Claude Code rewrites/
resets `~/.claude/settings.json` (update, reinstall, manual edit) so its
status reverts to `Detected`. On next launch `reconcile_enabled` reads
`anthropic=false` and skips it (`provider.rs:326-328`) - the user's
on-choice is silently dropped and routing is not restored. The same gap
means `reconcile_enabled` can never re-establish a *user-enabled* OpenAI/
Codex after its config is reset, defeating the feature's stated purpose
for every provider except default-on Anthropic in its untouched default
state.

Fix direction: make `enable()` persist the on-intent unconditionally
(mirror `disable()`), so `domains_enabled_persisted` is a true record of
intent rather than of live proxy domain state.

---

## 3. [Low, PLAUSIBLE] Windows startup reconcile lacks the stranded-loopback sweep macOS has

Files:
- `crates/core/src/proxy/manager.rs:270-294` (macOS: always runs `clear_stranded_loopback()`)
- `crates/core/src/proxy/manager_windows.rs:259-277` (Windows: no equivalent)

macOS `reconcile_on_startup` unconditionally calls
`clear_stranded_loopback()` (`manager.rs:287`) as belt-and-suspenders:
even when no snapshot survives, it clears any system proxy still pointing
at a dead loopback port, "otherwise strands every proxy-honoring app with
ERR_PROXY_CONNECTION_FAILED while Gate shows off"
(`manager.rs:266-268`).

Windows `reconcile_on_startup` handles `Ok(Some)` (restore) and `Err`
(force off) but `Ok(None)` does nothing, and there is no
stranded-loopback sweep afterward. If WinINET is left with
`ProxyEnable=1, ProxyServer=127.0.0.1:<deadport>` while the snapshot is
absent, Windows startup won't recover it; macOS would.

Rated Low/PLAUSIBLE rather than Confirmed because Windows `enable()` saves
the snapshot *before* redirecting the proxy (`manager_windows.rs:93` then
`:114`) and reverts before clearing the snapshot on disable, so the
"stranded with no snapshot" state is hard to reach through the normal
code path. It bites only if the snapshot file is lost/partial out of band
(user deletes app-support, partial write) - exactly the case macOS
defends against and Windows does not. Linux uses a delete-our-own-file
model, so it's N/A there.

---

## 4. [Low, PLAUSIBLE] `toggleProxy` / `setProvider` can double-submit via a stale `proxyBusy` closure

Files:
- `src/App.tsx:202-204` (`toggleProxy`), `src/App.tsx:243-245` (`setProvider`)

Both guards are `if (proxyBusy) return; setProxyBusy(true);`. `proxyBusy`
is captured from the render closure. Two rapid activations dispatched
before React re-renders (fast double-click / Enter-repeat) both observe
`proxyBusy === false` and both proceed, firing the backend op twice.
`setProvider` also depends only on `[proxyBusy]` in its `useCallback`, so
between renders the same stale closure is reused. Impact is limited
because the backend ops are largely idempotent (enable/disable re-apply
the same config), but on the master toggle a double-fire can interleave a
disable+enable pair. Fix direction: use a `useRef` busy latch set
synchronously, or disable the control on first activation.

---

## 5. [Low, PLAUSIBLE] Codex reports a misleading `Drifted` after the user logs out of Codex

Files:
- `crates/core/src/integrations/codex.rs:95-113` (`read_auth_mode` bails if `auth.json` absent)
- `crates/core/src/integrations/codex.rs:260-268` (`status` uses `read_auth_mode().unwrap_or(AuthMode::Chatgpt)`)
- `crates/core/src/integrations/codex.rs:316` (`connect` uses `read_auth_mode()?`)

`connect()` writes `base_url` using the auth mode read at connect time
(e.g. `apikey` -> `.../v1`). `status()` recomputes the expected base with
`read_auth_mode().unwrap_or(AuthMode::Chatgpt)`. If the user connected in
`apikey` mode and later logs out of Codex (removing `auth.json`),
`read_auth_mode()` bails and `status()` silently falls back to `Chatgpt`,
whose expected base is `.../codex`. That mismatches the persisted `.../v1`
and the tool is reported `Drifted("base_url is .../v1, expected
.../codex")` even though no config changed. The drift message points at a
mode change that didn't happen. Low impact (Codex can't route logged-out
anyway, and `reconcile_enabled` only touches `Detected`, not `Drifted`),
but the surfaced reason is wrong and reconnect is blocked until re-login.

---

## 6. [Info] Considered and judged safe

- **Concurrent reconcile / restore writers.** At startup two threads run
  (`lib.rs:786` `reconcile_enabled`, and `lib.rs:803-` `restore_all` +
  `manager().enable()`), plus a popover-focus `reconcile_enabled`
  (`lib.rs:720-`, guarded by the `POPOVER_VISIBLE` hidden->visible edge).
  All can call `ClaudeCode.connect` concurrently, which read-modify-writes
  `~/.claude/settings.json`. This is safe *only* because
  `primitives::write_file` writes to a temp file and renames (atomic, no
  torn file) and the env-merge is idempotent, so concurrent writers
  converge on identical content (last-writer-wins). It's correct today
  but fragile: any future non-idempotent step in `connect` would become a
  lost-update race, since nothing serializes tool-config writes across
  these threads (the `flock` op-lock covers only the proxy snapshot/
  drop-in path, not tool configs).

- **Non-atomic intent write.** `proxy/intent.rs` `write_intent` uses
  `fs::write` (not temp+rename), unlike the rest of the config layer. A
  crash mid-write could truncate `intent.json`, but the reader treats an
  unreadable/absent intent as `false`, so the failure mode is "routing
  not auto-restored after reboot" rather than corruption. Acceptable;
  noted for consistency.