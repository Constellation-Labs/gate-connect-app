/**
 * Analytics seam. The rest of the app calls `track` / `trackError` and the
 * `note*` functions here and never touches posthog-js directly, so the whole
 * thing no-ops cleanly when no build-time key is configured.
 *
 * Privacy posture for a credentials product: manual events only - no
 * autocapture, no session recording, no auto-pageviews. Event props pass through
 * an allowlist so a sensitive value (gateway host, API key, path) can't ride
 * along by accident, and Tauri-side errors are classified before send rather
 * than shipping the raw string.
 *
 * **Identity (AG-960).** Events are filed under this install's id from the first
 * one: the Rust install id (`install-id` in the data dir) is the PostHog
 * distinct id, bootstrapped at init, so an event sent before sign-in belongs to
 * the machine and survives a webview storage reset. Once the app is paired,
 * events also carry the `organization` group (the org id, nothing else about
 * the org), and a Constellation sign-in identifies the install with the
 * account's Cognito `sub` - the opaque id the dashboard and the gateway already
 * file that person's events under - so the install funnel can join the
 * dashboard's download click to the gateway's first request. No name, email,
 * key or path is ever sent; `docs/analytics-events.md` is the inventory.
 *
 * Consent (AG-603) gates all of it: an opted-out install sends nothing, except
 * one `diagnostics_opted_out` record at the moment it opts out.
 */
import posthog from "posthog-js";
import { getVersion } from "@tauri-apps/api/app";
import { POSTHOG_KEY_VALUE, POSTHOG_HOST } from "./config";
import { fetchPlatform } from "./platform";
import {
  classifyError,
  connectionFailureReason,
  type ConnectionFailureReason,
  type ErrorContext,
} from "./errors";
import {
  analyticsMilestoneClaim,
  coworkSettingCheck,
  getPreferences,
  installId as fetchInstallId,
  type AuthMode,
} from "./api";
import { errorContext } from "./errorContext";

/** The only event names we ever emit. `docs/analytics-events.md` lists every one
 *  with its props, and `analytics.inventory.test.ts` keeps the two in step. */
export type AnalyticsEvent =
  | "app_launched"
  | "popover_opened"
  | "signed_in"
  | "workspace_forgotten"
  | "key_replaced"
  | "proxy_enabled"
  | "proxy_disabled"
  | "domain_toggled"
  | "tool_toggled"
  | "group_toggled"
  | "ca_trusted"
  | "env_export_enabled"
  | "env_export_disabled"
  | "ca_untrusted"
  | "tour_completed"
  | "tour_skipped"
  | "update_shown"
  | "update_installed"
  | "update_dismissed"
  | "agents_closed"
  | "stale_agents_shown"
  | "routing_notice_shown"
  | "oauth_offer_shown"
  | "oauth_offer_accepted"
  | "launch_at_login_toggled"
  | "error_shown"
  // The install funnel (AG-960). Sent through the functions below rather than
  // `track`, because each has a rule `track` does not know: once per install,
  // sent instantly, or sent while the client is being switched off.
  | "app_first_launched"
  | "pairing_completed"
  | "tool_connected"
  | "first_request_proxied"
  | "connection_failed"
  | "diagnostics_opted_out";

/** The funnel milestones: each sent at most once per install (per tool, for
 *  `tool_connected`), claimed through a Rust-side marker so three windows and a
 *  wiped webview cannot send it twice. */
export type MilestoneEvent =
  | "app_first_launched"
  | "pairing_completed"
  | "tool_connected"
  | "first_request_proxied";

export type Props = Record<string, string | number | boolean>;

/**
 * Prop keys allowed on the wire. Anything not listed is dropped before send -
 * the backstop against a host/key/path slipping into an event payload.
 */
export const ALLOWED_PROP_KEYS: ReadonlySet<string> = new Set<string>([
  "has_account",
  "proxy_available",
  "routing_on",
  "codex_drifted",
  "provider",
  "provider_count",
  "domain",
  "tool",
  "routed",
  "enabled",
  "launch_at_login",
  "context",
  "title",
  "source",
  "count",
  "step",
  // `agents_closed` and `routing_notice_shown` have always passed these, and the
  // allowlist has always dropped them. Both are a count or a flag about Gate's
  // own dialogs, nothing of the user's.
  "restarted",
  "inline",
  // The install funnel (AG-960). `auth_mode` is "oauth" or "api_key",
  // `org_count` how many organizations the sign-in offered, `surface` whether a
  // tool was connected by writing its config or by routing a proxy domain,
  // `reason` a label from `ConnectionFailureReason`, `detail` which Claude
  // setting a `cowork_setting_missing` read.
  "auth_mode",
  "org_count",
  "surface",
  "reason",
  "detail",
  // The error context (`lib/errorContext.ts`), which rides `trackError` and
  // `captureException` and no other event. Listed here rather than waved past
  // `sanitize` because the allowlist is the backstop, and a context object is
  // exactly the kind of thing that grows a field nobody reviewed. `install_id`
  // is also a super-property now; it is the distinct id before sign-in anyway.
  "install_id",
  "os_version",
  "tools_detected",
  "verdict_states",
  "feed_state",
]);

/** Whether this window may send right now: the client exists and consent is on. */
let enabled = false;
/** Whether `posthog.init` has run. Distinct from `enabled`: a user who opts out
 *  and back in must not re-initialise a live client. */
let started = false;
/**
 * What the user said, as far as this window knows. `null` until the preference
 * read answers, and for good if it fails: consent that cannot be confirmed is
 * not consent, and it is also not a refusal, which matters to the milestones
 * (see `trackMilestone`).
 */
let consent: boolean | null = null;
/** The in-flight or finished start, so concurrent callers share one init. */
let starting: Promise<void> | null = null;
/** The in-flight or finished `initAnalytics`. */
let booting: Promise<void> | null = null;
/** Whether `initAnalytics` is still deciding. Events tracked meanwhile wait in
 *  `pending` rather than being dropped, and are dropped only if the answer is no. */
let deciding = false;
/**
 * Captures made before the client could send them, replayed in order once it
 * can - after the super-properties and the distinct id are in place, which is
 * the point: the first event of a launch used to race `register` and could go
 * out without a version or a platform.
 *
 * Only filled while consent is being established (the boot read, or a start
 * after the user turned the switch on). Cleared the moment the answer is no, so
 * nothing captured before an opt-out can leave after it. Capped, because a
 * start that never finishes must not grow it forever.
 */
let pending: Array<() => void> = [];
const PENDING_CAP = 50;

function sanitize(props?: Props): Props | undefined {
  if (!props) return undefined;
  const out: Props = {};
  for (const [k, v] of Object.entries(props)) {
    if (ALLOWED_PROP_KEYS.has(k)) out[k] = v;
  }
  return out;
}

/**
 * Telemetry must never break a user flow. Every public entry point below
 * routes through this: a PostHog failure (blocked host, CSP, a broken
 * init) becomes a console note, not an exception thrown into the caller.
 * This is load-bearing, not defensive habit - `track` sits directly on the
 * onboarding window's close path, where a throw leaves a window the user
 * cannot close (Tauri prevents the native close whenever JS listens for
 * close-requested, and only destroys the window if that handler resolves).
 */
function safely(what: string, fn: () => void): void {
  try {
    fn();
  } catch (e) {
    console.warn(`[gate] analytics ${what} failed`, e);
  }
}

/** Send now if we may, hold it if consent is still being decided, drop it
 *  otherwise. The one door every capture goes through. */
function send(fn: () => void): void {
  if (enabled) {
    fn();
    return;
  }
  if ((deciding || (starting !== null && !started)) && consent !== false) {
    if (pending.length < PENDING_CAP) pending.push(fn);
  }
}

/**
 * Start PostHog, if there is a key **and** the user has not opted out.
 *
 * Consent is read before the client is constructed, not after: opting out and
 * then initialising would put the user's device on the wire before the opt-out
 * took effect, however briefly. An install that has opted out never creates the
 * client at all.
 *
 * A failed read means **do not collect**. `preferences::load()` is infallible on
 * the Rust side, so the only way here is the IPC itself failing - and consent that
 * cannot be confirmed is not consent. The cost is a session of missing telemetry
 * on an app that is already misbehaving.
 *
 * No-op without a build-time key either way, so dev builds and unconfigured
 * releases send nothing. Idempotent: every window calls it once, and a second
 * call returns the first one's promise.
 */
export function initAnalytics(): Promise<void> {
  if (!booting) booting = boot();
  return booting;
}

async function boot(): Promise<void> {
  if (!POSTHOG_KEY_VALUE) return;
  deciding = true;
  try {
    const read = await getPreferences()
      .then((p) => p.share_diagnostics)
      .catch(() => null);
    // A switch flipped while the read was in flight is the newer answer.
    if (consent === null) consent = read;
    if (consent !== true) {
      pending = [];
      return;
    }
    await startPosthog();
  } finally {
    deciding = false;
  }
  // Every window runs this; the claim decides which one reports it.
  void trackMilestone("app_first_launched");
}

/**
 * Construct the client with its identity already settled.
 *
 * Three reads first, all local: the install id, the app version and the
 * platform. The install id is the reason to wait. `bootstrap.distinctID` makes
 * it the distinct id before the first capture - posthog-js's `_init` registers
 * it as `distinct_id` (and `$device_id`) and marks the user anonymous, which is
 * `posthog-core.js` ~575-582 in 1.407.2 - so nothing is ever filed under a
 * random browser id, and a wiped `localStorage` comes back as the same install
 * rather than a new person. Version, platform and install id are then
 * registered as super-properties before `enabled` flips, so the first event of
 * a launch carries them; they used to be registered in an unawaited `then` that
 * the first event could beat.
 *
 * Note what bootstrap does on every launch: it re-registers the install id as
 * an ANONYMOUS distinct id even if the last launch identified. That is why
 * `applySession` identifies again once the session is known, and why that costs
 * one `$identify` per launch of a signed-in install.
 */
function startPosthog(): Promise<void> {
  if (!POSTHOG_KEY_VALUE) return Promise.resolve();
  if (!starting) {
    starting = (async () => {
      const [id, app_version, platform] = await Promise.all([
        fetchInstallId().catch(() => null),
        getVersion().catch(() => "unknown"),
        fetchPlatform().catch(() => "unknown"),
      ]);
      safely("init", () =>
        posthog.init(POSTHOG_KEY_VALUE, {
          api_host: POSTHOG_HOST,
          autocapture: false,
          capture_pageview: false,
          capture_pageleave: false,
          disable_session_recording: true,
          person_profiles: "identified_only",
          ...(id ? { bootstrap: { distinctID: id } } : {}),
        }),
      );
      started = true;
      safely("register", () =>
        posthog.register({ app_version, platform, ...(id ? { install_id: id } : {}) }),
      );
      if (consent !== true) {
        // Switched off while we were starting. Nothing was captured yet; make the
        // off stick in PostHog's own persistence and send nothing.
        safely("opt_out_capturing", () => posthog.opt_out_capturing());
        pending = [];
        return;
      }
      enabled = true;
      applySession();
      const queued = pending;
      pending = [];
      for (const fn of queued) fn();
    })();
  }
  return starting;
}

/** Where a consent change came from, for `diagnostics_opted_out`'s `source`. */
export type ConsentSource = "settings" | "onboarding" | "onboarding_skip";

/** How long an opt-out waits on the marker store before it stops the client
 *  anyway. Local file I/O; this only bounds a wedged IPC. */
const OPT_OUT_CLAIM_TIMEOUT_MS = 2000;

/**
 * Apply a consent change made in Settings or onboarding, so the switch controls
 * something rather than only recording an intention.
 *
 * Turning it **off** stops every entry point here at once (`enabled` drops
 * synchronously), then records the opt-out, then opts the client out -
 * `opt_out_capturing` also persists PostHog's own flag, so nothing queued leaks
 * out after the user said no. The record is `diagnostics_opted_out`, sent at
 * most once per install (a Rust-side marker) and sent instantly, before the
 * client stops, so it actually leaves the machine: an event queued for the next
 * batch would be dropped by the opt-out it describes. Only a live client
 * records it; an install that never started (opted out at launch, or an
 * unreadable preference) has nothing to switch off and nothing to report from.
 *
 * **Once per install, not once per opt-out.** Someone who opts out, back in and
 * out again is recorded the first time only, which is what the ticket asks
 * ("recorded once"). The later history is still readable: PostHog's own
 * `$opt_in` event marks every opt back in.
 *
 * Turning it **on** starts the client if this session never did (the opted-out
 * install case) and opts back in otherwise.
 *
 * Resolves when the change has fully landed; callers may ignore it. Safe to
 * call with the value it already has.
 */
export function setAnalyticsConsent(
  consented: boolean,
  source: ConsentSource = "settings",
): Promise<void> {
  consent = consented;
  if (!consented) {
    const wasLive = enabled;
    enabled = false;
    pending = [];
    if (!started) return Promise.resolve();
    const stop = () => {
      // A switch turned back on while the record was in flight wins.
      if (consent === false) safely("opt_out_capturing", () => posthog.opt_out_capturing());
    };
    if (!wasLive) {
      stop();
      return Promise.resolve();
    }
    return recordOptOut(source).then(stop, stop);
  }
  if (!started) return startPosthog();
  if (enabled) return Promise.resolve();
  safely("opt_in_capturing", () => posthog.opt_in_capturing());
  enabled = true;
  applySession();
  return Promise.resolve();
}

async function recordOptOut(source: ConsentSource): Promise<void> {
  const claimed = await Promise.race([
    analyticsMilestoneClaim("diagnostics_opted_out").catch(() => false),
    new Promise<boolean>((resolve) => setTimeout(() => resolve(false), OPT_OUT_CLAIM_TIMEOUT_MS)),
  ]);
  if (!claimed) return;
  // Straight to posthog rather than through `send`: `enabled` is already false,
  // and this is the one event that is allowed past it.
  safely("capture", () =>
    posthog.capture("diagnostics_opted_out", sanitize({ source }), { send_instantly: true }),
  );
}

/**
 * The anonymous device id, or why there is none to show. Three states because
 * "analytics never started" and "it started and we could not read the id" are
 * different findings when a support thread is asking why no events arrived.
 */
export type AnalyticsId =
  | { kind: "id"; value: string }
  | { kind: "disabled" }
  | { kind: "unavailable" };

/**
 * The id PostHog files this install's events under, for the diagnostics report:
 * the install id, or the account's Cognito `sub` once a Constellation sign-in
 * has identified the install - the one string that lets a pasted report be lined
 * up against its event stream.
 */
export function analyticsId(): AnalyticsId {
  if (!enabled) return { kind: "disabled" };
  try {
    const value = posthog.get_distinct_id();
    return value ? { kind: "id", value } : { kind: "unavailable" };
  } catch (e) {
    console.warn("[gate] analytics get_distinct_id failed", e);
    return { kind: "unavailable" };
  }
}

export function track(event: AnalyticsEvent, props?: Props): void {
  const clean = sanitize(props);
  send(() => safely("capture", () => posthog.capture(event, clean)));
}

// ---------------------------------------------------------------------------
// Milestones (AG-960)

/** Markers this window has already asked about, so a signal that repeats (the
 *  relay's traffic report, every 30s per tool) costs one IPC, not one per
 *  report. The answer itself lives in Rust. */
const askedHere = new Set<string>();

/**
 * Send a funnel milestone once per install.
 *
 * Waits for the start to settle, then claims the marker and sends only if the
 * claim is won. Three outcomes, and the difference between the last two is a
 * decision:
 *
 * - **Consent on**: claim, and send on a win.
 * - **Consent off**: claim, and send nothing. The milestone happened while the
 *   user was opted out, so it is spent: opting back in later must not report,
 *   weeks late and with a wrong timestamp, something that happened during the
 *   opt-out. That is what makes the funnel's gap read as "opted out" rather
 *   than as a late conversion.
 * - **Consent unknown** (the preference read failed, or there is no build key):
 *   leave it unclaimed. Nobody said no, so a later launch that can confirm
 *   consent may still report it.
 *
 * A claim that fails (the store could not answer) sends nothing: a first launch
 * reported twice is worse than one not reported.
 */
export async function trackMilestone(
  event: MilestoneEvent,
  props?: Props,
  marker: string = event,
): Promise<boolean> {
  if (!POSTHOG_KEY_VALUE || askedHere.has(marker)) return false;
  if (booting) await booting;
  if (starting) await starting;
  if (consent === null) return false;
  askedHere.add(marker);
  let won: boolean;
  try {
    won = await analyticsMilestoneClaim(marker);
  } catch {
    // Unanswered, so not spent: let a later signal ask again.
    askedHere.delete(marker);
    return false;
  }
  if (!won || !enabled) return false;
  const clean = sanitize(props);
  safely("capture", () => posthog.capture(event, clean));
  return true;
}

/** What the shell knows about the signed-in session. */
export interface SessionFacts {
  /** A usable credential and, for OAuth, an organization. */
  signedIn: boolean;
  authMode: AuthMode | null;
  /** The Cognito `sub`, for an OAuth session. */
  sub: string | null;
  /** The organization this install routes for: the one chosen at sign-in, or
   *  for an API key the one the gateway resolved it to. */
  orgId: string | null;
}

let session: SessionFacts | null = null;
let reportsPairing = true;
let identifiedAs: string | null = null;
let groupedAs: string | null = null;
let orgChoices: number | null = null;

/**
 * Tell the seam who is signed in. Called by the shells whenever account, OAuth
 * or activity state changes; cheap and idempotent, so they need not diff.
 *
 * Every window that can send events calls this, so whatever it sends carries
 * the org group - a `tool_connected` won by the tray must not go out ungrouped.
 * Only the window that owns sign-in reports `pairing_completed`: the tray
 * re-reads the account on the same `session-changed` edge and could otherwise
 * win the claim without the org count the sign-in knows. `reportPairing: false`
 * is how it says so.
 */
export function noteSession(
  facts: SessionFacts,
  { reportPairing = true }: { reportPairing?: boolean } = {},
): void {
  session = facts;
  reportsPairing = reportPairing;
  if (enabled) applySession();
}

/** How many organizations the sign-in offered, for `pairing_completed`'s
 *  `org_count`. Called by the org picker's load. */
export function noteOrgChoices(count: number): void {
  orgChoices = count;
}

/**
 * Tie this install's events to the account and the org.
 *
 * **`identify(sub)`, not `alias(sub)`, and why it is safe.** In posthog-js
 * 1.407.2 `identify` sends `$identify` with `$anon_distinct_id` = the current
 * (bootstrapped, anonymous) install id whenever the id changes and the current
 * user is known-anonymous (`posthog-core.js` ~2205-2215). PostHog merges the
 * anonymous install person INTO the `sub` person, which may already exist and
 * be identified - it is the dashboard's person, who clicked the download - and
 * that is exactly the join the funnel needs. `alias` goes the other way: it
 * asks to fold the alias id into the current person, and PostHog refuses that
 * for an id that is already identified (the method's own comment calls it
 * "VERY BAD" for an existing person, ~2839-2842). A second account signing in
 * on the same machine is safe too: the install id already belongs to the first
 * person, which is identified, so the server does not merge them; the second
 * account's events are simply filed under its own `sub`.
 *
 * Only OAuth has a `sub`. An API-key account has no user identity on this
 * machine, so it is grouped but never identified; its funnel joins through the
 * `organization` group instead.
 *
 * The group is the org id and nothing else. The org's name is the dashboard's
 * to set on the group; sending it from here would put a customer name on every
 * install's events for no analytical gain.
 */
function applySession(): void {
  const s = session;
  if (!s || !s.signedIn) return;
  if (s.authMode === "oauth" && s.sub && identifiedAs !== s.sub) {
    const sub = s.sub;
    safely("identify", () => posthog.identify(sub));
    identifiedAs = sub;
  }
  if (s.orgId && groupedAs !== s.orgId) {
    const org = s.orgId;
    safely("group", () => posthog.group("organization", org));
    groupedAs = org;
  }
  if (s.orgId && reportsPairing) {
    void trackMilestone("pairing_completed", {
      auth_mode: s.authMode ?? "unknown",
      ...(s.authMode === "oauth" && orgChoices !== null ? { org_count: orgChoices } : {}),
    });
  }
}

/**
 * A tool was connected: its config written, or its proxy domain routed. Called
 * on every success; the milestone is once per tool per install.
 *
 * Connecting the Claude row (the `anthropic` domain) also reads whether Claude
 * Desktop keeps local Cowork off, since that is the one way this connect
 * succeeds and still routes nothing: see `reportCoworkSetting`.
 */
export function noteToolConnected(tool: string, surface: "config" | "domain"): void {
  void trackMilestone("tool_connected", { tool, surface }, `tool_connected.${tool}`);
  if (surface === "domain" && tool === "anthropic") void reportCoworkSetting();
}

/**
 * The relay saw a gateway-bound request leave for the gateway, from these tools.
 * The first one is `first_request_proxied`: Gate forwarded a real request for a
 * real tool, measured where it happens. See `docs/analytics-events.md` for why
 * this and not a gateway read.
 */
export function noteTrafficObserved(tools: readonly (string | null)[]): void {
  const tool = tools.find((t): t is string => typeof t === "string" && t.length > 0);
  void trackMilestone("first_request_proxied", { source: "relay", ...(tool ? { tool } : {}) });
}

/**
 * The gateway named this machine as one it has had traffic from. The fallback
 * signal for `first_request_proxied` where the relay cannot report - Linux, whose
 * engine runs in a helper daemon with no observer - and the gateway's own word
 * that a request from this install arrived.
 */
export function noteGatewayAttributed(): void {
  void trackMilestone("first_request_proxied", { source: "gateway" });
}

// ---------------------------------------------------------------------------
// Connection failures (AG-960)

/**
 * The error contexts that are a step of connecting: a tool, a domain, routing
 * itself, the certificate, or the backend's own restore of all of those. A
 * failure in one of these also sends `connection_failed`, beside the
 * `error_shown` every failure sends.
 */
const CONNECTION_CONTEXTS: ReadonlySet<ErrorContext> = new Set<ErrorContext>([
  "connect",
  "provider_toggle",
  "proxy_toggle",
  "trust_ca",
  "restore_routing",
  "provider_restore",
]);

/** How long one window stays quiet about the same failure. A dead session fails
 *  every read and a stuck port fails every retry; one report per cause per
 *  window per five minutes is the finding, the rest is noise. */
const FAILURE_REPEAT_MS = 5 * 60 * 1000;
const lastFailure = new Map<string, number>();

/**
 * Send `connection_failed`. Sent instantly rather than batched: the ticket asks
 * for a failure to be visible within a minute, and the batch timer lives in a
 * webview the OS may throttle while it is hidden, which is exactly when a
 * startup restore fails.
 */
export function reportConnectionFailure(
  reason: ConnectionFailureReason,
  context: string,
  props?: { tool?: string; detail?: string },
): void {
  const key = `${reason}|${context}|${props?.tool ?? ""}`;
  const now = Date.now();
  const last = lastFailure.get(key);
  if (last !== undefined && now - last < FAILURE_REPEAT_MS) return;
  lastFailure.set(key, now);
  const clean = sanitize({ ...props, reason, context });
  send(() =>
    safely("capture", () =>
      posthog.capture("connection_failed", clean, { send_instantly: true }),
    ),
  );
}

/**
 * Report the Claude Desktop setting that keeps local Cowork off, if one does.
 *
 * Once per install per setting (a marker claimed only while sending is
 * allowed), because the setting is a standing condition, not an event: someone
 * who reconnects the Claude row ten times with it off has one finding, not ten.
 * The read is Claude's own config file, so this is a deterministic check, not a
 * guess from traffic; `core::analytics::cowork_setting_missing` says exactly what
 * it reads and what it cannot see.
 */
async function reportCoworkSetting(): Promise<void> {
  if (!POSTHOG_KEY_VALUE) return;
  if (booting) await booting;
  if (starting) await starting;
  if (!enabled) return;
  const detail = await coworkSettingCheck().catch(() => null);
  if (!detail) return;
  const won = await analyticsMilestoneClaim(`cowork_setting_missing.${detail}`).catch(() => false);
  if (!won) return;
  reportConnectionFailure("cowork_setting_missing", "connect", { tool: "anthropic", detail });
}

/**
 * The gateway refused or could not be reached on a read the shell made - the
 * connection step's view of "auth rejected" and "offline", taken from the typed
 * `FailureCode` rather than from an error message.
 */
export function noteGatewayFailure(code: string): void {
  if (code === "rejected" || code === "signed_out") {
    reportConnectionFailure("auth_rejected", "gateway");
  } else if (code === "offline") {
    reportConnectionFailure("offline", "gateway");
  }
}

/**
 * Record a user-facing failure: a paired `error_shown` event plus a PostHog
 * exception. We send the *classified* title + context, never the raw Tauri
 * error string (it can carry hosts/paths). `props` lets a call site attach
 * extra allowlisted dimensions (e.g. which provider's toggle failed).
 *
 * A failure in a connecting context also sends `connection_failed` with its
 * reason. `reason` is the backend's typed answer where it had one; otherwise
 * the reason is read from the error the same way the title is.
 */
export function trackError(
  err: unknown,
  context: ErrorContext,
  props?: Props,
  reason?: ConnectionFailureReason | null,
): void {
  const { title } = classifyError(err, context);
  // The state Gate was in, merged UNDER the call site's own props so a caller
  // naming the tool it was toggling still wins. Errors only: see
  // `lib/errorContext.ts` for why this is not a super-property.
  track("error_shown", { ...errorContext(), ...props, context, title });
  // The paired exception carries the same state. It is a separate record from
  // the `error_shown` event above, and whoever triages the exception list does
  // not have that event beside them.
  const ctx = { ...sanitize(errorContext()), context };
  send(() =>
    safely("captureException", () => posthog.captureException(new Error(title), ctx)),
  );
  if (CONNECTION_CONTEXTS.has(context)) {
    const named = props?.tool ?? props?.domain;
    reportConnectionFailure(
      reason ?? connectionFailureReason(err, context),
      context,
      typeof named === "string" ? { tool: named } : undefined,
    );
  }
}

/**
 * Forward an uncaught JS exception (genuine frontend crash) with its real
 * stack. These are our own bugs, not Tauri error strings, so the stack is the
 * point and there's nothing of the user's to redact.
 */
export function captureException(err: unknown): void {
  // Sanitised like any other payload: an uncaught exception is our own bug and
  // its stack is the point, but the context riding beside it goes through the
  // same allowlist everything else does.
  const ctx = sanitize(errorContext());
  send(() => safely("captureException", () => posthog.captureException(err, ctx)));
}
