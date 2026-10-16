# Analytics event inventory

Every event Gate Connect sends to PostHog, what it carries, and the rules that
decide whether it is sent at all. `src/lib/analytics.ts` is the only code that
talks to PostHog, and `src/lib/analytics.inventory.test.ts` fails if an event or
an allowed property is added there without a row here (or the reverse).

Nothing on this page is sent when the build has no PostHog key (every dev build),
or when the user has turned off **Share diagnostic data**, with the one exception
of `diagnostics_opted_out`.

## Identity

- **Before sign-in, and for every install that is never identified**, events
  are filed under this install's id: the `install-id` file in the data dir,
  generated locally and the same id routed requests carry as
  `x-gate-install-id`. It is passed to posthog-js as `bootstrap.distinctID`, so
  the first event of every launch already carries it and a wiped webview
  storage does not create a new person. Base events use it from the first
  launch; the id already rode every error event as `install_id` before AG-960.
- **Super-properties** on every event: `app_version`, `platform`
  (`macos`, `windows`, `linux`, `unknown`) and `install_id`, registered before
  the first event of a launch is released.
- **Constellation sign-in (OAuth)**, once the diagnostics question is answered
  yes: the install is identified with the Cognito `sub` from the id token, the
  id the dashboard identifies its PostHog person with and the one the gateway
  sends `first_gateway_request` under.
  - The **first** identification of an install calls `identify(sub)`, which
    merges the anonymous install person, and what it sent before sign-in, into
    the `sub` person. `identify` rather than `alias`, because the `sub` person
    usually already exists and is identified (it clicked the download), and
    PostHog will not fold an identified id into another person. An install
    that ran with an API key before its first Constellation sign-in merges
    the same way: nothing ever identified it, so its install person is
    nobody's until then.
  - The identity is stored in the data dir (`analytics-identity.json`), so
    every later launch bootstraps the `sub` directly as an identified distinct
    id and sends no `$identify`, and every window (the tray, the intro) follows
    the same identity.
  - **Any later account change** (a second account signing in on the machine,
    or the same one after a sign-out) resets the client first, so only a fresh
    empty id is merged into the new person. The app merges the install id into
    a person at most once per install, and never files a later account B under
    account A's person. Events from the install before B's sign-in stay with
    A's person, where they were sent.
  - **Sign-out** (in the app or `gate-connect logout`) or Reset clears the
    stored identity in the core (`oauth::clear`, which every sign-out path
    reaches), and the app's backend announces it to every window. Each window then resets to a
    fresh anonymous PostHog id, NOT back to the install id: once identified,
    the install id belongs to that account's person, so filing under it would
    keep sending as the account that left. `install_id` stays a
    super-property. The next launch of such an install bootstraps nothing and
    keeps that fresh id, and if its stored client state was never reset (the
    sign-out happened while no client ran), it is reset at the next start
    instead of resuming the old person. An app session whose reads RESOLVED to
    "not signed in" (no account file, or a credential that is gone or was
    refused) is treated the same way. A read that could not answer decides
    nothing: an account read that rejected (the account file or the secret
    store could not be read), an OAuth status read that failed, or a session
    the identity provider or the secret store could not give (offline) all keep
    the identity exactly as it is. Pasting an API key over a Constellation
    sign-in also leaves that identity.
  - **The core only stores a sub for the live session.** A window's view of the
    session lags the core's, so `save_identity_in` refuses a save naming a sub
    that is not the stored token bundle's (`oauth::current`, witnessed on
    `account.json`), and the app broadcasts the stored record, which moves the
    window back onto it. A save in flight when a sign-out ran therefore cannot
    put the account back. A session that could not be READ (the secret store
    did not answer) is a different refusal: nothing is broadcast, the window
    keeps its identity, and it retries the save (three times, 15 seconds apart,
    and again at its next session note). Either refusal still stores a
    requested `ever_identified`, because the client has already merged the
    install id by then. Every writer of `analytics-identity.json` runs its
    read-modify-write under a process-wide mutex and an advisory lock on
    `analytics-identity.lock` (`flock` on macOS and Linux, an unshared open on
    Windows), so the app's windows and `gate-connect logout` cannot lose each
    other's writes. The wait is bounded only against another process: the file
    lock gives up after ten seconds, so a stuck CLI (or a stuck app, for the
    CLI) cannot hang a sign-out, while the app's own writers queue on the mutex
    with no timeout. If a CLI logout's forget gives up while the app's save
    holds the lock, the record keeps the old sub until the main window next
    notes the session as signed out, or at the latest until the next launch,
    whose first identity read finds no account and finishes the forget. The
    record is announced to the windows from inside the lock, so announcements
    follow the order of the
    writes.
- **Pairing** sets the `organization` group to the org id, once the question
  is answered yes: the org chosen at sign-in, or for an API-key account the
  org the gateway resolved the key to (read from `/v1/me/activity`). Every
  milestone waits for it, up to two minutes (see Milestones). Only the id;
  the org's name is the dashboard's to set. In PostHog, setting a group turns on
  person processing, so an API-key install gets a person profile keyed on its
  install id once grouped.
- **API-key installs are never tied to a person**, by the app or by anything
  else: an API key carries no user, and the key's creator is not necessarily
  the person at this machine. Their events stay on the install id, an
  anonymous person, and are counted by `organization` (see the funnel below).
  Reset, a replaced key, or a key that now resolves to another org changes
  nothing about the install id, because nothing about it belongs to anybody.
  **Known limit:** an install that ran with an API key and later signs in with
  Constellation merges its earlier history into that account at the first
  identification, like any pre-sign-in history. On a machine two people share,
  that history may be the other person's.
- An `analytics-identity.json` whose content cannot be parsed fails closed: it
  reads as identified, so the install files under a fresh anonymous id from
  then on. A read that fails for a passing reason (permission, a busy file)
  makes the app treat the install that way for that read, but never writes it
  back: the next read of the unchanged record decides again. A logout the CLI
  makes while the app is open is on disk at once and takes effect in the app at
  its next account read, or at the next launch; there is no message from the
  CLI to the running app.
  **Known limit:** if the app support directory cannot be resolved, the
  identity reads as the default (fail open) until the next identity broadcast
  corrects it.
- Never sent: names, emails, API keys, tokens, gateway hosts, file paths, error
  text. Error events carry a classified title (an uncaught rejection that is
  not an `Error` too); failure events carry a reason from a closed list.
- **Nothing is captured automatically**, whatever the PostHog project's own
  settings say. In posthog-js 1.407.2 several features fall back to the
  project's remote config when the client leaves them undefined (exception
  autocapture, dead clicks, heatmaps, web vitals), so the client pins every one
  off by name (`CLIENT_CONFIG` in `analytics.ts`), turns off the remote config
  and flags requests (`advanced_disable_flags`), and loads no external script
  (`disable_external_dependency_loading`, which also keeps the toolbar and the
  recorder out). Surveys, product tours, conversations, web experiments, site
  apps, session recording, autocapture, page views and console log capture are
  off too.

## Consent

Two tiers, because the events that predate AG-960 already had a rule and this
change does not widen what they send.

- **Base events** (every row below not marked AG-960) keep their existing rule:
  sent while **Share diagnostic data** is on, which it is by default on a fresh
  install before the onboarding step has asked. The preference is read before
  the client is constructed; off, or unreadable, means no client and nothing
  sent. Events tracked while the read is in flight wait and are dropped if the
  answer is no.
- **Everything AG-960 added** (identify, group, every milestone,
  `connection_failed`) is held in memory until the diagnostics question has
  been **answered** (`share_diagnostics_recorded`), and lost if the app quits
  first. Nothing is claimed while
  it is held. A yes releases it in order, identity and group first, each event
  stamped with the time it happened; a no spends the held milestones unsent and
  drops the rest. The onboarding step comes after sign-in, so on a fresh install
  the first launch, the pairing and any early connection failure all wait for
  it.
- Turning sharing off (Settings, onboarding's Continue with it off, or Skip)
  stops every event at once and opts the client out, and sends one
  `diagnostics_opted_out` (below). The change is broadcast to every window.
- Turning it back on lifts posthog-js's own persisted opt-out, which outlives a
  relaunch, and sends one PostHog `$opt_in`, from the window where it was
  turned on (the other windows lift theirs silently; posthog-js would otherwise
  send one per window). A launch that lifts a stale opt-out sends none.
- A milestone that happens while opted out is spent, not deferred: opting back
  in never reports it late. A milestone that happens while consent is unknown
  (the preference could not be read) is left for a later launch.

**`diagnostics_opted_out`** is the answer itself, so it is the one AG-960 event
not held. It is sent at most once per install (a second opt-out after opting
back in is not recorded), straight to PostHog's capture endpoint so it does not
race the client being switched off. It is filed on the person: under the
account's `sub` when a Constellation sign-in is known (without an `$identify`,
so an install that was never identified is not merged by it), otherwise under
the install id (or, for an install identified once and signed out since, under
its fresh anonymous id), with the org group when known, and carries only
`source`. It is NOT sent with `$process_person_profile: false`: PostHog
attributes past personless events to a person after a merge, but personless
events cannot be used to build cohorts or in group analytics
(https://posthog.com/docs/data/anonymous-vs-identified-events), and the funnel's
"opted out" breakdown is exactly a cohort of people who performed it.
At most once rather than at least once: the marker is claimed before the send,
so a send that fails is lost rather than retried, because a duplicate would
count one install's opt-out twice while a lost one only leaves it looking like a
drop-off.

## Events with no sender

The popover shell (`App.tsx`) was removed in #383, and with it the only call
sites of eight events: `popover_opened`, `proxy_enabled`, `proxy_disabled`,
`ca_trusted`, `update_shown`, `update_dismissed`, `stale_agents_shown` and
`oauth_offer_shown`. Their names stay in `AnalyticsEvent`, and so in the table
below, because the inventory test keeps the two in step; nothing in the app
sends them now, so PostHog only holds them from builds before that removal. The
same goes for the `provider_count`, `codex_drifted` and `launch_at_login`
props, which only that shell's `app_launched` carried. Every AG-960 event is
sent by the main window (`NewUiApp`), the onboarding window or the tray window.

## Milestones

Each is sent at most once per install, instantly (not batched). The claim is a
marker file per milestone under `<data dir>/analytics-milestones/`, created
with `create_new`, so exactly one of the three windows (or any process) wins it,
and it is claimed only while the client is actually delivering. The store is
built in a staging directory with its `.store` sentinel (and `.legacy` marker,
when legacy) inside and renamed into place, so no start ever sees it half made,
and a store in place is never an empty directory that a racing creator's
rename could replace.

**Every milestone carries the `organization` group.** A milestone that is
released (or happens) before the install's org is known waits for it, up to
two minutes (`ORG_WAIT_MS`), and the client's group is set to that org right
before the capture, so a first launch held until the diagnostics answer goes
out grouped like the pairing after it, with its own original timestamp. The
org is usually known by then, because the question comes after pairing; what
waits is an API-key install whose org only the activity read learns, or a tray
or intro window that learns it from the stored identity. **If no org arrives
within the bound, the milestone is sent without one**, and any group left from
an earlier account is cleared first: it still counts in the person funnel and
in trends, and is missing from the organization funnel, which is the truth
for an install that never paired. Nothing is claimed while it waits, so a quit
during the wait leaves it for the next launch; if sharing is switched off
during the wait, the milestone is spent unsent, like any other.

An install that ran Gate Connect before this store existed (an `account.json`
or `preferences.json` already on disk when the store is created) never sends
the first-occurrence milestones, because it cannot know whether the first time
already happened. `diagnostics_opted_out` and the Cowork condition are not first
occurrences and are still sent there. **A CLI-first install is judged legacy
too**: `gate-connect` writes `account.json` before the app ever runs, so an
install that signed in through the CLI and then opened the app does not send
`app_first_launched` (or the other first-occurrence milestones). That
under-reports the funnel's first step for CLI-first users rather than reporting
a first launch for machines that are not new.

## Event inventory

| Event | Properties | When it fires |
| --- | --- | --- |
| `app_launched` | `has_account`, `proxy_available`, `routing_on` (builds before #383 also sent `provider_count`, `codex_drifted`, `launch_at_login` from the popover shell) | Every launch, once the first state read lands. |
| `app_first_launched` | none beyond the super-properties | AG-960 milestone. The first launch of a fresh install. |
| `pairing_completed` | `auth_mode` (`oauth`, `api_key`), `org_count` (OAuth, when the picker loaded) | AG-960 milestone. The first time the install is signed in with an org: after the org is chosen (OAuth), or when the gateway first resolves the API key's org. Carries the org group and, for OAuth, the account identity. |
| `tool_connected` | `tool` (registry slug, or a proxy domain slug such as `anthropic` for Claude Desktop and Cowork), `surface` (`config`, `domain`) | AG-960 milestone, once per tool. The first successful connect of that tool from any switch. |
| `first_request_proxied` | `source` (`relay`, `gateway`), `tool` (relay only, when the relay named the sender) | AG-960 milestone. `relay`: Gate's relay or engine forwarded a gateway-bound request for a routed tool (the `traffic-observed` report, about 5 seconds after the burst). `gateway`: the gateway listed this install among the ones it has had traffic from, the fallback on Linux where the engine runs in a helper daemon with no observer. |
| `connection_failed` | `reason`, `context`, `tool` (when known), `detail` (Cowork only) | AG-960. A failure on a connecting, sign-in or pairing step, sent instantly (not batched) once the diagnostics question is answered. At most once per window per reason, context and tool every 5 minutes. While the question is open at most 20 are held, so failures cannot crowd the milestones out of the 100-item hold. |
| `diagnostics_opted_out` | `source` (`settings`, `onboarding`, `onboarding_skip`) | AG-960. Once per install, the first time sharing is switched off; see Consent. |
| `popover_opened` | none | Not sent since #383 (see Events with no sender). The popover shell was reopened from the tray. |
| `signed_in` | none | A sign-in or API-key save completes (before any org is chosen). |
| `workspace_forgotten` | none | Reset completes. |
| `key_replaced` | none | The API key is replaced. |
| `proxy_enabled` | `source` (`toggle`, `restored`) | Not sent since #383. Popover shell: routing turned on, or restored at launch. |
| `proxy_disabled` | `source` | Not sent since #383. Popover shell: routing turned off. |
| `domain_toggled` | `domain`, `routed` or `enabled` | A proxy domain row is switched. |
| `tool_toggled` | `tool`, `routed` | A config tool is switched. |
| `group_toggled` | `provider`, `enabled` | A family switch is used. |
| `ca_trusted` | none | Not sent since #383. Popover shell: the certificate is trusted. |
| `ca_untrusted` | none | The certificate is removed from the trust store. |
| `env_export_enabled` | none | The command-line environment channel is turned on. |
| `env_export_disabled` | none | It is turned off. |
| `tour_completed` | `source` | The intro tour is finished. |
| `tour_skipped` | `source`, `step` | The intro tour is skipped. |
| `update_shown` | `source` (`banner`, `panel`) | Not sent since #383. An update is offered. |
| `update_installed` | none | An update is installed. |
| `update_dismissed` | `source` | Not sent since #383. An update offer is dismissed. |
| `agents_closed` | `count`, `restarted` | Running tools were closed or restarted from Gate. |
| `stale_agents_shown` | none | Not sent since #383. The stale-agents hint is shown. |
| `routing_notice_shown` | `enabled`, `inline` | The routing change notice is shown. |
| `oauth_offer_shown` | none | Not sent since #383. The offer to move a key account to sign-in is shown. |
| `oauth_offer_accepted` | none | That offer is accepted. |
| `launch_at_login_toggled` | `enabled` | Launch at login is switched. |
| `error_shown` | `context`, `title`, plus the error context (`install_id`, `os_version`, `tools_detected`, `verdict_states`, `feed_state`, `routing_on`) and the call site's `tool`, `domain`, `provider`, `routed`, `enabled` | Any user-facing failure. Paired with a PostHog exception carrying the same title. |

PostHog's own `$identify`, `$groupidentify` and `$opt_in` events also appear,
from the calls described above.

## Properties

Only these keys ever leave the app; any other key is dropped before sending.

| Property | Meaning |
| --- | --- |
| `has_account` | An account file exists. Left out when the account could not be read. |
| `proxy_available` | This platform has the proxy subsystem. |
| `routing_on` | The engine is running. |
| `codex_drifted` | Codex's config was hand-edited away from Gate (popover launch, before #383). |
| `provider` | Family or provider group id. |
| `provider_count` | Enabled providers (popover launch, before #383). |
| `domain` | Proxy domain slug. |
| `tool` | Tool or domain slug. |
| `routed` | The switch's new state. |
| `enabled` | The switch's new state (older call sites). |
| `launch_at_login` | Launch at login is on (popover launch, before #383). |
| `context` | Which action failed, from a closed list (`ErrorContext`), or `gateway` for a failed gateway read. |
| `title` | The classified error title. |
| `source` | Where an action came from (per event, above). |
| `count` | How many tools were closed. |
| `step` | Tour step. |
| `restarted` | How many tools were restarted. |
| `inline` | The notice was drawn inline. |
| `auth_mode` | `oauth` or `api_key`. |
| `org_count` | Organizations offered at sign-in. |
| `surface` | `config` or `domain`. |
| `reason` | A `connection_failed` reason, below. |
| `detail` | Which Claude setting a `cowork_setting_missing` read. |
| `install_id` | This install's id (also the pre-sign-in distinct id). |
| `os_version` | OS name and version, on errors only. |
| `tools_detected` | Installed tool slugs, on errors only. |
| `verdict_states` | Routing verdict per tool, on errors only. |
| `feed_state` | Security feed connection state, on errors only. |

## Connection failure reasons

`reason` on `connection_failed` is one of:

| Reason | Meaning and source |
| --- | --- |
| `port_in_use` | A loopback port Gate binds is held by another process (a second app, or `gate-connect proxy relay`). Decided from the error's type (`io::ErrorKind::AddrInUse` in the chain) for backend failures; for command rejections, from the relay's own sentence or the OS's "address in use" words. |
| `cowork_setting_missing` | Connecting the Claude row succeeded, but Claude Desktop keeps local Cowork off, so a Cowork task never reaches this machine's network. `detail` is `user` (`preferences.secureVmFeaturesEnabled: false` in a `claude_desktop_config.json`), `org_cloud_only` (`preferences.coworkLocalTasksOffLatched: true` there, Claude's cached copy of an org policy that runs Cowork in the cloud only) or `enterprise` (the `secureVmFeaturesEnabled` policy set to 0 under `HKLM\SOFTWARE\Policies\Claude`, Windows only). Read once on each connect of the `anthropic` domain, from every config Claude itself may use (see below), any-true, and reported once per install per `detail`. |
| `routing_off` | A proxy-routed tool refused because the engine is not running. |
| `ca_trust_declined` | The OS certificate prompt was declined or dismissed. |
| `prompt_declined` | Another OS prompt (the admin prompt for the system proxy) was cancelled. |
| `offline` | The gateway could not be reached (typed `FailureCode::Offline` on gateway reads). |
| `auth_rejected` | The gateway refused the session or key: typed `FailureCode::Rejected` on gateway reads or in a command's JSON failure envelope, or a 401 worded as one (`returned 401`, `status 401`, `401 Unauthorized`, `unauthorized`; a bare `401` inside a port or an id does not count). `SignedOut` is not a refusal (there was no credential to send) and is not reported. |
| `sign_in_not_completed` | The browser sign-in was declined (`access_denied`) or abandoned (the five-minute wait for the redirect ran out). |
| `unknown` | None of the above. |

Contexts that send it: `connect`, `provider_toggle`, `proxy_toggle`,
`trust_ca`, `restore_routing`, `provider_restore`; `gateway` (the Overview's
activity read); and the sign-in and pairing steps, `sign_in` (browser sign-in or
API key save), `org_list` (reading the organizations, including the automatic
pick of the only one) and `org_select` (choosing one).

**Which Claude config is read.** Every `claude_desktop_config.json` Claude
Desktop 2.16120.0's own resolver may use (its `$Re()` for first-party data dirs
and `Gu()` for the third-party deployment): on Windows
`%LOCALAPPDATA%\Claude-Data`, `%APPDATA%\Claude`,
`%LOCALAPPDATA%\Packages\Claude_pzs8sxrjxfjjc\LocalCache\Roaming\Claude` (the
MSIX package, which local Cowork requires) and `%LOCALAPPDATA%\Claude-3p`; on
macOS `~/Library/Application Support/Claude` and `.../Claude-3p`. Any file that
says off is enough.

**Limits of `cowork_setting_missing`.** The keys are Claude Desktop's internal
names, found in its 2.16120.0 bundle (the preference schema, the defaults
`secureVmFeaturesEnabled: true` and `coworkLocalTasksOffLatched: false`, and the
support check that returns `disabled_by_user`, `disabled_by_org_policy` and
`disabled_by_enterprise`), not a documented contract, and may change. It does not
see: the macOS managed-profile copy of the policy (a binary plist under
`/Library/Managed Preferences`), platform limits (macOS below 14, no
virtualization), or the per-account "Only on your computer" choice on claude.ai,
which is stored server-side. Anthropic's help center says that choice is removed
on 2026-10-06, after which new Cowork tasks run in the cloud:
https://support.claude.com/en/articles/15520349-use-claude-cowork-on-web-desktop-and-mobile
A missing file, key or unparseable JSON reads as "not blocked".

## Delivery

posthog-js batches events and flushes every 3 seconds by default
(`DEFAULT_FLUSH_INTERVAL_MS` in its `request-queue`). The milestones and
`connection_failed` bypass the batch (`send_instantly`), and
`diagnostics_opted_out` is posted directly, so each is on the wire within
seconds even from a hidden window whose timers are throttled.

**The one-minute latency holds only once the diagnostics question has been
answered.** Before that, AG-960's events are held in memory (see Consent),
so a sign-in or pairing failure on a fresh install, which happens before the
onboarding step asks, reaches PostHog when the question is answered yes, with
its original timestamp, and never if it is answered no or never answered.

## Building the install funnel in PostHog

No SQL. Product analytics, New insight, Funnels, **Aggregating by:
organization**:

1. `setup_download_clicked` (sent by the dashboard)
2. `app_first_launched`
3. `pairing_completed`
4. `first_gateway_request` (sent by the gateway)

- This is the funnel the ticket asks for, and it covers every account type,
  API keys included: every app milestone carries the `organization` group
  (see Milestones), and the dashboard and the gateway send theirs with it
  (gate repo: dashboard-web's `org-context.tsx` and `setup-analytics.ts`,
  dashboard-api's `activation.service.ts`). Use
  a conversion window of at least 14 days. An install that never paired sends
  its first launch ungrouped, so it is not in this funnel, which is the truth
  about it.
- **The person funnel is secondary, and OAuth only.** The same four steps
  aggregated by unique users join end to end for Constellation sign-ins: the
  dashboard and the gateway file under the Cognito `sub`, and the app's install
  person is merged into it at the install's first sign-in (once the question is
  answered yes). API-key installs are never a person, so they do not appear in
  it past the dashboard step.
- **Opted out, not dropped off.** `diagnostics_opted_out` carries the
  `organization` group too, so the same insight with that event as an extra
  step, or a trend of it aggregated by organization, shows which orgs have
  installs that stopped reporting. For the person funnel, create a behavioural
  cohort "Opted out of diagnostics": persons who performed
  `diagnostics_opted_out` at any time (filed under the account's `sub` for a
  Constellation sign-in, so it lands on the same person as the dashboard and
  gateway steps), and break the funnel down by it. Opted-out installs stop
  sending app milestones but still reach step 4, which the gateway sends
  regardless, so their gap shows in their own row instead of as drop-off.
- Break down by `platform` or `app_version`, or by `reason` on a
  `connection_failed` trend, to see why installs stall between steps 3 and 4.
