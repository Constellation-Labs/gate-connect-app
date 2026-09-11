# One switch per app

Status: built. Follows `docs/routing-taxonomy-proposal.md`,
which this deliberately hides rather than removes.

## Decision

The ledger stops being one row per routable surface and becomes one row per
**app the user has on their machine**. A switch routes everything that app does,
whatever mechanism each surface needs.

| Band | Switch | Covers | Mechanisms in one switch |
|---|---|---|---|
| Apps | **Claude** | `claude-code` CLI, `anthropic` API, `claude-web` Chat | config write + host intercept |
| | **ChatGPT / Codex** | `codex` CLI, `chatgpt-apps` Chat, `chatgpt` Work | config write + host intercept |
| | **OpenRouter** | `openrouter` | host intercept |
| Tools | **OpenClaw** | `openclaw` | config write |
| | **Hermes** | `hermes` | config write |
| | **OpenCode** | `opencode`, `opencode` Zen / Go | config write + host intercept |
| | **Terminal** | `env-proxy` | machine environment |
| | **OpenAI API** | `openai` | host intercept |

`openai` and `env-proxy` stay separate switches rather than sharing a heading:
they are different mechanisms with different blast radii, and nothing depends on
both. `api.openai.com` is not part of the ChatGPT app - no OpenAI product Gate
configures rides it - so it does not belong to that switch.

## What this trades away, and the mitigation

Two of the ChatGPT switch's three members, and one of Claude's, are
`Credential::Additive`: they carry the user's own signed-in session rather than a
key Gate brokers. **The invariant on this branch is that no group switch may
route such a row**, enforced in Rust by `provider::cascade_domains` and in the UI
by `cascadeTargets`.

An app switch breaks that by design. That is a product decision, taken
deliberately: "one switch turns my Claude on" is worth more than the guarantee,
provided the user knows which of the two things they just did.

**The mitigation is consent, not prevention.** An app switch whose members
include an additive row asks once, on first enable, naming what it routes:

> Gate will also route your Claude chats. Those go through Gate on the account
> you are already signed in with, not on a key Gate holds for you.

`Credential` stays exactly as it is - data on the row, still driving the
diagnostics triple, the CLI table and the pane copy. What changes is that
`cascades()` stops being the UI's hard gate and becomes the thing that decides
whether the confirmation appears. The invariant's *purpose* - never route a
signed-in session unknowingly - survives; the mechanism moves from "impossible"
to "confirmed".

## The Rust side needs no change, which was worth checking

Master-off calls `disable_inner` per enabled provider, which iterates
`cascade_domains` only, so a session domain is never flipped by the provider
layer in either direction. Its flag persists in `domains.json` and survives a
routing cycle untouched.

So an app switch that turns `claude-web` on cannot have that silently undone by
a master off/on, and `restore_all` needs no new case. The UI cascade is
client-side (`setGroupRouted` walking `cascadeTargets`), which is the only place
the rule has to change.

One divergence to accept and document: after this, the UI's app switch and the
CLI's `provider::enable` mean different things - the CLI still routes brokered
rows only. The CLI has no app-switch equivalent today anyway, so nothing
regresses; it just stops being true that the two surfaces do the same thing.

## Work

**`groups.ts`** - most of it, and mechanical. `buildGroups` swaps its `CLIENTS`
bucketing for a `SECTIONS` table. `cascadeTargets` gains the additive rows when
the caller has consent. `Group` keeps `cascadeDesired` for the switch state,
which already does the right thing for a mixed section.

**The rail** draws one row per app under two band eyebrows.

**The confirmation** is a new dialog, and `Modal` already has the widths. The
"has the user consented for this section" bit needs somewhere to live -
`preferences.rs` is the natural home, keyed per section rather than globally, so
consenting to Claude says nothing about ChatGPT.

**The pane aggregates**, and it turned out to be nearly free. Every section has
at most one config tool - a property of the sections rather than a coincidence,
since each app has a single thing Gate writes a file for - and the gateway
attributes nothing to a host, so there is no second reading to add in. The pane
reads that one tool and says what it cannot attribute, which it already knew how
to do.

**`sectionStatus` is the new part.** A section spans mechanisms, so its one
status line has to answer for all of them: an exception on any surface outranks
everything, then routing if every governed surface is, then a "Partly routed"
state a per-surface ledger never had to describe, then off. It reads the
brokered half for on/off and every member for exceptions, so a section is not
"off" because its session surface is.

**The tray matches, and that is not cosmetic.** A tray still drawing
per-surface rows would put a switch directly on the session surfaces - a way to
route someone's sign-in without ever being asked.

**Untouched:** the CLI, the diagnostics report, `taxonomy.rs`, and every Rust
invariant. The session surfaces keep working as they do now; what changes is
which switch reaches them.
