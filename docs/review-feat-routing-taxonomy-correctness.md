# Review: `feat/routing-taxonomy` - correctness

Base: `origin/feat/new-app-ui`. 55 files, +3712/-1315.

## Summary

The taxonomy itself is sound and the Rust half holds: `cascade_domains` derives
the additive exclusion from the row rather than from a hand-kept array, the
`provider::state` / `off_members` / `restore_all` / `snapshot_and_disable_everything`
paths all read the derived set, and their behaviour is byte-for-byte what
`proxy_domain_slugs` produced before, so nothing regressed there. The damage is
concentrated in the UI collapse from one switch per surface to one switch per
app, and it is one root cause with several faces: a section's switch renders
from `Group.cascadeDesired > 0` (ANY brokered member is on) while the same click
acts through `cascadeTargets`, whose complement is a different set entirely.
Where a section has no brokered member at all the two never meet and the switch
becomes inert in both directions while the surfaces underneath it are routing;
where a section mixes a connected domain with a disconnected tool the switch
reads on and the only click available turns off the half that was working. This
is principle 2's named failure, arriving through the section layer rather than
through the observed/intent split it was written for. Alongside that sit three
surviving section-id-vs-member-key crossings (`multiProviderSlugs`,
`brandMarkFor`, `openSectionTool`), a dead per-member failure path in both
shells' `routeSection`, and an e2e fake backend that now models the opposite of
the cascade rule the branch exists to enforce.

Everything below was checked against the code, and the two switch findings were
reproduced by running `buildGroups` / `sectionStatus` / `cascadeTargets` directly.

---

## High

### H1. A section with no brokered member is a switch that can never be turned off, over surfaces that are routing

`src/lib/groups.ts:824`, `src/lib/verdict.ts:158-162`, `src/NewUiApp.tsx:1513`,
`src/components/gc/Sidebar.tsx:600`

`SECTIONS`' `chatgpt` entry is `["codex", "chatgpt-apps", "chatgpt"]`
(`src/lib/groups.ts:668`). `codex` is the only brokered member: both chatgpt.com
catalog entries are `Credential::Additive` (`crates/core/src/proxy/catalog.rs:344`,
`:402`). On any machine without the Codex CLI installed, `buildGroups` drops it
(`groups.ts:763`) and the section is two additive rows.

Reproduced with that section and both domains enabled:

```
members           [ 'chatgpt-apps', 'chatgpt' ]
cascadeDesired    0            -> Sidebar renders the switch OFF
sectionStatus     { kind: "not-routed", detail: "Off" }
cascadeTargets(on)  []
cascadeTargets(off) [ 'chatgpt-apps', 'chatgpt' ]
```

Concretely: user clicks the ChatGPT / Codex switch, accepts the session-consent
dialog, both chatgpt.com surfaces start being intercepted. The row then reads
"ChatGPT / Codex - Not routed - Off" with the switch off. `Sidebar.tsx:600` sends
`onToggle(slug, !app.on)`, so every subsequent click requests `next = true`,
`cascadeTargets(group, true, {sessions:true})` returns `[]` (both members are
already `intended`), and nothing happens. The one non-empty set is the off
cascade, which the switch can never ask for. The new window shell has no
per-member switch, so the only way back is the master routing switch or the CLI.

Two wrongs in one row: the status line claims "Not routed" over traffic Gate is
decrypting (`group.routed === 2` in the same object), and the switch flips
nothing.

`sectionStatus` reaches "Off" because `governed = group.members.filter(m => m.cascade)`
is empty, so neither the `routing === governed.length` branch nor the
`routing > 0` branch can fire (`verdict.ts:158-161`).

**Fix:** `sectionStatus` and the switch have to answer for the members the click
actually moves, not for the brokered subset. The narrow fix is to make
`cascadeDesired` (and `governed`) fall back to every member when a section has no
brokered member; the honest fix is to derive the rendered state from
`cascadeTargets` itself, so "on" means `cascadeTargets(g, true, …).length === 0`
and "off" means the off set is empty, which is the only definition that cannot
disagree with the click. Either way, add a `groups.test.ts` case for a section
whose members are all additive - the existing "reads the switch off the brokered
half" test at `groups.test.ts:178` covers the mixed case, where a brokered member
still exists to flip, and passes for the wrong reason.

### H2. `cascadeDesired > 0` is an ANY, so a partly-on section renders fully on and the only click available is the wrong one

`src/lib/groups.ts:824`, `src/NewUiApp.tsx:1513`, `src/TrayApp.tsx:720`,
`src/screens/FamilyPanel.tsx:110`

The same mismatch without the dead end. `anthropic` ships `enabled: true`
(`crates/core/src/proxy/catalog.rs:81`), so on a machine with Claude Code
installed but not yet routed:

```
members            claude-code:false  anthropic:true  claude-web:false
cascadeDesired     1            -> switch renders ON
cascadeTargets(on, sessions)  [ 'claude-code', 'claude-web' ]
cascadeTargets(off, sessions) [ 'anthropic' ]
```

The Claude row shows the switch ON (and, before the sweep answers, "Not protected
- Checking", then "Not routed - Blocked" or "Partly routed"). The user wants
Claude Code routed. There is no click that does it: the switch is already on, and
the single click it accepts asks `next = false`, which disconnects
`anthropic` - the one surface that was working. They have to turn Claude's
desktop routing off and then on again, re-answering the session-consent dialog,
to get the CLI routed.

This is the sentence `groups.ts`' own header and principle 2 are about, one layer
up: the rendered state and the acted-on set are computed from different
predicates over different subsets.

**Fix:** as H1 - one predicate. `on` should mean "there is nothing left for this
switch to turn on", i.e. `cascadeTargets(g, true, {sessions}).length === 0`, with
`cascadeDesired` retired or redefined to match.

---

## Medium

### M1. `multiProviderSlugs.has(view.slug)` compares a section id against member keys, so the Terminal pane now offers a model card

`src/NewUiApp.tsx:3404`, built at `:1361-1370`

`multiProviderSlugs` is a set of **member keys** (`m.key` for members with
`coversAllProviders`): `opencode`, `openclaw`, `hermes`, `env-proxy`
(`groups.ts:581`, `MULTI_PROVIDER_CLIENTS` at `groups.ts:145`). `view.slug` is a
**section id**. Three of the four happen to collide (`opencode`, `openclaw`,
`hermes` are both), and one does not: the environment channel's member key is
`env-proxy` and its section id is `terminal` (`groups.ts:700-707`).

So `multiProviderSlugs.has("terminal")` is false. `openDomain` is also false
(`env-proxy` is in `tools`, so `openSectionTool` resolves it), and the pane
therefore renders `modelChoice` for the Terminal row. Before this branch
`view.slug` was the member key `env-proxy` and the row was correctly excluded.
The result is a Gate-model picker, with its paid confirmation, on the one row
whose traffic `inject_model_choice` cannot stamp: `client_tool` never returns
`env-proxy` for anything (`crates/core/src/proxy/mod.rs:1431-1443`), so
`gate_models_for("env-proxy")` is consulted for a slug no request will ever carry.

**Fix:** resolve the section's members before asking, e.g.
`sectionMemberKeys(view.slug).some(k => multiProviderSlugs.has(k))`, or key
`multiProviderSlugs` by section id at construction.

### M2. `openSectionTool` does not filter out not-installed tools, so a pane can be keyed on a tool that is not in its own section

`src/NewUiApp.tsx:519`

`sectionMemberKeys(view.slug).find(key => tools.some(t => t.slug === key))` scans
the raw `list_tools` output, which includes `status.kind === "not_installed"`
entries (`TrayApp.tsx:737` filters for exactly those). `buildGroups` excludes
them from members (`groups.ts:763`) and `apps` excludes them
(`NewUiApp.tsx:1448`).

Given: Claude Code not installed, the Claude section drawn from `anthropic` +
`claude-web`. Then `openSectionTool === "claude-code"`, so:

- `openDomain` is false, so `unattributed` is false and the stat tiles are
  presented as a real reading.
- `partialReading` is `undefined` (`NewUiApp.tsx:2571` finds no config **member**),
  so the "these counts cover X" caveat is suppressed.
- `toolActivity` fires a control-plane read filtered to `claude-code`, which by
  construction has no rows, and the pane draws `0 / 0` under a heading reading
  "Claude" while the Claude desktop app's routed traffic is unattributed.
- the model card is offered and stores a preference against a tool that is not on
  the machine; `appFor(apps, "claude-code")` misses, so the picker's own heading
  falls back to a generic name.

That is a number with nothing behind it in the one place principle 6 names.

**Fix:** filter the `find` to `t.status.kind !== "not_installed"`, which makes
`openSectionTool` agree with the section's own membership.

### M3. `routeSection`'s per-member failure collection is dead code in both shells

`src/NewUiApp.tsx:1283-1303`, `src/TrayApp.tsx:765-783`

Neither `setAppRouted` nor `setDomainRouted` throws: both catch everything
internally and report through `onError` (`src/lib/useRouting.ts:333-346`,
`:426-432`). So the `try { … } catch { failed.push(m.name) }` around them never
fires, `failed.length` is always 0, and the "Couldn't route X / <section> is
partly on - the rest of it moved" message can never be shown. The user does get
an error (the generic per-operation one `useRouting` raises), but never the one
sentence that says which surface of the section is now out of step, which is the
whole reason the message was written.

Secondary consequences of the same gap: `routeSection` cannot tell a decline
(`Declined`, e.g. the OpenCode env dialog inside `setAppRouted`) from a success,
and `movedTools` is built from `targets` rather than from what actually wrote, so
`runningApps.offerAfterChange` offers to close apps whose config write failed or
was declined.

**Fix:** have `routeSection` read `setAppRouted`'s existing boolean return and
give `setDomainRouted` one, rather than relying on exceptions that cannot arrive.

### M4. Turning a section off no longer offers to close the apps whose config it rewrote

`src/NewUiApp.tsx:1296`, `src/TrayApp.tsx:777`

`if (next && movedTools.length > 0) await runningApps.offerAfterChange(movedTools)`.
The per-tool path it replaces does it in both directions
(`NewUiApp.tsx:1250-1253`), and the reason applies equally: disconnecting
restores the tool's own config, and a running `claude` or `codex` keeps the Gate
relay URL until it restarts. After turning the Claude section off, Claude Code is
still pointed at a route the user just switched off, with nothing on screen
saying so.

**Fix:** drop the `next &&` guard, matching `routeApp`.

### M5. The e2e fake backend now cascades a family switch over the additive domains, which is the invariant the branch exists to enforce

`e2e/install.ts:82`, `:427`, `:441`

`Provider.domain_slugs` changed meaning on this branch: it used to be "what the
family switch flips" and is now "every domain in this family, cascaded or not",
with `cascade_domain_slugs` carrying the former meaning
(`crates/core/src/provider.rs:50`, `:295-299`). The fake backend's three readers
were not updated:

- `provider_enable` (`:427`) sets `enabled = true` for every slug in
  `domain_slugs`, so `provider_enable("anthropic")` in the fake routes
  `claude-web` - the user's claude.ai session. The Rust path refuses this, and
  `claude_web_is_not_reachable_by_enabling_the_anthropic_provider`
  (`provider.rs:1774-1790`) is the test that pins it.
- `provider_disable` (`:441`) mirrors it.
- `syncProvider` (`:82`) computes `p.enabled` from all `domain_slugs`, while the
  real `proxy_domains_enabled` reads only `cascade_domains`
  (`provider.rs:252-264`). In the fake, enabling `chatgpt` alone makes the
  `openai` provider read enabled; in Rust that provider's cascade is empty, so it
  never can.

The shipped frontend no longer calls `provider_enable`
(`e2e/routing.spec.ts:259` asserts as much), so this is not a live product bug -
it is that the suite now models the opposite of the rule and would not catch a
regression that reintroduced the call.

**Fix:** use `cascade_domain_slugs` in all three places. The field is already on
the fixture (`e2e/backend.ts:522`, `:537`).

### M6. `client_tool` and `classify_client` read `originator` and the `oai-*` markers in opposite order

`crates/core/src/proxy/mod.rs:1449` vs `:2282-2306`

`classify_client`: `originator` present -> `App`, checked **before** the
`oai-device-id` / `oai-client-version` / `x-openai-target-route` set, which yields
`Web`.
`client_tool`: `openai_web(headers).or_else(|| chatgpt_app(headers, domain))`, so
the `oai-*` set is checked **first** and yields `chatgpt-web`; `originator` only
answers when none of them is present.

A single request carrying both is therefore routed as the desktop app
(`rules_for_client` leaves the entry's full rewrite prefixes, and `engine`'s
Cloudflare handling is gated on `ClientClass::App`, `:2229-2230`) and recorded as
the website. That puts the ChatGPT desktop app's turns into the `chatgpt-web`
series, which is exactly the "two vendors' traffic in one series" failure
`CHATGPT_WEB_CLIENT`'s own doc says the split exists to prevent (`:1550-1551`).

Today the two agree only because of an empirical claim - "`oai-*` emitted by the
website and never by the app in any capture" (`:2296`) - that neither function
asserts and that the app's shell, which renders the same chatgpt.com front-end,
could falsify with one build.

Second half of the same asymmetry: `chatgpt_app` is scoped to
`CHATGPT_HOST_DOMAINS` and is correctly ignored off chatgpt.com
(`originator_is_ignored_off_chatgpt_com`, `:4082`), while `openai_web` reads no
domain at all. `client_tool(h_with_oai_device_id, Some("anthropic"))` returns
`"chatgpt-web"`, which is the cross-vendor mislabelling the same test exists to
prevent for the other header. No test pins it.

**Fix:** make the two precedence chains one order, and scope `openai_web` to the
matched entry the way `chatgpt_app` already is. If the current order is
deliberate, say which of the two is authoritative and add a test that asserts
what a both-headers request produces on each side.

### M7. The rail and the pane draw different brand marks for the same row

`src/NewUiApp.tsx:1514` vs `:3351`, `src/TrayApp.tsx:721`

The rail takes `brandMarkFor(g.members[0]?.key ?? g.id)` (a member key); the pane
header takes `brandMarkFor(view.slug)` (a section id). `BRAND_BY_SLUG`
(`src/components/gc/BrandMark.tsx:86-102`) is keyed by member key, so:

| section | rail mark | pane mark |
|---|---|---|
| `claude` | Claude Code | none (falls back to the initial) |
| `chatgpt` | Codex | OpenAI knot (via the `chatgpt` **domain** slug that collides with the section id) |
| `openai-api` | OpenAI knot | none |

`chatgpt` lands on a plausible mark only because the section id happens to equal
a catalog domain slug, which is the collision the branch warns about elsewhere
(`TrayApp.tsx:795-801`).

The rail's own expression is unstable too: `members[0]` is the first **surviving**
member, so uninstalling Claude Code silently changes the Claude row's icon from
the Claude Code mark to the Claude starburst.

**Fix:** one resolution for both, e.g. a `brandMarkForSection(id)` that walks
`sectionMemberKeys(id)` in order and returns the first mark it finds.

---

## Low

### L1. `routeSection` has no in-flight guard of its own, and `routing.busy` is released between members

`src/lib/useRouting.ts:189-196`, `src/NewUiApp.tsx:1283`

`settle()` calls `setBusy(false)` before the resync, by design. Each member of a
cascade goes through its own `setBusy(true)` / `settle()` pair, so between
members `routingBusy` is false and every switch in the window is live again. A
click landing in that window starts a second `routeSection` concurrently: after
member one of the Claude on-cascade lands, `cascadeDesired` becomes 1 and the
switch flips to on mid-cascade, so a second click asks for `next = false` and the
two cascades interleave, leaving the section in a state neither asked for.

The inner `busy` guard does not catch it: `routeSection` holds the `routing`
object captured at click time, whose `setAppRouted` closes over `busy === false`
from that render, so the guard reads a stale false for every member. That is what
makes the cascade work at all, and it is also why it cannot protect against a
second cascade.

Narrow, since it needs a click in a sub-second gap, but a `useRef` in-flight flag
on `routeSection` would close it outright.

### L2. `session_routing_accepted` keys on a frontend-only identifier

`crates/core/src/preferences.rs:80-102`, `src/NewUiApp.tsx:1547`

The recorded consent is the section id, and section ids exist only in
`SECTIONS` (`src/lib/groups.ts:637-723`). Renaming `chatgpt` to
`chatgpt-codex` silently discards every recorded consent and re-asks, and adding
a new additive member to an already-accepted section never asks at all
(`needsSessionConsent` is a per-section boolean, not per member). Neither is
wrong today; both are worth a line in the field's doc, which currently says
"keyed by section id" without saying that the backend cannot validate one.

### L3. `SessionConsentDialog` names surfaces the user cannot identify

`src/components/gc/dialogs.tsx:1709-1745`, `src/lib/groups.ts:887`

`sessionMembers(group).map(m => m.name)` yields the rail's one-word surface
labels, so the ChatGPT dialog reads "This also routes chat and subscription,
which Gate sees on the account you are already signed in with." The dialog is the
one place a person is told their signed-in session is about to be routed, and
"chat and subscription" names nothing on their machine. `MEMBER_HINTS`
(`groups.ts:135-163`) already carries the sentence that would.

### L4. `groups` recomputes on every `providers` change for no reason

`src/NewUiApp.tsx:1331-1350`, `src/TrayApp.tsx:551-560`

`buildGroups` no longer takes `providers` (`src/App.tsx:960`, `:1212`,
`src/screens/Home.tsx:105`), but `providers` is still in both `useMemo`
dependency arrays. Harmless, but it rebuilds the whole ledger on every provider
poll.

### L5. A collapsed `master-off` / `needs-trust` notice reaches only one section's pane

`src/lib/notices.ts:140-157`, `src/NewUiApp.tsx:2342`

`buildNotices` emits one notice for these two causes and gives it the
`memberKey` of the first member that carried it. `paneNotice` matches on
`sectionMemberKeys(view.slug).includes(n.memberKey)`, so only the section owning
that member draws the card; every other affected section's pane draws nothing,
while the comment at `:2340` reads as if the notice speaks for all of them. This
predates the branch (the lookup used to be on the member key directly) but the
comment is new and overstates what the code does.
