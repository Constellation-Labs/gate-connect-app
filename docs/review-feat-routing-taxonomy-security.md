# Security review: `feat/routing-taxonomy`

Base `origin/feat/new-app-ui`, 55 files, +3712/-1315. Lens: security.

## Summary

The central move of this branch is sound and is a real improvement in the
property it targets. Making `Credential` a field on the catalog row and deriving
the family cascade from it (`crates/core/src/provider.rs:161`) removes the
hand-kept exclusion array that the old comments spent paragraphs warning about,
and every Rust path that could cascade - `enable_inner`, `disable_inner`,
`off_members`, `proxy_domains_enabled`, and through them `restore_all`,
`reconcile_enabled` and `routing::enable` - now filters through `cascade_domains`,
so no additive row can ride a family switch from the CLI, a restore or a
reconcile. The serde defaults on the three new `ProxyDomain` fields fail in the
safe direction (`Observed` does not cascade) and are unreachable from disk
anyway. The deliberate break of the invariant in the window and tray shells is
gated, recorded per section, and fails closed when preferences cannot be read.
What is weaker than the branch's own comments claim is everything downstream of
that: the consent gate is enforced entirely by two `if` statements in the
renderer with nothing behind them in Rust, the e2e fake backend now models the
opposite of the production rule, the host-scope sentence is missing from the
tray entirely and from the window pane on exactly the sections that route a
session, and one realistic install configuration produces a section that routes
the user's chatgpt.com session while the UI says "Off" and offers no way to turn
it back off.

## High

### H1. A section made only of additive rows routes a signed-in session, reports "Off", and cannot be switched off

`src/lib/verdict.ts:158`, `src/lib/groups.ts:824`, `src/lib/groups.ts:763`,
`src/NewUiApp.tsx:1513`, `src/NewUiApp.tsx:1538`, `src/TrayApp.tsx:801`

`buildGroups` drops a tool whose status is `not_installed`
(`src/lib/groups.ts:763`). The `chatgpt` section is
`["codex", "chatgpt-apps", "chatgpt"]`, and `codex` is its only brokered member -
both domains are `Credential::Additive`. So on a machine with the ChatGPT desktop
app but no Codex CLI, that section's members are two additive rows and nothing
else. Three things then line up wrongly:

- `group()` computes `cascadeDesired` from `m.cascade && intended(m)`
  (`src/lib/groups.ts:824`), so it is permanently `0`.
- The rail row's switch reads `on: g.cascadeDesired > 0`
  (`src/NewUiApp.tsx:1513`), so it renders off forever.
- `sectionStatus` computes `governed = group.members.filter((m) => m.cascade)`
  (`src/lib/verdict.ts:158`); with `governed` empty it falls through both the
  `protected` and the `Partly routed` branches and returns
  `{ kind: "not-routed", detail: "Off" }` (`src/lib/verdict.ts:162`).

Concrete sequence. The user clicks the ChatGPT switch. `toggleRailApp` computes
`next = !on = true`, sees `needsSessionConsent` (`src/lib/groups.ts:882`) and
raises `SessionConsentDialog`. The user accepts.
`routeSection(section, true)` runs `cascadeTargets(group, true, { sessions: true })`,
which returns both additive members, and `setDomainRouted` enables `chatgpt-apps`
and `chatgpt`. Gate is now decrypting chatgpt.com and inspecting traffic
authenticated by the person's own session cookie. The rail row still says "Off"
and the switch still renders off, because `cascadeDesired` is still `0`. Every
subsequent click sends `next = true` again, and `cascadeTargets(group, true, …)`
returns `[]` because both members are already `intended` - so the click is a
no-op. The user cannot stop it from the control that started it, in either shell
(`src/TrayApp.tsx:801` is the same code).

This is the exact failure principle 2 in `CLAUDE.md` describes, applied to the
one surface where being wrong means silently intercepting someone's signed-in
session, and it is the inverse of the reassurance the product is selling.

What to do: make the switch's rendered state and `sectionStatus` answer for
every member the switch actually flips, not only the brokered half. The cleanest
form is a second count on `Group` - the members a *consented* cascade governs -
used by `sidebarGroups` and `sectionStatus` whenever `needsSessionConsent` is
true for that section and the section has been accepted. At minimum, a section
with no brokered member must derive `on` and its status from its additive
members, so the switch can be turned off. Add a `groups.test.ts` case for a
section whose only members are additive; there is none today.

## Medium

### M2. The e2e fake backend now cascades additive domains, so the branch's central invariant is unasserted end to end

`e2e/install.ts:427`, `e2e/install.ts:441`, `e2e/install.ts:82`,
`e2e/backend.ts:521`, `e2e/backend.ts:535`

`provider_enable` and `provider_disable` in the fake backend iterate
`p.domain_slugs` and flip every one of them. That was correct while
`domain_slugs` held only the cascaded half. This branch widened the fixture's
`domain_slugs` to the full family - `["anthropic", "claude-web"]` and
`["chatgpt", "chatgpt-apps"]` (`e2e/backend.ts:521`, `e2e/backend.ts:535`) - and
added a separate `cascade_domain_slugs` that the mock never reads. So in every
Playwright run, `provider_enable("anthropic")` turns on `claude.ai` and
`provider_enable("openai")` turns on both ChatGPT session surfaces, which is
precisely what `provider::cascade_domains` exists to forbid. `syncProvider`
(`e2e/install.ts:82`) has the same divergence: it reads a provider as enabled
when only its session surface is on, where Rust's `proxy_domains_enabled`
(`crates/core/src/provider.rs:253`) filters through `cascade_domains` first.

The consequence is not a live vulnerability, it is that the e2e layer would
green-light a regression of the invariant this whole branch is about, and would
also mask the H1 state machine.

What to do: have both mock handlers iterate `p.cascade_domain_slugs`, and have
`syncProvider` read the same field. One line each.

### M3. The host-scope sentence is suppressed on exactly the panes that route a session

`src/NewUiApp.tsx:2578`, `src/NewUiApp.tsx:2533`, `src/lib/groups.ts:341`

`rowScope` returns `undefined` whenever `chatScope` is set
(`src/NewUiApp.tsx:2586`), on the reasoning that `credentialScopeNote` "already
opens with the same host sentence in its own words". It does not: it opens with
the hosts of the *additive* member only (`src/lib/groups.ts:370`). On the Claude
pane, `chatScope` is built from `claude-web` and names `claude.ai`. The section's
other host member, `anthropic` / `api.anthropic.com`, is `Scope::Host` and
brokered - flipping it intercepts api.anthropic.com for every proxy-honouring
client on the machine - and its scope sentence is now suppressed, so the pane
never says it. The same applies to the ChatGPT pane's second host entry.

`rowScope` additionally takes only the *first* member with
`scope === "host" || scope === "machine"` (`src/NewUiApp.tsx:2585`), so even when
it does render, a section with two host members on different hosts names one of
them.

What to do: render both notes when both apply, and aggregate the hosts across
every `host`-scoped member in the section rather than picking the first.

### M4. The tray offers the same session-routing switch and says nothing about scope

`src/TrayApp.tsx:801`, `src/components/gc/dialogs.tsx:1709`

The tray draws the same sections from the same `buildGroups` and dispatches the
same `routeSection` with `{ sessions: true }`. It imports neither `scopeNote`
nor `credentialScopeNote` (verified across `src/TrayApp.tsx` and
`src/components/gc/Tray.tsx`), and it has no pane on which to draw them. The only
thing a tray user is told before their claude.ai or chatgpt.com session is routed
is `SessionConsentDialog`'s subtitle, which names the credential ("which Gate
sees on the account you are already signed in with") and never the scope. The
fact the taxonomy was introduced to surface - `Scope::Host` means every client on
the machine that talks these hosts, not just the app the switch is named for - is
absent from the tray entirely.

What to do: add the host sentence to `SessionConsentDialog`'s body for any
`host`-scoped surface it lists. That fixes both shells at once, since the window
raises the same component.

### M5. Consent is a renderer-side convention with nothing enforcing it below the IPC boundary

`src-tauri/src/lib.rs:2406`, `crates/core/src/preferences.rs:416`,
`src/NewUiApp.tsx:1547`, `src/TrayApp.tsx:810`

`accept_session_routing` is write-only. Nothing in Rust reads
`session_routing_accepted` - the only readers are `src/NewUiApp.tsx:1546` and
`src/TrayApp.tsx:809`. `proxy_set_domain` performs no credential check, so any
caller that reaches that command routes an additive row without the dialog. The
Rust-side guarantee the comments repeatedly invoke
(`src/lib/groups.ts:1008`, `src/components/gc/dialogs.tsx:1692`: "nothing the CLI
or a restore does can flip them") is true only of the *family cascade*; the
app-switch break of the invariant is policed by two `if` statements in the
renderer and nothing else.

The recording and defaulting themselves are correct, and this is worth stating
because it is the half that does fail safely:

- `Preferences::session_routing_accepted` is `#[serde(default)]` over a `Vec`,
  so a file written by an older build reads as empty
  (`crates/core/src/preferences.rs:103`), and `Default` sets it empty
  (`crates/core/src/preferences.rs:219`). Absent is "never asked", not "accepted".
- `preferences::load` falls back to `Preferences::default()` on a missing or
  unparseable file (`crates/core/src/preferences.rs:321`), so a corrupt
  preferences file means the question is asked again. Fails closed.
- Both shells read `prefs?.session_routing_accepted ?? []`, so a failed or
  not-yet-completed preferences load also means the question is asked. Fails
  closed.
- `accept_session_routing` is recorded before the routing runs
  (`src/NewUiApp.tsx:3037`), which is right: the answer is the person's and
  stands even if a write then fails.

What to do: if the consent is meant to be a security control rather than a
courtesy, move the check into `proxy_set_domain` in `src-tauri/src/lib.rs` -
refuse to enable a domain whose `credential` does not cascade unless that
domain's section id is in `session_routing_accepted`. If it is meant to stay a
UI courtesy, soften the three comments that describe it as a Rust-enforced
guarantee, because the next person to add a call site will believe them.

## Low

### L6. Attribution is steerable by any intercepted local client

`crates/core/src/proxy/mod.rs:1418`, `:1470`, `:1504`, `:1234`

`client_tool` now reads three caller-controlled headers.
`anthropic-client-platform: desktop_app` (or a non-empty `anthropic-client-app`)
stamps `claude-desktop`; a non-empty `originator` on a chatgpt.com entry stamps
`chatgpt`; `oai-device-id` and friends stamp `chatgpt-web`. Any local process
whose traffic is being intercepted can send these, so it can file its requests
under another app's name in the gateway's `client_tool` column and therefore in
the per-app counters the user reads on the App pane. The doc comment's framing -
"reading them is not a guess at all, it is the app saying what it is" - holds for
a cooperative client and not for a hostile one.

Two things limit this and are worth crediting. `inject_attribution` removes
`GATE_CLIENT_HEADER` before stamping (`crates/core/src/proxy/mod.rs:1234`), so a
caller cannot set the value directly, only steer which branch fires. And
`chatgpt_app` is gated on the slug the *routing decision* matched
(`crates/core/src/proxy/mod.rs:1471`, fed from `MatchedRoute.slug` at
`crates/core/src/proxy/engine.rs:887`), not on a header, so the generically-named
`originator` cannot cross vendors. That gating is the right design and it is the
reason this is Low rather than Medium: nothing in routing, credential injection
or the cascade rule reads `client_tool`.

What to do: nothing structural. Consider one sentence in `client_tool`'s doc
saying the value is caller-steerable and must not be used for anything but
display, so a later reader does not promote it to an authorization input.

### L7. The CLI routes a session surface with no warning at the moment of the act

`crates/cli/src/main.rs:687`

`gate-connect proxy domain claude-web on` enables the row and prints
`Enabled claude-web.` The new `proxy domains` table does carry
`additive` / `host` (`crates/cli/src/main.rs:755`), but a user who runs the
toggle directly never sees it. The GUI now asks a question before this exact act;
the CLI does the same thing silently.

What to do: when the resolved catalog entry's `credential` does not cascade,
print one line after the toggle naming the credential and the hosts - the same
two facts `credentialScopeNote` gives in the GUI.

### L8. Local write access to the domains file or the Linux helper socket bypasses the dialog

`crates/core/src/proxy/config.rs:75`, `crates/core/src/proxy/helper.rs:294`,
`crates/core/src/proxy/control.rs:240`

`load_domains` applies a `{slug: bool}` map from `proxy/domains.json`, so writing
`{"claude-web": true}` enables session interception with no dialog. Over the
Linux helper socket, `Request::SetIntercept` carries `ProxyDomain` values and
`validate_domains` checks only `slug`, `hosts` and `upstream_url` - not `enabled`
and not the new taxonomy fields - so a socket client can arm an additive row
directly. Both are pre-existing and are equivalent to local file/socket access,
which is already a full compromise of this app's trust boundary. Flagged only
because the branch's comments now describe the Rust layer as the thing that makes
session routing impossible without consent, and it is not.

## Traced and clean

- **Serde defaults on `ProxyDomain`'s new fields**
  (`crates/core/src/proxy/mod.rs:1752` onward). `default_credential` is
  `Observed`, whose `cascades()` is `false`, so an entry that arrives without the
  field cannot ride a family switch - the safe direction, and explicitly the
  reason `Brokered` was not chosen. `default_scope` is `Host`, the widest claim,
  so the UI over-warns rather than under-warns. `default_client` is `AnyApp`,
  which is cosmetic (the ledger groups by `SECTIONS`, not by `client`). They are
  also unreachable from disk: `config::load_domains`
  (`crates/core/src/proxy/config.rs:19`, `:75`) persists only a `HashMap<String, bool>`
  and rebuilds every other field from `default_domains()`. Nothing on the engine
  path reads `credential` or `scope`.
- **`provider::enable` / `disable`.** `enable_inner`
  (`crates/core/src/provider.rs:429`) and `disable_inner` (`:493`) both iterate
  `cascade_domains(&p)`, which filters on `credential.cascades()` against the
  built-in catalog rather than the persisted one (`:161`). An unknown slug is
  excluded rather than included. `off_members` (`:746`) and
  `proxy_domains_enabled` (`:253`) use the same filter, so the family's rendered
  state and its cascade agree.
- **`provider::restore_all`, `snapshot_and_disable_all`,
  `snapshot_and_disable_everything`, `reconcile_enabled`, `restore_one`.** All
  operate on provider slugs and re-enter `enable_inner` / `disable_inner`, so a
  restore cannot enable an additive row and a teardown cannot be tricked into one.
- **`routing::enable`** (master ON, and the CLI's `proxy enable` at
  `crates/cli/src/main.rs:647`) restores providers through the same path. No
  additive slug reachable.
- **The popover shell.** `src/App.tsx:998` calls `cascadeTargets(group, on)` with
  no `opts`, so the old shell keeps the brokered-only cascade it always had;
  `cascadeTargets`'s default is the safe one (`src/lib/groups.ts:1002`).
  `src/screens/GroupMembers.tsx:115` now keys its explanatory paragraph on
  `member.credential !== "brokered"` rather than the retired `chat` flag, which
  is a strictly wider set (it would also catch `Observed`). Per-member switches
  there remain the deliberate per-row act.
- **`cascadeTargets` itself** (`src/lib/groups.ts:996`). `sessions` is an opt-in
  parameter defaulting to absent, it is passed by exactly two call sites
  (`src/NewUiApp.tsx:1275`, `src/TrayApp.tsx:758`), and both are reached only
  through a consent check. The section-lookup-before-member-lookup ordering in
  both `toggleRailApp` (`src/NewUiApp.tsx:1540`) and `toggleApp`
  (`src/TrayApp.tsx:802`) is load-bearing - the section id `chatgpt` collides
  with the catalog slug `chatgpt` - and is correct in both.
- **The window's pane switch** (`src/NewUiApp.tsx:3362`) dispatches through
  `toggleRailApp`, so it inherits the consent gate rather than bypassing it. The
  notice-action paths (`src/NewUiApp.tsx:1418`, `:2379`) call `setAppRouted` on a
  config tool slug only, never a domain.
- **`classify_client` / `rules_for_client`**
  (`crates/core/src/proxy/mod.rs:2256`, `:2326`). Unchanged by this branch beyond
  extracting a header-name constant. Forging `anthropic-client-platform:
  web_claude_ai` yields `ClientClass::Web`, which *subtracts* rewrite prefixes
  against the entry's own list and cannot widen them; forging the other direction
  gets the unnarrowed rules, which is what an unrecognised client already gets.
  Either way it affects only the forger's own requests.
- **Secret handling.** The new CLI columns are the three taxonomy slugs
  (`crates/cli/src/main.rs:755`-`:800`). The diagnostics report adds
  `client, scope, credential` per tool row and per domain row
  (`src/lib/diagnosticsReport.ts:367`, `:385`) - three enum words, no host
  credential, no key prefix, no cookie. The one new debug line in the helper
  daemon (`crates/core/src/proxy/helper.rs:315`) prints a count and enabled slugs
  and is gated on `engine::debug_log()`. `inject_gate_credential`'s new `domain`
  argument and `MatchedRoute.slug` carry a catalog slug, not a secret. No new
  `Serialize` reaches a credential.
- **`useRouting.confirmCaTrusted`** (`src/lib/useRouting.ts:223`). The new
  `caTrustedThisSession` ref is cleared when the CA is untrusted
  (`src/lib/useRouting.ts:514`), so the session memo cannot outlive the
  certificate and let a later cascade skip the trust gate.
