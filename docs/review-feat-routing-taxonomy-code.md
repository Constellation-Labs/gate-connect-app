# Code review: `feat/routing-taxonomy` (code-quality lens)

Base `origin/feat/new-app-ui`, 55 files, +3712/-1315. Reviewed for readability,
naming, dead code, over-engineering, duplication, missing error handling at
boundaries, test coverage and comment hygiene. The branch is in good shape on
the axis it cares most about: `crates/core/src/taxonomy.rs` is a genuinely
smaller model than the `chat_domain_slugs` array it replaces (which is fully
gone from Rust, not left as dead code), `buildGroups` can no longer drop a
catalog entry, and `src/lib/groups.test.ts` covers the new ledger functions
closely - 1012 unit tests pass and `tsc --noEmit` is clean. The cost lands in
four places. First, `Client`'s serde wire names disagree with its own `slug()`
for three of eight variants, so the frontend's `ClientId` union is wrong at
runtime and two rows silently lose a behaviour; every fixture hardcodes the
`slug()` spelling, which is why nothing is red. Second, the section abstraction
carries a structural hole: a section whose members are all
`Credential::Additive` renders its switch permanently off and cannot be turned
on, which the ChatGPT / Codex section reaches on any machine without the Codex
CLI. Third, the two shells now hold a 35-line verbatim copy of the cascade -
error copy included - which the tray's own comment predicts will diverge.
Fourth, comment hygiene slipped in a specific, repeated way: five places gained
a new doc block stacked above an existing one instead of replacing it, leaving
the old block documenting nothing and the new symbol documented by the wrong
text.

## H - must fix

### 1. `Client`'s serde wire names disagree with `Client::slug()` for three variants
`crates/core/src/taxonomy.rs:116-137, 141-152`

`#[serde(rename_all = "kebab-case")]` splits each variant on its internal
capitals, so `ChatGpt`, `OpenCode` and `OpenClaw` go over the wire as
`"chat-gpt"`, `"open-code"` and `"open-claw"`, while `slug()` returns
`"chatgpt"`, `"opencode"` and `"openclaw"`. Confirmed by running
`serde_json::to_string` over `Client::ALL` against this crate's own serde:

```
claude-code -> "claude-code"    chatgpt  -> "chat-gpt"
codex       -> "codex"          opencode -> "open-code"
hermes      -> "hermes"         openclaw -> "open-claw"
```

`Client` is serialized directly on both row kinds - `Tool.client`
(`src-tauri/src/lib.rs:81, 161`) and `ProxyDomain.client`
(`crates/core/src/proxy/mod.rs:1729-1730`) - and the frontend was written
against the `slug()` spelling:

- `src/lib/api.ts:50-58` declares `ClientId` as `... | "chatgpt" | "opencode" |
  "openclaw" | ...`. The type is a lie at runtime, and `tsc` cannot see it.
- `MULTI_PROVIDER_CLIENTS.has(tool.client)` (`src/lib/groups.ts:145-150`, used
  at `:581`) returns false for the OpenCode and OpenClaw tools, so
  `coversAllProviders` is dropped and both panes render a model card for a tool
  that routes whatever providers the user configured in it. That set is the one
  thing on this branch still keyed by client string.
- `src/lib/diagnosticsReport.ts:373, 395` prints `open-code` to whoever is
  reading the report.

Nothing is red because every fixture hardcodes the `slug()` spelling
(`e2e/backend.ts:443, 465`, `src/lib/groups.test.ts:128-129, 159`), and the CLI
is unaffected because `crates/cli/src/main.rs:772` goes through
`d.client.slug()`. So the defect is invisible to the whole suite and visible
only on a real backend.

Fix: per-variant `#[serde(rename = "chatgpt")]` / `"opencode"` / `"openclaw"`,
or implement `Serialize`/`Deserialize` off `slug()`. Then pin it - assert
`serde_json::to_string(&c) == format!("\"{}\"", c.slug())` for every
`Client::ALL` entry. Nothing pins this today, which is why it shipped.

### 2. A section with no brokered member has a switch that can never read on
`src/lib/groups.ts:824`, `src/lib/verdict.ts:158-162`,
`src/NewUiApp.tsx:1513`, `src/TrayApp.tsx:720`

`Group.cascadeDesired` counts `m.cascade && intended(m)`, and `cascade` is
`credential === "brokered"` (`groups.ts:570`, `:608`). Both shells render the
section switch from `on: g.cascadeDesired > 0`, and `sectionStatus` gates its
"protected" branch on `governed.length > 0` where `governed` is the same
brokered filter.

The ChatGPT / Codex section's only brokered member is the `codex` config tool
(`groups.ts:664-673`); `chatgpt-apps` and `chatgpt` are both
`Credential::Additive` (`crates/core/src/proxy/catalog.rs:343`, `:401`). On a
machine with no Codex CLI, `buildGroups` drops the tool
(`groups.ts:760`) and the section is left with two additive members. Then:

- `cascadeDesired` is 0, so the switch renders off however the two hosts are set.
- `sectionStatus` skips both the protected and the partly-routed branch and
  returns `not-routed / "Off"` over two surfaces that may be routing.
- Clicking it on runs the consent dialog and `routeSection`, which does route
  both hosts - and the switch snaps straight back to off, because
  `cascadeDesired` is still 0.
- Clicking again does nothing at all: consent is recorded, and
  `cascadeTargets(section, true, { sessions: true })` now returns `[]` because
  both members are already `intended`.

This is principle 2's bug in a new shape and principle 6's too - the row states
"Off" about traffic it is carrying. It is not the known drifted-tool case: this
happens with nothing drifted and no tool present at all.

Fix: make the switch and `sectionStatus` read the set the switch actually
flips. Since `routeSection` always passes `sessions: true`, that set is every
member, not the brokered half. Either add the consent-aware count to `Group`
(a `switchDesired` computed the way `cascadeTargets` filters) or fall back to
`desired > 0` when `governed.length === 0`. Whatever is chosen, `cascadeDesired`
and `cascadeTargets(..., { sessions: true })` must agree about which members
define the switch, or the switch and the click keep disagreeing.

### 3. `openSectionTool` skips the `not_installed` filter the ledger applies
`src/NewUiApp.tsx:517-521`

`openSectionTool` resolves the section's config tool from raw `tools`
(`tools.some((t) => t.slug === key)`), while `buildGroups` includes a tool only
when `tool.status.kind !== "not_installed"` (`src/lib/groups.ts:760`). For an
open section whose tool is present in the catalog but not installed - the
Claude section on a machine without Claude Code - the two disagree:

- `openTool` is `"claude-code"`, so every per-tool read fires for a tool that
  can never have traffic, and the pane reports a quiet day. That is exactly the
  failure the retained comment at `:495-502` describes.
- `openDomain` is `false`, so `AppPane`'s `unattributed` flag is off.
- `partialReading` (`:2568-2576`) resolves its tool from the built ledger
  instead, so it is `undefined` and the "These counts cover X" caveat is
  suppressed.

Net effect: a section whose figures are structurally empty renders zeroes with
no caveat and no unattributed marker. Resolve `openSectionTool` from the same
filtered set - the built ledger's members, or `tools.filter(t => t.status.kind
!== "not_installed")` - so one predicate decides it.

### 4. Fake backend derives provider `enabled` from the wrong field
`e2e/install.ts:82-85`

`syncProvider` computes `domainsOn` from `p.domain_slugs`, but on this branch
`domain_slugs` means every domain in the family (`e2e/backend.ts:83-87`) while
`crates/core/src/provider.rs:252-263` derives `enabled` from `cascade_domains`
only. In e2e, enabling `chatgpt-apps` alone flips the `openai` provider to
`enabled: true`; the real backend cannot do that, since openai's cascade is
empty. Read `p.cascade_domain_slugs` instead - this is the exact field the
branch redefined, so the fixture is now green on a state the product forbids.

## M - should fix

### 5. `routeSection` is duplicated verbatim across both shells, error copy included
`src/NewUiApp.tsx:1270-1306`, `src/TrayApp.tsx:754-790`

The two bodies are identical apart from one comment sentence: the same
`cascadeTargets` call, the same single CA gate, the same `failed` loop, the same
`movedTools` derivation and the same two user-facing strings ("Couldn't route
X" / "X is partly on - the rest of it moved..."). The tray's own doc says
"`NewUiApp.routeSection`'s twin, and it has to be", but nothing forces the
duplication: both shells already share `useRouting` and `lib/groups.ts`, which
is where every other shared rule on this branch lives. Copy in two files is the
one thing the dialog comment at `dialogs.tsx:1697-1700` explicitly argues
against for the consent question.

It has already started drifting: `NewUiApp` clears `actionError` inside the
`if (section)` branch (`:1550`) while `TrayApp` clears it before
(`TrayApp.tsx:815`).

Fix: lift it to `useRouting` as `setSectionRouted(section, next)` beside
`setFamilyRouted`, or to a `lib/routeSection.ts` taking the `routing` and
`runningApps` handles. The same applies to the `SessionConsentDialog`
`onConfirm` body (`NewUiApp.tsx:3026-3042` / `TrayApp.tsx:1146-1162`), which is
also character-identical.

### 6. `routeSection` discards every underlying error
`src/NewUiApp.tsx:1284-1289, 1297-1303` and the tray twin

`catch { failed.push(m.name) }` throws the error object away, and the
`ClassifiedError` built from it sets `raw: ""`. `ClassifiedError.raw`
(`src/lib/errors.ts:67`) is what the Details disclosure and the diagnostics
report read, so a section cascade is the one routing path in the app that can
fail and leave nothing for anyone to debug - not a message, not a backend
string. Every other caller runs the failure through `classifyError`.

Keep the first error (or join them), classify it, and put the real text in
`raw`. Related, same lines: `movedTools` is derived from `targets` rather than
from the members that succeeded, so `offerAfterChange` is called for a tool
whose write just failed.

### 7. Five new doc blocks were stacked on top of the old ones
`src/lib/groups.ts:297-317`, `src/NewUiApp.tsx:495-502`,
`src/NewUiApp.tsx:2549-2558`, `crates/core/src/proxy/engine.rs:1560-1580`,
`src-tauri/src/lib.rs:2400-2405`

The same mistake five times, in both languages: a new symbol or a new `/** ...
*/` was wedged between an existing doc comment and the symbol it described. The
earlier block now documents nothing, and the newer symbol is documented by text
about something else.

- `groups.ts:297-317` is the old `chatScopeNote` doc ("What a chat row's switch
  covers..."). The block that follows documents `scopeNote`, and
  `credentialScopeNote` at `:354` - the function the orphan is actually about -
  has no doc at all.
- `NewUiApp.tsx:495-502` is the old `openDomain` doc, now sitting above
  `openSectionTool`'s own block; `openDomain` at `:521` has none.
- `NewUiApp.tsx:2549-2558` describes `rowScope` ("How wide this row
  reaches...") but sits above `partialReading`'s block, so it reads as
  `partialReading`'s preamble and `rowScope` at `:2578` has none.
- `engine.rs:1560-1580` is `apply_rewrite`'s doc ("Repoint a request at the
  gateway..."). `struct MatchedRoute` was inserted at `:1589` between it and
  `apply_rewrite` at `:1598`, so the struct carries three paragraphs about
  header injection and path stripping, and `apply_rewrite` is undocumented.
- `src-tauri/src/lib.rs:2400-2402` is `set_share_diagnostics`' doc; the new
  `accept_session_routing` command was inserted at `:2406` between it and
  `set_share_diagnostics` at `:2412`, so the new command carries two unrelated
  doc paragraphs and the old one has none.

Move each block to the symbol it describes, or delete it where the newer block
supersedes it. This matters more than usual in a codebase that carries this
much rationale in comments: a reader who trusts the block above a symbol is
being misdirected, and four of the five orphans are long enough to be believed.

### 8. `sectionStatus`' doc promises severity ordering the code does not do
`src/lib/verdict.ts:141-162`

The doc says "the strictest member wins, in the order a user would care" and
names "an error, drift, an untrusted certificate" as rule 1. The code does
`known.find((l) => l.kind === "drifted" || l.kind === "not-protected")` - array
order over the section's draw order, with no severity ranking at all. Two
concrete divergences:

- A member whose verdict has not landed yet is `{ not-protected, "Checking" }`
  (`verdict.ts:83`). If it is drawn first, the section reports "Checking" over a
  sibling that is genuinely drifted.
- An untrusted certificate never reaches this branch for a proxy member:
  `proxyMemberStatus` returns `{ not-routed, "Blocked" }` for it
  (`verdict.ts:115-118`), which the `find` does not match. The doc's third
  example is therefore false.

`groupSummary` in `groups.ts:906-971` already does the explicit severity ladder
for the same data. Either reuse that ordering here or correct the doc to say
"the first member with a non-routing line".

### 9. The session CA memo is never reconciled with the real trust state
`src/lib/useRouting.ts:210, 216, 226, 516`

`caTrustedThisSession` short-circuits `ensureCaTrusted` for the life of the
window and is cleared in exactly one place: `untrustCa`. Any other way
`proxy.ca_trusted` goes back to false - the reset flow, a CA removed from the
store out of band, a `certutil -D` - leaves the ref stuck true, and every
subsequent routing click silently skips the trust install for a certificate
that is not there.

The documented motivation is one cascade ("a section switch makes up to three
in a row"), not one session. Scope it to that: have `routeSection` do the trust
once and pass a "already gated" flag down, or clear the ref in an effect when
`proxy.ca_trusted` transitions to false.

### 10. `const roster = false` leaves a dead branch and 20 lines of comment
`src/screens/Home.tsx:876-881, 967`

The roster was removed by assigning a literal `false` to the variable that
gated it, so `secondLine` at `:881` folds to `!!exception`, the `{roster && (
... )}` block at `:967` is unreachable, and the comment block at `:860-875`
still argues about a measurement of rosters that can no longer be drawn.
Neither `tsc` nor the test suite flags it. Delete the variable, the JSX block
and the superseded comments, keeping only the one-sentence note about why there
is no group named by exclusion any more.

### 11. `GroupMember.description` is written and read nowhere
`src/lib/groups.ts:446-452, 540, 593`

Both member constructors call `describeMember` to populate `description`, and
nothing consumes it: the pane now calls `describeSection(view.slug)`
(`NewUiApp.tsx:3350`), and `rg` finds no other reader. The field's own doc still
claims it is "carried on the member so the rail, the pane and the family panel
cannot disagree about it", which is no longer how any of the three get it.
Drop the field and the two calls, or wire the pane back through it - but not
both.

### 12. Most of `MEMBER_HINTS` is now unreachable
`src/lib/groups.ts:118-140`, `src/NewUiApp.tsx:1516`, `src/TrayApp.tsx:723`

Both shells take a section's hover as `g.members.map((m) => m.hint).find(Boolean)`
- the first member with a hint, in draw order, which is tools first. So the
Claude row shows "Claude Code CLI and IDE plugins" and the ChatGPT row shows
"Codex CLI and IDE extension", and the four entries the table works hardest to
justify - `anthropic`, `claude-web`, `chatgpt-apps`, `chatgpt`, each with a
comment explaining its per-surface scoping - are rendered nowhere. Their whole
stated purpose was naming Cowork and Work, which the section hover now never
does.

Separately, `hintForMember(t.slug)` at `NewUiApp.tsx:1463` and
`TrayApp.tsx:667` feeds `apps`, whose entries are consumed by `sectionStatus`
and then deleted from `bySlug`; only the unreachable "unclaimed" group would
render them. Either compose the section hover from all its members' hints, or
delete the four unreachable entries and the per-tool `hint` on `apps`.

### 13. Four section descriptions duplicate their member description verbatim
`src/lib/groups.ts:55-83` vs `:637-722`

`openclaw`, `hermes`, `terminal`/`env-proxy` and `openai-api`/`openai` carry
byte-identical strings in `MEMBER_DESCRIPTIONS` and in `SECTIONS[].description`
- the `openai` one is 137 characters copied exactly. `describeSection` already
falls back to `describeMember(id)` (`:853`), so a single-member section could
just omit its `description` and let the fallback answer, except that the
fallback keys on the section id and three of the four ids differ from the
member key. Either key the fallback off `sectionMemberKeys(id)[0]` and delete
the four duplicates, or add a comment saying the copies are deliberate. As it
stands, editing one and not the other is silent.

### 14. `cascade: boolean` flattens a three-variant `Credential`, then is read as "additive"
`src/lib/groups.ts:466-477, 883, 888`

`cascade` is `credential === "brokered"`, and three call sites read `!cascade`
as though it meant `additive`: `needsSessionConsent` (`:883`),
`sessionMembers` (`:888`) and `NewUiApp.tsx:2538`. `Credential` has a third
variant, `observed` (`src/lib/api.ts:28`, `taxonomy.rs`), whose whole stated
purpose is that "inspected" and "credential swapped" are not forced to share a
sentence. An `observed` row would be named in the consent dialog as a surface
"you are already signed in with" and would get `credentialScopeNote`'s "carries
the credential you're already signed in with" - both false for it.

Nothing ships `observed` today, which is why this is M and not H. The cheap fix
is to test `m.credential === "additive"` at those three sites and leave
`cascade` meaning only what its name says.

Related: `GroupMember.cascade`'s doc at `:466-476` states "only a brokered row
cascades", which `cascadeTargets(..., { sessions: true })` now deliberately
breaks. The exception is documented inside `cascadeTargets` but not on the
field, so the field's doc reads as an invariant that no longer holds.

### 15. `toggleRailApp`'s doc describes the shell it replaced
`src/NewUiApp.tsx:1535-1537`

"The rail mixes tools and proxy domains now: a domain routes through
`setDomainRouted`..." describes the fallback path at `:1555-1568`, which is no
longer how any rail row dispatches - every row's slug is a section id and the
section branch returns at `:1552`. The fallback's `member?.kind === "proxy"`
half is now only reachable while `proxy` is still null and `groups` is empty,
where `groups.flatMap(...)` cannot find a member either, so it always takes the
`routeApp` arm. Rewrite the doc to lead with the section cascade, and consider
reducing the fallback to the `routeApp` call it actually makes.

### 16. `accept_session_routing` has no test on any level
`e2e/install.ts:500`, no spec; no component test for `SessionConsentDialog`

The branch's headline safety mechanism - consent recorded so the question is
asked once - is stubbed in the fake backend and asserted nowhere. `rg` finds no
reference outside `install.ts`/`backend.ts`, and `dialogs.tsx`'s
`SessionConsentDialog` has no component test. A regression that routes but
never records, so the dialog returns on every switch, passes the whole suite.
Add: accept, assert `lastCall("accept_session_routing")` is
`{ section: "claude" }`, then toggle the section off and on again and assert no
dialog.

### 17. `confirmCaTrusted` and the session memo are untested
`src/lib/useRouting.ts:244-256`, `src/lib/useRouting.test.tsx`

`useRouting.test.tsx` exists and was touched by this branch (fixture fields
only). Neither `confirmCaTrusted` nor `caTrustedThisSession` has a test, and
the bug they fix - three certificate dialogs for one cascade - is precisely the
kind that returns silently. Two tests: a second `ensureCaTrusted` in the same
session does not re-ask, and `untrustCa` makes it ask again.

### 18. e2e: assertions that no longer test what they are named for
`e2e/new-ui-routing.spec.ts:1072-1081, 996-1014, 1031-1033, 1048`

- `:1072` "a band's eyebrow counts protected rows over rows" asserts only
  `/of \d+$/`, which passes for `0 of 0`. The old version asserted `"0 of 3"`,
  flipped a switch, then asserted `"1 of 3"`. Restore an exact count and the
  route-then-recount step; the section membership is fixed by `SECTIONS`, so an
  exact string is safe.
- `:996` "an app switch routes every surface that app uses" asserts the two
  domains and not the `codex` config tool, which the comment one line above
  names as the third member. The config half of the cascade could stop firing
  and this stays green.
- `:1031` reads the call log immediately after "Not now" - two unsettled
  negatives. Settle on something positive first (dialog gone, switch still
  `aria-checked="false"`).
- `:1048` asserts `/These counts cover/` and drops the surface name, which is
  the entire point of commit `0ff113af`. `AppPane.test.tsx:92` already asserts
  `/These counts cover Claude Code/`; match that specificity.

### 19. e2e fixtures make the unsafe taxonomy value the default
`e2e/backend.ts:101-103`, `e2e/install.ts:250-252, 376-377`

`client`/`scope`/`credential` are optional on `DomainFixture`, and `install.ts`
defaults them for tools only - `proxy_status` returns domains raw. An omitted
`credential` reaches `groups.ts:609` as `undefined`, so `cascade` is `false` and
the row silently becomes a consent-gated session surface, the opposite of the
common case. `e2e/routing.spec.ts:189-191` already had to hand-add the three
fields to one literal to work around it. Make them required on `DomainFixture`
(and type `client` as `ClientId` rather than `string`), or default them the way
tools are.

### 20. e2e: the fixture catalog is missing two shipped domains, and two tests depend on the gap
`e2e/backend.ts:501-507`, `e2e/new-ui-routing.spec.ts:381-386`,
`e2e/new-ui-tray.spec.ts:195-197`

The fixture carries 5 of the real catalog's 7; `openrouter`
(`catalog.rs:406`) and `opencode` (`:442`) are missing, both `supported: true`.
`opencode` is a member key of the OpenCode section (`groups.ts:692-696`), so on
a real machine that section exists with no tool installed - which is exactly
the premise the two tests deny ("Nothing in the default catalog belongs to the
OpenCode section"). Add the two domains and move those tests onto a section
that is genuinely empty.

### 21. Three hand-typed copies of one slug table, with a doc claiming otherwise
`crates/core/src/proxy/mod.rs:1425-1441`, `crates/core/src/taxonomy.rs:141-152`,
`crates/core/src/registry.rs:18-27`

`client_tool`'s doc (`mod.rs:1414-1416`) says "Slugs are `Client` slugs, which
the tool ones coincide with by construction". They coincide by retyped string
literal: the user-agent array holds `("codex", "codex")`,
`("opencode", "opencode")`, `("openclaw", "openclaw")` while the same function
reaches for `Client::ChatGpt.slug()` at `:1477` and `Client::ClaudeDesktop.slug()`
at `:1512` and `:1519`. Finding 1 is what this costs: a rename that splits the
wire name from `slug()` also splits the attribution column, and half of it
would still be right. Use `Client::Codex.slug()` and friends in the array.

### 22. `CHATGPT_HOST_DOMAINS` is a new hand-kept slug list with no test
`crates/core/src/proxy/mod.rs:1488`

Its own doc says "A new chatgpt.com entry must be added here or its app traffic
goes unattributed" - which is exactly the hand-kept-array failure mode this
branch exists to retire, reintroduced three commits later. It is derivable:
`default_domains()` filtered to entries whose `hosts` contain `chatgpt.com`.
Either derive it or add a test asserting the two agree. `rg` finds only the
definition and its single use.

### 23. `display_name()`, `vendor()` and `ALL` are called only by their own tests
`crates/core/src/taxonomy.rs:156-197`

Across `crates/`, `src-tauri/` and `src/`, the only non-test caller of anything
on `Client` is `slug()`. Their docs assert "The group heading", "the ledger
group id" and "in the order the ledger draws them", but the ledger's headings,
ids and order all come from the hand-kept `SECTIONS` table at
`src/lib/groups.ts:637-720` (ids `claude`, `chatgpt`, `terminal`,
`openai-api`), which never groups by `member.client` - it only passes the field
through at `:568` and `:606`. So the module's headline claim, "it is what the
ledger groups by" (`taxonomy.rs:106-107`), is not true of the shipped UI.
Either wire the ledger to `Client` or delete the three and correct the docs.

### 24. The catalog's taxonomy fields are asserted nowhere
`crates/core/src/proxy/catalog.rs` (no `#[test]` in the file)

`catalog.rs` has zero tests, and the one cross-check that exists
(`provider.rs:1805-1826`) only walks the slugs the three providers list. No
entry's `client` or `scope` is asserted anywhere, and the `openai`,
`openrouter` and `opencode` entries' `credential` is unpinned - the field that
now decides whether a row can ride a family switch. A single entry flipped from
`Additive` to `Brokered` would route somebody's signed-in session with the
whole suite green. Add a catalog invariant test beside
`every_resolved_endpoint_lands_on_an_inference_prefix`.

### 25. `ProxyDomain`'s `scope` doc argues for the opposite of what the catalog does
`crates/core/src/proxy/mod.rs:1737-1743`

It reads "Not a constant even though most entries are `Scope::Host`: `chatgpt`
is reached by Codex through the loopback relay... An entry that is only ever
relayed is `Scope::Client`." But `catalog.rs:389-403` decides the reverse in as
many words ("Host, not Client, even though Codex arrives through the relay"),
and all seven catalog entries are `Scope::Host`. The field is constant across
the catalog; only the integrations vary (`env_proxy.rs:82`). Rewrite the
justification around the integration seam, or the next person to add an entry
will follow the doc into contradicting the catalog.

### 26. "Unreachable in practice" is wrong, and the serde defaults are load-bearing
`crates/core/src/proxy/mod.rs:1745-1748`

The comment justifies `default_client` / `default_credential` / `default_scope`
as unreachable because "every `ProxyDomain` in the process was built by the
catalog". `ProxyDomain` is also deserialized off the Linux helper-daemon IPC:
`Request::SetIntercept` carries `Vec<ProxyDomain>` (`helper_client.rs:217-222`)
and the daemon deserializes it at `helper.rs:289-303`. That daemon is detached
and outlives the GUI, so a GUI/daemon build skew is precisely when these fire -
and `Credential::Observed` as the fallback is the right answer for exactly that
reason. The defaults are correct; the comment talks the next reader out of
them. Say the daemon instead.

### 27. The new attribution values cannot be read back
`crates/core/src/activity.rs:98, 151`, `crates/core/src/registry.rs:20-25`

`overview_json` and `tool_events_json` both take `tool: ToolId`, and
`ToolId::slug()` covers only `claude-code, codex, opencode, openclaw, hermes,
env-proxy`. The four `client_tool` values this branch starts emitting -
`claude-desktop`, `claude-web`, `chatgpt`, `chatgpt-web` - have no `ToolId`, so
no query can filter on them. The desktop-app attribution the last four commits
added is write-only from the app's side. That may be intentional for now, but
`client_tool`'s doc should say so, since the pane's `partialReading` caveat
(`src/components/gc/AppPane.tsx:205-222`) is the user-visible consequence.

## L - nits

- `src/NewUiApp.tsx:1348`, `src/TrayApp.tsx:567`: `providers` is still in the
  `groups` memo's dependency array after `buildGroups` stopped taking it. Dead
  dep, recomputes the ledger on every provider poll.
- `src/lib/groups.test.ts:299`: `sectionStatus` lives in `src/lib/verdict.ts`
  but its tests sit in `groups.test.ts` and import from `"./verdict"`, while
  `src/lib/verdict.test.ts` exists and does not cover it. Move them.
- `src/lib/verdict.ts:143`: the second parameter is a `Map` named `appFor`,
  which is also the name of a real lookup function in both shells
  (`NewUiApp.tsx`'s `appFor(apps, slug)`). Rename to `statusBySlug`.
- `src/lib/verdict.ts:161`: the "Partly routed" branch is only reachable when
  no member's own line is `not-protected`, because the `exception` find at
  `:155` already returns any such line. Worth a word, since the doc presents it
  as rule 3 of 4.
- `src/NewUiApp.tsx:526`: `const openTool = openSectionTool;` is a bare alias
  with its own doc comment. Two names for one value in a 4000-line file; keep
  one.
- `src/NewUiApp.tsx:2570-2574`: `partialReading` names a `GroupMember` `tool`
  and then reads `tool.tool?.product_name`. Rename the local to `configMember`.
- `src/NewUiApp.tsx:1281`, `src/TrayApp.tsx:765`: `confirmCaTrusted` runs even
  when `targets` is empty, so re-routing an already-routed section can raise a
  certificate dialog with nothing to do. Guard on `targets.length > 0`.
- `src/components/gc/dialogs.tsx:1723-1725`: `surfaces.map(lowercase).join(" and ")`
  reads correctly for one or two surfaces and badly for three ("a and b and c").
  Both shipping sections have at most two, so this is latent; an Oxford join
  helper would settle it.
- `src/NewUiApp.tsx:3035` / `src/TrayApp.tsx:1155`:
  `acceptSessionRouting(...).catch(() => {})` swallows a failed consent write
  with no comment. The surrounding comment explains the ordering, not the
  swallow. One line saying "a failed record just means we ask again" would do.
- `src/lib/groups.ts:354-382`: `credentialScopeNote` returns early when
  `member.domain?.hosts` is absent, so the `scope !== "host"` arm at `:377`
  ("It covers X alone.") is only reachable for a proxy member with hosts and a
  non-host scope, which no catalog entry has. Either drop the arm or drop the
  guard.
- e2e comment hygiene, four comments describing the replaced shell:
  `e2e/new-ui-engine.spec.ts:122` (test still titled "a chat domain starts the
  engine" while driving the brokered `OpenAI API` row), `e2e/backend.ts:347-354`
  ("lands under Experimental beside OpenCode"), `e2e/backend.ts:513-515`
  ("rows named 'App' / 'Web' / 'CLI'"), `e2e/new-ui-running-apps.spec.ts:153`
  ("both tools are a 'CLI' there").
- `e2e/backend.ts:398, 434, 456`: `CLAUDE_WEB_DOMAIN`, `CHATGPT_DOMAIN` and
  `CHATGPT_APPS_DOMAIN` are exported but referenced only inside `backend.ts`.
  Pre-existing, worth sweeping while the file is open.
- `e2e/new-ui-routing.spec.ts:392-398` and `e2e/new-ui-tray.spec.ts:201-207`
  inject the same OpenCode tool literal with a drifting
  `default_upstream_url` (`https://gw.example/opencode` vs `https://opencode.ai`).
  `backend.ts` already has an `OPENCODE` fixture; export it.
- `crates/core/src/provider.rs:746`: `cascade_domains(p)` is called inside the
  `for d in domains` loop, rebuilding the whole catalog once per persisted
  domain. `:253` and `:524` already hoist it; this one does not.
- `crates/core/src/provider.rs:200-207`: `cascade_domain_slugs` has no reader.
  `rg` finds it only in the TS type declaration (`src/lib/api.ts:492`) and
  `diagnosticsReport.test.ts`; `domain_slugs` likewise. Two public DTO fields
  nobody consumes - wire the UI to them or drop them.
- `crates/core/src/taxonomy.rs:53-54`: "No entry ships this today" about
  `Credential::Observed` is already false - it is constructed at
  `proxy/relay.rs:122` and returned by `proxy/mod.rs:1758`. Same claim is
  repeated in `src/lib/api.ts:26-28`.
- `crates/core/src/proxy/mod.rs:1477, 1552`: the attribution slugs collide with
  catalog domain slugs - `CLAUDE_WEB_CLIENT` is `"claude-web"`, also the domain
  slug at `catalog.rs:93`, and `Client::ChatGpt.slug()` is `"chatgpt"`, also the
  one at `catalog.rs:348`. Two namespaces, one spelling, on rows the UI keys by
  slug. The constants' doc explains why they are not `Client` values and says
  nothing about the collision.
- `src/lib/api.ts:491`: the doc still names `chat_domain_slugs`, a symbol that
  no longer exists in Rust. `crates/core/src/integrations/openclaw.rs:55` runs
  85 chars after the reflow, against the file's 80-col comment wrap.
- `e2e/fixtures.ts:81-90, 111`: `routeApp` hardcodes `SESSION_SECTIONS` and
  clicks confirm unconditionally for those names, duplicating catalog knowledge
  `state().proxy.domains` already carries and hanging on a second ON of the same
  section in one test. The tests at `:279` and `:300` already work around it by
  hand.

## What is good

- **`taxonomy.rs` earns its place, and the old array is genuinely gone.**
  Replacing a parallel `chat_domain_slugs` with a per-entry `Credential`
  removes a class of bug rather than moving it: the exclusion can no longer be
  forgotten when a catalog entry is added, and both the Rust cascade and the TS
  one read the same field. `rg` finds no surviving exclusion array in Rust -
  only prose references in `provider.rs:38`, `:203` and `taxonomy.rs:31`. The
  deletion was done properly.
- **`buildGroups` cannot drop a row.** The synthesised-section fallback at
  `groups.ts:790-795` is the right answer to "what happens to a catalog entry
  nobody has filed", and the comment says so in one sentence.
- **The intent-versus-flow split survived the refactor.** `desired` versus
  `routed`, and the new `intended()` helper for the drifted case, are carried
  carefully through `group()`, `cascadeTargets` and `sectionStatus`, with the
  reasoning attached where a future reader would otherwise re-introduce the bug.
- **Unit coverage of the new ledger is genuinely good.** `groups.test.ts` grew
  to cover the section shape, consent, the cascade's three rules, the scope
  notes and the hint table, with test names that state the invariant rather
  than the mechanics. 1012 tests pass and `tsc --noEmit` is clean.
- **`useRouting.confirmCaTrusted` is the right shape** - returning "go on or
  not" rather than throwing, with the declined case documented as an answer and
  not an error.
- **The e2e suite was actually retargeted**, not disabled: no `test.skip`,
  no `test.fixme`, no commented-out tests and no new `waitForTimeout`.
