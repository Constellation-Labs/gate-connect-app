# Onboarding audit v0.2.0: source verification

Source-verified response to the user report filed 9 Sep 2026 ("Gate Connect
onboarding audit", 6 findings). Every finding was checked against the `v0.2.0`
tag (what the reporter ran) and against `main` in both repositories as of
10 Sep 2026.

Two repositories are involved. The menu-bar app is this repo; the console and
the docs site live in the `gate` monorepo under `apps/dashboard-web` and
`apps/docs`.

- `gate-connect-app` @ `c86eb529`
- `gate` @ `c9c7f6e93`

**All six findings reproduce unchanged on `main`.** Nothing had been fixed
between v0.2.0 and today, so every line reference below is live on both.

## Verdicts

### F1 Nothing tells the user which environment they are in - CONFIRMED, and understated

The host renders as a bare truncating mono line in `text-gc-ink-3` with no
environment marker (`src/screens/Home.tsx:361-379`). That much the report has
right. What it does not say is that the app also **hardcodes every console
link to production regardless of the gateway it is pointed at**:

- `src/lib/config.ts:48-49` - `GATE_DASHBOARD_URL` and `GATE_API_KEYS_URL` are
  constants pinned to `app.constellationgate.ai`.
- Consumed by `Home.tsx:787` (open dashboard), `OrgPicker.tsx:165`,
  `FirstRun.tsx:200`.

So a staging-pointed app does not merely fail to warn. It walks the user to
the production console by hand. The mismatch the report describes is
constructed by the product, not stumbled into by the user.

**How the reporter reached staging.** The default is production
(`src/lib/config.ts:11`) and release builds do not override it: there is no
`VITE_GATE_DEFAULT_BASE_URL` in `.github/workflows/release.yml` at v0.2.0 or on
main. It came from Settings, where **Dev mode** is a plain visible button in
the Help section (`Settings.tsx:744`) and Staging is one click further
(`Settings.tsx:763`). The confirm panel warns that switching forgets the stored
key, disconnects tools and relaunches. It says nothing about the dashboard
going quiet. Two unguarded clicks, a permanent consequence, and no persistent
marker afterwards.

### F2 Both troubleshooting paths send the user the wrong way - CONFIRMED

Docs step 5 is verbatim as quoted
(`apps/docs/src/content/docs/getting-started/quickstart-gate-connect.md`).

The console half: `SetupListening.tsx` polls the org request count on a 4s
cycle and, after `FIRST_MESSAGE_PROMPT_DWELL_MS` of nothing
(`SetupListening.tsx:34`, 45s), offers Gatekeeper. The gateway is named nowhere
on that screen, and the handoff carries no environment question with it.

### F3 Console step 3 asks for an action the app no longer offers - CONFIRMED; the caveat resolves against the cautious reading

The report asks someone with the source to settle whether the key field exists
only in the signed-out state. It does, and it is weaker than that:

- The field lives only in first-run, and there it sits behind a **secondary
  disclosure** - "Use an API key instead" (`FirstRun.tsx:161`, placeholder
  `sk-gw-…` at `:176`) - underneath the primary "Sign in with Constellation"
  button (`FirstRun.tsx:132`).
- Once connected by OAuth the entire key block in Settings is gated
  `{!isOAuth && …}` (`Settings.tsx:433`).

For a user on the primary, encouraged path there is no key field in **any**
state, signed in or out. Console step 3 is therefore not "unreachable for
anyone already connected"; it documents the legacy alternative as if it were
the only path. Docs step 2 has the identical defect and the report treats it as
merely mid-migration.

Confirmed independently by `plans/oauth-default-api-key-evaluation.md` in the
`gate` repo: OAuth traffic is attributed to a synthetic `cognito:<user>:<org>`
identity with **no `gateway_api_keys` row at all**. There is no key for the
Gate Connect OAuth path to create, paste, or store.

### F4 Proxy and Routing are the same thing under two names - CONFIRMED

Console: `SetupGateConnect.tsx:137` (step 3 description),
`SetupConnect.tsx:28` (choose-your-path card), and the mock's own label at
`SetupGateConnect.tsx:240`.

App: zero user-facing "Proxy" strings at v0.2.0 or on main. The only survivors
are internal prop names (`onToggleProxy`, `onEnableRouting` in `App.tsx`),
which no user reads. The console is the last surface using the old term, as
the report says.

### F5 The checklist never registers work the user already did - CONFIRMED, mechanism identified, one claim wrong

The counter is four steps (`components/get-started-menu.tsx:37-45`):

| Step | Source of truth |
| --- | --- |
| Choose your path | `localStorage` flag `path` |
| Connect a tool | `localStorage` flag `connected` |
| Send your first message | `api.requestStats().totalRequests > 0` |
| Simulate an attack | `localStorage` flag `attack` |

Three of four are browser-local flags (`lib/onboarding-progress.ts`), so they
are per-browser-profile, never per-account. `connected` is set by *clicking*
"Listen for my message" in the console, not by detecting anything. **Nothing in
the checklist observes the app.** 0 of 4 for a fully configured, actively
routing user is the expected output for a different browser, a cleared
profile, or a setup done outside the console flow. The one server-derived step
then read production, which was empty for the real reason.

**Correction.** The report's compounding-cost claim is wrong. Steps 4 and 5 are
not locked behind step 3. Both gate on
`keyReady = keyCreated || selectedKeyId !== null`, which comes from step 2's
create-or-select. Step 3 is a `SetupStepCard` with no action inside it, so it
can neither complete nor block. Selecting an existing active key in step 2
unlocked "Listen for my message". The confirmation screen was reachable. This
does not weaken F5, but it should not go into the ticket.

### F6 The embedded screenshot shows a build that no longer ships - CONFIRMED

It is not a screenshot but `GateConnectWidget`, an animated mock in the same
file (`SetupGateConnect.tsx:178-255`), cycling Idle/Proxy Off to
Connected/Proxy On. It rots exactly as fast as a screenshot and carries the
stale vocabulary in F4 as well.

## Findings the report did not have

### F7 Every console link in the app is pinned to production

Covered under F1 above. Tracked separately because it has its own fix and its
own blast radius: it is the only item here that actively misdirects rather than
passively failing to inform.

### F8 The console's gateway URL is declared three times, and two fallbacks point at staging

- `lib/api-client.ts:6` - `import.meta.env.VITE_GATEWAY_URL || ""`
- `components/connect-tabs.tsx:21` - falls back to **staging**
- `pages/Models.tsx:176` - falls back to **staging**

A production console build that lost `VITE_GATEWAY_URL` would hand users
staging URLs to copy while its own API calls went nowhere. Same root cause as
F1: there is no single place in the console that knows which environment it is.

### F9 The quickstart is checked in twice

`docs/getting-started/quickstart-gate-connect.md` and
`apps/docs/src/content/docs/getting-started/quickstart-gate-connect.md` are
near-identical, differing only in frontmatter and link style. A second drift
vector inside the docs repo, before the console bundle is even involved.

## Root cause

The report's structural finding is right and F8 sharpens it. Three surfaces
describe one product, and not one of them holds the environment as a fact it
can state. The app knows its gateway and does not label it; the console knows
its gateway at build time and never renders it; the docs cannot know it and so
assert a default. The rename from Proxy to Routing then travelled through the
app and half the docs and stopped, because nothing links the copy to the
version it describes.
