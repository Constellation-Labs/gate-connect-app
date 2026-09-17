# AG-879 - Gate Connect defect and UX cleanup

Sequencing plan for the 22 children of AG-879. Written 2026-09-16; rewritten
the same day once PR #273 merged (`890a3d1`) and removed the constraint the
first version was built around.

## What changed, and why this was rewritten

The first version of this plan sequenced the work around PR #273's open diff:
that PR rewrote most of the new UI, so the copy tickets - which look like the
cheap warm-up - were the worst place to start, because they live in
`dialogs.tsx` and `NewUiApp.tsx`. That constraint is gone. Nothing here is
blocked by another branch any more, so the order below is driven by the work
itself.

One thing survives from #273 and is worth carrying forward: it removed the
sidebar's shell-environment card, and it left question 24 open in
`docs/figma-questions-for-design.md` (the PAYG balance). Both touch tickets in
this epic - see Re-scope below.

## Buckets

**A. Engine and routing path** - AG-899 (High), AG-911 (High)
**B. Routing state and lifecycle** - AG-885, 892, 895, 897, 900
**C. Numbers** - AG-882, 884, 887, 891, 894, 896
**D. Copy** - AG-880, 886, 889, 893, 898, 901
**E. Small and self-contained** - AG-881, 883, 888

## Sequence

### 1. The C spike, first and before any C fix

Six "the number is wrong" tickets are unlikely to be six bugs. AG-882
(percentage disagrees with the dashboard), AG-894 (Overview chart stops hours
before the app chart), AG-896 (OpenCode shows nothing the dashboard records) and
AG-887 (Type column empty) all point at the same place: what Connect asks
`/v1/me/activity` for, and how it attributes the answer. Instrument one read
against staging, compare it to the dashboard for the same window and account,
and write down which of the six survive as separate bugs.

This is first because it is the only item whose *output changes what the other
items are*. Fixing the six one at a time is how a single bug becomes five
patches and one of them is wrong.

Needs: the isolated instance (see the memory note), a staging key, and a
dashboard to compare against.

### 2. E and D, while the spike's findings settle

Now unblocked, and genuinely cheap.

- **AG-881** is a delete. The epic's own Key decisions say "Connect does not
  display estimated dollar figures for token savings", so there is nothing to
  design.
- **AG-883** (Tokens Saved tile click adds space and jumps the page) and
  **AG-888** (multi-select for a single-model app) are self-contained.
- **D** batches by surface rather than by ticket: six copy tickets across three
  dialogs is three commits, not six.

### 3. B's process-detection half

**AG-895** (toggling OpenCode asks the user to close Codex) and **AG-900**
(closing Claude leaves it running) are both `crates/core` process work and share
a likely cause. **AG-892** (Hermes reported Protected while not routed) and
**AG-897** (OpenRouter routed but no activity) may fall out of the C spike
instead - check its findings before opening them.

### 4. A, last, and on its own branch

AG-911 changes what happens to every connection on the machine. It deserves its
own review and its own revert button, so it does not ride in a branch with
twenty copy fixes.

**Its first commit is a test seam, not the listener.**
`crates/core/src/proxy/system_proxy.rs` has none: `apply()` shells straight out
to `launchctl setenv` and `networksetup`, and `GATE_CONNECT_TEST_HOME` does not
reach it, so enabling routing changes the whole login session. That is both the
testing blocker and the reason this cannot be verified on a machine anyone is
working on.

Then the listener: keep one bound to `preferred_port` (`proxy/engine.rs`,
already persisted across runs) whenever routing is off, forwarding verbatim.
`system_proxy.rs` already documents the cause AG-899 reports - "already-running
processes keep their old environment ... neither fixable from here" - and AG-911
is the argument that it is fixable one step further out: leave something
listening on the port the stale variable names.

Watch the quit path. AG-911's scope note is explicit that app-exit runs the same
teardown as the toggle, so a listener that dies with the GUI fixes the switch
and leaves quitting broken the same way.

## Parked, waiting on one product answer

**AG-889 and AG-897 are one decision, not two.** Both are about entries that are
not apps: `api.openai.com` and OpenRouter are hosts any app can be pointed at,
and Gate cannot attribute their traffic per-entry because `client_tool` comes
from the caller's User-Agent. So neither page can ever show its own activity,
and both currently say so and nothing else.

**AG-889's suggested fix should not be taken as written.** It proposes folding
the OpenAI API entry into ChatGPT / Codex. `groups.ts` argues against that in
place, and the argument is specific: nothing OpenAI ships rides that entry -
Codex routes through the relay regardless, and the desktop app is on
`chatgpt.com` - so what depends on it is any *other* program calling
`api.openai.com`. Folding it in makes turning on ChatGPT silently route every
OpenAI-calling script on the machine, which is the same shape as the problem
AG-893 just fixed.

The suggestion also depends on "What this switch covers", which #273 removed
because no frame draws it. There is no longer a place on the switch to put the
disclosure it relies on.

**What the tickets actually ask for** is narrower than the suggestion: AG-889's
Expected is that no screen exists whose only message is that its own numbers are
unavailable, and AG-897's is that OpenRouter is "grouped with the provider
endpoints rather than under Apps". Both are satisfied by:

1. A provider-endpoints group, out of Apps, by the same rule that moved
   `env-proxy` in AG-893.
2. Honest copy on those pages - routed and protected, traffic not attributed to
   one app, counted in the Overview - instead of three `n/a` tiles and two
   "isn't attributed" cards.

**The question we cannot answer ourselves:** should the rail have a third
category, or do provider endpoints stay under Tools? That is a shape decision,
and it is adjacent to the epic's own open question, which AG-879 assigns to the
squad lead.

- If **yes**: implement both steps, AG-897 closes with it, the fold-in is dropped.
- If **no**: do step 2 alone, which is AG-889's literal Expected, and the entries
  stay where they are.
- If the **fold-in** is still wanted: someone has to accept that routing ChatGPT
  routes unrelated scripts, and find somewhere to disclose it.

Nothing was implemented for either ticket. AG-897's one-word band change is held
with it, because it is the same decision.

## Re-scope before anyone picks these up

- **AG-893** is half-resolved. #273 removed "Also set shell environment
  variables" from the sidebar, so the pair this ticket is about no longer both
  exist. It also opened a new hole: the OpenCode dialog still turns `env_export`
  on machine-wide and the new UI has nowhere to turn it off. That hole is closer
  to what AG-893 should now be about.
- **AG-890** is popover state-sync, and `CLAUDE.md` records the popover as on
  its way out. Retiring one shell may be worth more than syncing two.
- **AG-899** is the report and AG-911 is the fix. Expect to close AG-899 by
  verifying it, not by patching it.
- **AG-891** (Gate plan reads "Unavailable" for a Pro account) may be the same
  read as question 24 in `docs/figma-questions-for-design.md`, which asks
  whether the frontend should see `billing_mode` at all. Worth answering
  together.

## Testing note

Use the isolated instance for anything that signs in or onboards. It does not
cover `system_proxy.rs`, so until step 4's seam exists, turning routing on in
any instance sets machine-wide proxy variables.
