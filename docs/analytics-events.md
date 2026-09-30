# Analytics event inventory

Every event Gate Connect sends to PostHog, what it carries, and the rules that
decide whether it is sent at all. `src/lib/analytics.ts` is the only code that
talks to PostHog, and `src/lib/analytics.inventory.test.ts` fails if an event or
an allowed property is added there without a row here (or the reverse).

Nothing on this page is sent when the build has no PostHog key (every dev build),
or when the user has turned off **Share diagnostic data**, with the one exception
of `diagnostics_opted_out`.

## Identity

- **Before sign-in** every event is filed under this install's id: the
  `install-id` file in the data dir, generated locally and the same id routed
  requests carry as `x-gate-install-id`. It is passed to posthog-js as
  `bootstrap.distinctID`, so the first event of every launch already carries it
  and a wiped webview storage does not create a new person.
- **Super-properties** on every event: `app_version`, `platform`
  (`macos`, `windows`, `linux`, `unknown`) and `install_id`. They are registered
  before the first event of a launch is released.
- **Constellation sign-in (OAuth)** calls `identify(sub)`, where `sub` is the
  Cognito subject from the id token: the same id the dashboard identifies its
  PostHog person with and the one the gateway sends `first_gateway_request`
  under. PostHog merges the anonymous install person into that person, so events
  from before sign-in join the dashboard's. `identify` rather than `alias`
  because the `sub` person is usually already identified (it clicked the
  download); PostHog refuses to identify an already-identified install into a
  second account, so two accounts on one machine are not merged.
  Because bootstrap re-registers the install id on each launch, a signed-in
  install sends one `$identify` per launch.
- **Pairing** sets the `organization` group to the org id: the org chosen at
  sign-in, or for an API-key account the org the gateway resolved the key to
  (read from `/v1/me/activity`). Only the id; the org's name is the dashboard's
  to set. API-key accounts are never identified, since they carry no user.
- Never sent: names, emails, API keys, tokens, gateway hosts, file paths, error
  text. Error events carry a classified title; failure events carry a reason
  from a closed list.

## Consent

- The preference is read before the client is constructed. Off, or unreadable,
  means no client and nothing sent. Events tracked while the read is in flight
  wait and are dropped if the answer is no.
- Turning it off (Settings, or onboarding's Continue with it off, or Skip) stops
  every event at once, sends `diagnostics_opted_out` instantly, then opts the
  client out. `diagnostics_opted_out` is sent **once per install**: a second
  opt-out after opting back in is not recorded again. Opting back in sends
  PostHog's own `$opt_in`.
- A milestone that happens while opted out is spent, not deferred: opting back
  in never reports it late. A milestone that happens while consent is unknown
  (the preference could not be read) is left for a later launch.

## Milestones

Each is sent at most once per install. The claim is a marker file per milestone
under `<data dir>/analytics-milestones/`, created with `create_new`, so exactly
one of the three windows (or any process) wins it. An install that ran Gate
Connect before this store existed (an `account.json` or `preferences.json`
already on disk when the store is created) never sends the first-occurrence
milestones, because it cannot know whether the first time already happened.
`diagnostics_opted_out` and the Cowork condition are not first occurrences and
are still sent there.

## Event inventory

| Event | Properties | When it fires |
| --- | --- | --- |
| `app_launched` | `has_account`, `proxy_available`, `routing_on`; the popover shell also sends `provider_count`, `codex_drifted`, `launch_at_login` | Every launch, once the first state read lands. |
| `app_first_launched` | none beyond the super-properties | Milestone. The first launch of a fresh install. |
| `pairing_completed` | `auth_mode` (`oauth`, `api_key`), `org_count` (OAuth, when the picker loaded) | Milestone. The first time the install is signed in with an org: after the org is chosen (OAuth), or when the gateway first resolves the API key's org. Carries the org group and, for OAuth, the account identity. |
| `tool_connected` | `tool` (registry slug, or a proxy domain slug such as `anthropic` for Claude Desktop and Cowork), `surface` (`config`, `domain`) | Milestone, once per tool. The first successful connect of that tool from any switch. |
| `first_request_proxied` | `source` (`relay`, `gateway`), `tool` (relay only, when the relay named the sender) | Milestone. `relay`: Gate's relay or engine forwarded a gateway-bound request for a routed tool (the `traffic-observed` report, about 5 seconds after the burst). `gateway`: the gateway listed this install among the ones it has had traffic from, the fallback on Linux where the engine runs in a helper daemon with no observer. |
| `connection_failed` | `reason`, `context`, `tool` (when known), `detail` (Cowork only) | A failure on a connecting step, sent instantly (not batched). At most once per window per reason, context and tool every 5 minutes. |
| `diagnostics_opted_out` | `source` (`settings`, `onboarding`, `onboarding_skip`) | Once per install, the first time a live client is switched off. |
| `popover_opened` | none | The popover shell is reopened from the tray. |
| `signed_in` | none | A sign-in or API-key save completes (before any org is chosen). |
| `workspace_forgotten` | none | Reset completes. |
| `key_replaced` | none | The API key is replaced. |
| `proxy_enabled` | `source` (`toggle`, `restored`) | Popover shell: routing turned on, or restored at launch. |
| `proxy_disabled` | `source` | Popover shell: routing turned off. |
| `domain_toggled` | `domain`, `routed` or `enabled` | A proxy domain row is switched. |
| `tool_toggled` | `tool`, `routed` | A config tool is switched. |
| `group_toggled` | `provider`, `enabled` | A family switch is used. |
| `ca_trusted` | none | Popover shell: the certificate is trusted. |
| `ca_untrusted` | none | The certificate is removed from the trust store. |
| `env_export_enabled` | none | The command-line environment channel is turned on. |
| `env_export_disabled` | none | It is turned off. |
| `tour_completed` | `source` | The intro tour is finished. |
| `tour_skipped` | `source`, `step` | The intro tour is skipped. |
| `update_shown` | `source` (`banner`, `panel`) | An update is offered. |
| `update_installed` | none | An update is installed. |
| `update_dismissed` | `source` | An update offer is dismissed. |
| `agents_closed` | `count`, `restarted` | Running tools were closed or restarted from Gate. |
| `stale_agents_shown` | none | The stale-agents hint is shown. |
| `routing_notice_shown` | `enabled`, `inline` | The routing change notice is shown. |
| `oauth_offer_shown` | none | The offer to move a key account to sign-in is shown. |
| `oauth_offer_accepted` | none | That offer is accepted. |
| `launch_at_login_toggled` | `enabled` | Launch at login is switched. |
| `error_shown` | `context`, `title`, plus the error context (`install_id`, `os_version`, `tools_detected`, `verdict_states`, `feed_state`, `routing_on`) and the call site's `tool`, `domain`, `provider`, `routed`, `enabled` | Any user-facing failure. Paired with a PostHog exception carrying the same title. |

PostHog's own `$identify`, `$groupidentify` and `$opt_in` events also appear,
from the calls described above.

## Properties

Only these keys ever leave the app; any other key is dropped before sending.

| Property | Meaning |
| --- | --- |
| `has_account` | An account file exists. |
| `proxy_available` | This platform has the proxy subsystem. |
| `routing_on` | The engine is running. |
| `codex_drifted` | Codex's config was hand-edited away from Gate. |
| `provider` | Family or provider group id. |
| `provider_count` | Enabled providers (popover launch). |
| `domain` | Proxy domain slug. |
| `tool` | Tool or domain slug. |
| `routed` | The switch's new state. |
| `enabled` | The switch's new state (older call sites). |
| `launch_at_login` | Launch at login is on. |
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
| `cowork_setting_missing` | Connecting the Claude row succeeded, but Claude Desktop keeps local Cowork off, so a Cowork task never reaches this machine's network. `detail` is `user` (`preferences.secureVmFeaturesEnabled: false` in `claude_desktop_config.json`), `org_cloud_only` (`preferences.coworkLocalTasksOffLatched: true` there, Claude's cached copy of an org policy that runs Cowork in the cloud only) or `enterprise` (the `secureVmFeaturesEnabled` policy set to 0 under `HKLM\SOFTWARE\Policies\Claude`, Windows only). Read once on each connect of the `anthropic` domain, and reported once per install per `detail`. |
| `routing_off` | A proxy-routed tool refused because the engine is not running. |
| `ca_trust_declined` | The OS certificate prompt was declined or dismissed. |
| `prompt_declined` | Another OS prompt (the admin prompt for the system proxy) was cancelled. |
| `offline` | The gateway could not be reached (typed `FailureCode::Offline` on gateway reads). |
| `auth_rejected` | The gateway refused the session or key (typed `FailureCode::Rejected` or `SignedOut` on gateway reads, or a 401 on a command). |
| `unknown` | None of the above. |

Contexts that send it: `connect`, `provider_toggle`, `proxy_toggle`,
`trust_ca`, `restore_routing`, `provider_restore`, and `gateway` (the Overview's
activity read).

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
(`DEFAULT_FLUSH_INTERVAL_MS` in its `request-queue`). `connection_failed` and
`diagnostics_opted_out` bypass the batch (`send_instantly`), so a failure is on
the wire within seconds even from a hidden window whose timers are throttled.

## Building the install funnel in PostHog

No SQL. Product analytics, New insight, Funnels:

1. `setup_download_clicked` (sent by the dashboard)
2. `app_first_launched`
3. `pairing_completed`
4. `first_gateway_request` (sent by the gateway)

- **Aggregated by unique users** this joins end to end for Constellation
  sign-ins: the dashboard and the gateway file under the Cognito `sub`, and the
  app's install person is merged into it at sign-in. Use a conversion window of
  at least 14 days.
- **Aggregated by `organization`** it covers every account type, including API
  keys, but step 2 has no org by construction (the app is not paired yet at its
  first launch), so use the three-step funnel `setup_download_clicked`,
  `pairing_completed`, `first_gateway_request` there, or add
  `first_request_proxied` as the app-side last step.
- **Opted out, not dropped off.** Create a behavioural cohort "Opted out of
  diagnostics": persons who performed `diagnostics_opted_out` at any time. Break
  the funnel down by that cohort. Opted-out installs stop sending app milestones
  but still reach step 4, which the gateway sends regardless, so their gap
  between steps shows in the cohort's own row instead of as drop-off.
- Break down by `platform` or `app_version`, or by `reason` on a
  `connection_failed` trend, to see why installs stall between steps 3 and 4.
