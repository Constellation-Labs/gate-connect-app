# AG-879 - Gate Connect defect and UX cleanup

Sequencing plan for the 22 children of AG-879. Written 2026-09-16, while
PR #273 (design conformance) is open and unmerged.

## The constraint that decides the order

PR #273 rewrites most of the new UI. Until it merges, anything it touches is a
conflict waiting to happen:

    src/NewUiApp.tsx            src/components/gc/AppPane.tsx
    src/components/gc/dialogs.tsx        .../Overview.tsx
    .../Sidebar.tsx             .../SettingsPane.tsx
    .../Tray.tsx                .../Modal.tsx
    .../ProviderMark.tsx        .../InstallationPicker.tsx (deleted)
    src/index.css               src/lib/activityGaps.ts
    e2e/new-ui-{engine,model-picker,routing,tray}.spec.ts

`NewUiApp.tsx` is the wiring hub, so nearly every *UI* ticket lands in it.

**This inverts the obvious order.** Copy fixes look like the cheap warm-up, but
they live in `dialogs.tsx` and `NewUiApp.tsx` - the two files #273 changed most.
The work that is genuinely free to start is the part with no UI in it at all:
the Rust engine, and the data layer under the numbers.

Untouched by #273, and therefore open:

    crates/core/**              src-tauri/**
    src/lib/activity.ts         src/lib/toolEvents.ts
    src/lib/verdict.ts          src/lib/groups.ts
    src/components/gc/metrics.tsx        .../SecurityEvents.tsx
    src/components/gc/base.tsx           .../banners.tsx
    src/screens/**              src/App.tsx (popover)

## Buckets

**A. Engine and routing path** - AG-899 (High), AG-911 (High)
**B. Routing state and lifecycle** - AG-885, 892, 895, 897, 900
**C. Numbers** - AG-882, 884, 887, 891, 894, 896
**D. Copy** - AG-880, 886, 889, 893, 898, 901
**E. Small and self-contained** - AG-881, 883, 888

## Sequence

### Phase 1 - starts now, no overlap

**1. The C spike (no files touched).** Six "the number is wrong" tickets are
unlikely to be six bugs. AG-882 (percentage disagrees with the dashboard),
AG-894 (Overview chart stops hours before the app chart), AG-896 (OpenCode shows
nothing the dashboard records) and AG-887 (Type column empty) all point at the
same place: what Connect asks `/v1/me/activity` for, and how it attributes the
answer. Instrument one read against staging, compare it to the dashboard for the
same window and account, and write down which of the six survive as separate
bugs. Fixing them one at a time is how a single bug becomes five patches.

**2. AG-911 seam first, then the listener.** `crates/core/src/proxy/system_proxy.rs`
has no test seam: `apply()` shells straight out to `launchctl setenv` and
`networksetup`, so it changes the developer's whole login session and
`GATE_CONNECT_TEST_HOME` does not cover it. That is both a testing blocker and
the reason this cannot be verified on a machine someone is working on. First
commit adds the seam; the listener follows.

Then AG-911 proper: keep a listener bound to `preferred_port`
(`proxy/engine.rs:79`, already persisted across runs) whenever routing is off,
forwarding verbatim. `system_proxy.rs:377` already documents the cause AG-899
reports - "already-running processes keep their old environment ... neither
fixable from here" - and AG-911 is the argument that it is fixable from one step
further out: leave something listening on the port the stale variable names.

Watch the quit path. AG-911's scope note is explicit that the app-exit teardown
runs the same code as the toggle, so a listener that dies with the GUI fixes the
switch and leaves quitting broken in exactly the same way.

**3. B's process-detection half** - AG-895 (toggling OpenCode asks the user to
close Codex) and AG-900 (closing Claude leaves it running). Both are
`crates/core` process work, both independent of the UI.

### Phase 2 - after #273 merges

**4. Whatever the C spike left standing**, now that `AppPane`/`Overview` are
free.

**5. E**, smallest first. AG-881 is a delete: the epic's own Key decisions say
"Connect does not display estimated dollar figures for token savings". AG-883
(tile click jumps the page) and AG-888 (multi-select for single-model apps) are
both self-contained - AG-888 lands in the picker #273 just redrew, so it must
wait.

**6. D**, batched by surface rather than by ticket. Six copy tickets across
three dialogs is three commits, not six.

## Re-scope before anyone picks these up

- **AG-893** is half-resolved by #273, which removed "Also set shell environment
  variables" from the sidebar. The ticket is about that card overlapping the
  Terminal switch; half of the pair no longer exists.
- **AG-890** is about the popover, which `CLAUDE.md` records as on its way out.
  Syncing state between two shells may be worth less than retiring one.
- **AG-899** overlaps AG-911 entirely. It is the report; AG-911 is the fix.
  Expect to close AG-899 by verifying it, not by patching it.

## Testing note

AG-911 cannot be verified on a machine someone is working on until the seam in
step 2 exists: enabling routing sets machine-wide proxy variables and changes
the system proxy. A running Claude Code session is safe (its environment is
already fixed, and its child shells inherit from it rather than from launchd),
but new terminals and GUI apps are not. The alternatives are the seam, a second
macOS user account, or a VM.
