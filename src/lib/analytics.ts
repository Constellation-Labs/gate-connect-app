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
 * **Two tiers of consent.** The events that predate AG-960 keep their old rule:
 * sent while "Share diagnostic data" is on, which it is by default, before the
 * onboarding step has asked. Everything AG-960 added - the account and org
 * identity (`identify`, `group`), every funnel milestone and `connection_failed`
 * - is held in memory until the question has been ANSWERED
 * (`share_diagnostics_recorded`), and lost if the app quits first. A yes releases what was held, in order; a no
 * spends the milestones unsent and drops the rest. The one exception is
 * `diagnostics_opted_out`, which is the answer itself.
 *
 * **Identity (AG-960).** Base events are filed under this install's id (the Rust
 * `install-id`), bootstrapped as the PostHog distinct id so a storage reset does
 * not make a new person; that id already rode every error event as `install_id`.
 * Once allowed and paired, events carry the `organization` group (the org id
 * only), and a Constellation sign-in identifies the install with the account's
 * Cognito `sub`, the id the dashboard and the gateway already use. The current
 * identity is kept in Rust (`analytics-identity.json`) so every window and every
 * launch agree; see `applyIdentity`. No name, email, key or path is ever sent;
 * `docs/analytics-events.md` is the inventory.
 */
import posthog from "posthog-js";
import { getVersion } from "@tauri-apps/api/app";
import { listen } from "@tauri-apps/api/event";
import { POSTHOG_KEY_VALUE, POSTHOG_HOST } from "./config";
import { fetchPlatform } from "./platform";
import {
  classifyError,
  connectionFailureReason,
  type ConnectionFailureReason,
  type ErrorContext,
} from "./errors";
import {
  analyticsIdentity,
  analyticsMilestoneClaim,
  coworkSettingCheck,
  getPreferences,
  installId as fetchInstallId,
  setAnalyticsIdentity,
  type AnalyticsIdentity,
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

/** Whether this window may send base events: the client exists and sharing is on. */
let enabled = false;
/** Whether `posthog.init` has run. A user who opts out and back in must not
 *  re-initialise a live client. */
let started = false;
/**
 * "Share diagnostic data", as far as this window knows. `null` until the
 * preference read answers, and for good if it fails: consent that cannot be
 * confirmed is not consent, and it is also not a refusal.
 */
let consent: boolean | null = null;
/** Whether the diagnostics question has been answered. `null` until read. The
 *  gate on everything AG-960 added; see the header. */
let answered: boolean | null = null;
/** The in-flight or finished start, so concurrent callers share one init. */
let starting: Promise<void> | null = null;
/** The in-flight or finished `initAnalytics`. */
let booting: Promise<void> | null = null;
/** Whether `initAnalytics` is still deciding. */
let deciding = false;
/**
 * Base captures made before the client could send them, replayed in order once
 * it can, after the super-properties and the distinct id are in place. Cleared
 * the moment the answer is no. Capped.
 */
let pending: Array<() => void> = [];
const PENDING_CAP = 50;
/** This install's id, once read: the distinct id of an unidentified install. */
let installIdValue: string | null = null;
/** Super-properties, kept to re-register after a `reset`. */
let superProps: Props = {};

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

/** Whether the client will actually deliver. posthog-js keeps an opt-out in its
 *  own storage across launches, and a capture while it stands is silently
 *  dropped, so this is asked of the client rather than assumed from `enabled`. */
function capturing(): boolean {
  if (!started) return false;
  try {
    return posthog.is_capturing();
  } catch {
    return false;
  }
}

/** Send a base event now if we may, hold it if consent is still being decided,
 *  drop it otherwise. */
function send(fn: () => void): void {
  if (enabled) {
    fn();
    return;
  }
  if ((deciding || (starting !== null && !started)) && consent !== false) {
    if (pending.length < PENDING_CAP) pending.push(fn);
  }
}

// ---------------------------------------------------------------------------
// The AG-960 gate

/** One thing AG-960 would send: `run` sends it, `spend` is what a "no" does. */
interface Held {
  run: () => Promise<void>;
  spend?: () => Promise<void>;
}

/** Held until the diagnostics question is answered. Capped: a question never
 *  answered must not grow this for the life of the process. */
let held: Held[] = [];
/** Milestone markers already in `held`, so a signal that repeats while the
 *  question is open (the relay's traffic report, every 30s) is held once. */
const heldMarkers = new Set<string>();
const HELD_CAP = 100;

type FunnelState = "open" | "held" | "refused" | "closed";

/**
 * Where AG-960's events stand right now.
 *
 * - `open`: sharing on, question answered, client delivering.
 * - `held`: the answer is not in yet (the boot read, or sharing on by default
 *   and the question not answered).
 * - `refused`: sharing off. Milestones are spent, the rest dropped.
 * - `closed`: no key, no readable preference, or no client. Nothing is sent and
 *   nothing is spent, so a later launch that can confirm consent still may.
 */
function funnelState(): FunnelState {
  if (!POSTHOG_KEY_VALUE) return "closed";
  if (deciding || (starting !== null && !started)) return consent === false ? "refused" : "held";
  if (consent === false) return "refused";
  if (consent !== true) return "closed";
  if (answered !== true) return "held";
  return capturing() ? "open" : "closed";
}

function funnel(item: Held): void {
  switch (funnelState()) {
    case "open":
      void item.run();
      return;
    case "held":
      if (held.length < HELD_CAP) held.push(item);
      return;
    case "refused":
      void item.spend?.();
      return;
    case "closed":
      return;
  }
}

/** Settle what was held: release it if the funnel is open now, spend it if it
 *  was refused, keep holding otherwise. Session effects go first, so every
 *  released event carries the identity and the group. */
function settleHeld(): void {
  const state = funnelState();
  if (state === "held") return;
  const items = held;
  held = [];
  heldMarkers.clear();
  // Closed for good (no readable preference, no client): nothing may be sent
  // and nobody said no, so nothing is spent either.
  if (state === "closed") return;
  if (state === "open") {
    applySessionNow();
    for (const item of items) void item.run();
  } else {
    spendSession();
    for (const item of items) void item.spend?.();
  }
}

// ---------------------------------------------------------------------------
// Start

/**
 * Start PostHog, if there is a key **and** sharing is on.
 *
 * Consent is read before the client is constructed, not after: an install that
 * has opted out never creates the client at all. A failed read means **do not
 * collect**. No-op without a build-time key. Idempotent: every window calls it
 * once, and a second call returns the first one's promise.
 *
 * Also subscribes to the two backend broadcasts that keep windows in step: a
 * consent change made in another window, and a change of analytics identity.
 */
export function initAnalytics(): Promise<void> {
  if (!booting) booting = boot();
  return booting;
}

async function boot(): Promise<void> {
  if (!POSTHOG_KEY_VALUE) return;
  subscribe();
  deciding = true;
  try {
    const prefs = await getPreferences().catch(() => null);
    // A switch flipped while the read was in flight is the newer answer.
    if (consent === null) consent = prefs ? prefs.share_diagnostics : null;
    if (answered === null) answered = prefs ? prefs.share_diagnostics_recorded === true : null;
    if (consent !== true) {
      pending = [];
      return;
    }
    await startPosthog();
  } finally {
    deciding = false;
    settleHeld();
  }
  // Every window runs this; the claim decides which one reports it.
  void trackMilestone("app_first_launched");
}

/** The backend's broadcasts, emitted by `src-tauri/src/lib.rs` under the same
 *  names (`ANALYTICS_IDENTITY_EVENT`, `ANALYTICS_CONSENT_EVENT`) and pinned on
 *  both sides by `analytics.contract.test.ts`. */
export const ANALYTICS_IDENTITY_EVENT = "analytics-identity-changed";
export const ANALYTICS_CONSENT_EVENT = "analytics-consent-changed";

let subscribed = false;
function subscribe(): void {
  if (subscribed) return;
  subscribed = true;
  try {
    void listen<{ share_diagnostics: boolean; recorded: boolean }>(
      ANALYTICS_CONSENT_EVENT,
      (e) => void applyConsent(e.payload.share_diagnostics, e.payload.recorded, null),
    ).catch(() => {});
    void listen<AnalyticsIdentity>(ANALYTICS_IDENTITY_EVENT, (e) =>
      followIdentity(e.payload),
    ).catch(() => {});
  } catch {
    // Outside Tauri (unit tests, plain-browser dev) there is nothing to hear.
  }
}

/**
 * Construct the client with its identity already settled.
 *
 * Reads first, all local: the install id, the stored analytics identity, the
 * app version and the platform. Then `bootstrap`:
 *
 * - A stored identified `sub` (only ever written after the question was
 *   answered yes, see `applyIdentity`) bootstraps as `{ distinctID: sub,
 *   isIdentifiedID: true }`: posthog-js then registers it as the identified
 *   distinct id with no event (`posthog-core.js` ~575-582 in 1.407.2), so a
 *   signed-in launch sends no `$identify` at all.
 * - Otherwise the install id bootstraps as an anonymous distinct id.
 *
 * A persisted posthog-js opt-out outlives the launch that made it
 * (`__ph_opt_in_out_<token>`, `consent.js`), and `init` does not clear it, so a
 * user who opted out, relaunched and opted back in would have a client that
 * silently drops every capture. Consent is ours to decide, so a stale
 * persisted opt-out is lifted here.
 */
function startPosthog(): Promise<void> {
  if (!POSTHOG_KEY_VALUE) return Promise.resolve();
  if (!starting) {
    starting = (async () => {
      const [id, stored, app_version, platform] = await Promise.all([
        fetchInstallId().catch(() => null),
        analyticsIdentity().catch(() => null),
        getVersion().catch(() => "unknown"),
        fetchPlatform().catch(() => "unknown"),
      ]);
      installIdValue = id;
      if (stored) adoptStoredIdentity(stored);
      // An install that was identified once and is signed out now gets no
      // bootstrap: its install id already belongs to that person, so filing
      // under it would put this launch back on the account that left. The
      // client keeps the fresh anonymous id the sign-out `reset` gave it.
      const bootstrap = identifiedAs
        ? { distinctID: identifiedAs, isIdentifiedID: true }
        : id && !everIdentified
          ? { distinctID: id }
          : undefined;
      safely("init", () =>
        posthog.init(POSTHOG_KEY_VALUE, {
          api_host: POSTHOG_HOST,
          autocapture: false,
          capture_pageview: false,
          capture_pageleave: false,
          disable_session_recording: true,
          person_profiles: "identified_only",
          ...(bootstrap ? { bootstrap } : {}),
        }),
      );
      started = true;
      superProps = { app_version, platform, ...(id ? { install_id: id } : {}) };
      safely("register", () => posthog.register(superProps));
      if (consent !== true) {
        safely("opt_out_capturing", () => posthog.opt_out_capturing());
        pending = [];
        return;
      }
      liftStaleOptOut();
      enabled = true;
      const queued = pending;
      pending = [];
      for (const fn of queued) fn();
    })();
  }
  return starting;
}

function liftStaleOptOut(): void {
  safely("opt_in_capturing", () => {
    if (posthog.has_opted_out_capturing()) posthog.opt_in_capturing();
  });
}

// ---------------------------------------------------------------------------
// Consent

/** Where a consent change came from, for `diagnostics_opted_out`'s `source`. */
export type ConsentSource = "settings" | "onboarding" | "onboarding_skip";

/**
 * Apply a consent answer given in THIS window (Settings or onboarding).
 *
 * The answer is recorded as an answer (`answered` = true) whichever way it
 * went, which is what releases or spends what AG-960 was holding. The backend
 * broadcasts the same answer to the other windows once `set_share_diagnostics`
 * lands, and they apply it without recording anything.
 *
 * Turning it **off** stops every entry point at once, sends
 * `diagnostics_opted_out` (see `recordOptOut`), and opts the client out;
 * `opt_out_capturing` persists PostHog's own flag, so nothing queued leaks out
 * after the user said no. Turning it **on** starts the client if this session
 * never did and opts back in otherwise.
 */
export function setAnalyticsConsent(
  consented: boolean,
  source: ConsentSource = "settings",
): Promise<void> {
  return applyConsent(consented, true, source);
}

async function applyConsent(
  consented: boolean,
  recorded: boolean,
  source: ConsentSource | null,
): Promise<void> {
  const was = consent;
  consent = consented;
  if (recorded) answered = true;
  if (!consented) {
    enabled = false;
    pending = [];
    // Only the window the user clicked in records it, and only for a real
    // transition from sharing to not sharing.
    if (source !== null && was === true) void recordOptOut(source);
    if (started) safely("opt_out_capturing", () => posthog.opt_out_capturing());
    settleHeld();
    return;
  }
  if (!started) {
    await startPosthog();
  } else if (!enabled) {
    safely("opt_in_capturing", () => posthog.opt_in_capturing());
    enabled = true;
  }
  settleHeld();
}

/**
 * Send `diagnostics_opted_out`, at most once per install.
 *
 * **Straight to PostHog's capture endpoint, not through the client.** The
 * client is being switched off in the same tick, and the record must not
 * depend on which of the two wins: it goes by `fetch` with `keepalive`, the
 * same route `diagnosticsUpload.ts` uses, while the client's own opt-out
 * happens immediately beside it.
 *
 * **On the person, without merging anyone.** The ticket needs the funnel to
 * show this install as "opted out": the record is filed under the account's
 * `sub` when a Constellation sign-in is known (the person the dashboard and the
 * gateway use, whether or not this install was ever identified - no
 * `$identify` is sent, so the install id is not merged), otherwise under the
 * install id, and it carries the org group when the org is known. Props:
 * `source` only.
 *
 * **At most once, not at least once.** The marker is claimed before the send,
 * so a send that fails is not retried. A duplicate would count one person's
 * opt-out twice in every insight built on it; a lost one shows that install as
 * a drop-off, the state the funnel was already in before this existed. The
 * claim has no timeout racing it: an unanswered claim sends nothing.
 */
async function recordOptOut(source: ConsentSource): Promise<void> {
  // No build key, no destination: nothing may be posted, and the marker must
  // not be spent on a record that went nowhere.
  if (!POSTHOG_KEY_VALUE) return;
  const won = await analyticsMilestoneClaim("diagnostics_opted_out").catch(() => false);
  if (!won) return;
  const s = session;
  const distinctId =
    identifiedAs ??
    (s?.authMode === "oauth" && s.sub ? s.sub : null) ??
    // An install identified once belongs to that person; after its sign-out the
    // record goes under the client's own fresh id rather than back onto them.
    (everIdentified ? currentDistinctId() : installIdValue);
  if (!distinctId) return;
  const org = s?.orgId ?? storedOrg;
  try {
    await fetch(`${POSTHOG_HOST}/i/v0/e/`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      keepalive: true,
      body: JSON.stringify({
        api_key: POSTHOG_KEY_VALUE,
        event: "diagnostics_opted_out",
        distinct_id: distinctId,
        properties: { source, ...(org ? { $groups: { organization: org } } : {}) },
      }),
    });
  } catch (e) {
    console.warn("[gate] analytics opt-out record failed", e);
  }
}

/**
 * The id PostHog files this install's events under, or why there is none to
 * show, for the diagnostics report.
 */
export type AnalyticsId =
  | { kind: "id"; value: string }
  | { kind: "disabled" }
  | { kind: "unavailable" };

/**
 * The install id, or the account's Cognito `sub` once a Constellation sign-in
 * has identified the install - the one string that lets a pasted report be
 * lined up against its event stream.
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

/** Markers this window has already settled, so a signal that repeats (the
 *  relay's traffic report, every 30s per tool) costs one IPC, not one per
 *  report. The answer itself lives in Rust. */
const askedHere = new Set<string>();

/** How long a milestone waits for an API-key install's org before going out
 *  without it. The org arrives with the main window's first activity read;
 *  sending before it would leave the event ungrouped for good. */
export const ORG_WAIT_MS = 120_000;

/**
 * Send a funnel milestone once per install, sent instantly.
 *
 * - **Open**: claim the marker, and send on a win. Claimed only while the
 *   client is actually delivering, so a marker is never spent into a client
 *   that drops it.
 * - **Held** (the question is not answered yet): nothing is claimed; it waits.
 * - **Refused** (sharing off): claim and send nothing. It happened while the
 *   user had said no, so it is spent: a later opt-in cannot report it late.
 * - **Closed** (no key, unreadable preference): nothing is claimed, so a later
 *   launch that can confirm consent may still report it.
 *
 * A claim that fails sends nothing: a first launch reported twice is worse than
 * one not reported. Resolves once the milestone is settled (sent, spent or
 * held); the boolean is whether it was sent.
 */
export async function trackMilestone(
  event: MilestoneEvent,
  props?: Props,
  marker: string = event,
): Promise<boolean> {
  if (!POSTHOG_KEY_VALUE || askedHere.has(marker)) return false;
  // When it happened, not when it was released: a first launch held until the
  // diagnostics answer must still sort before the pairing that came after it.
  const at = new Date();
  if (booting) await booting;
  if (starting) await starting;
  if (funnelState() === "held") {
    if (heldMarkers.has(marker)) return false;
    heldMarkers.add(marker);
  }
  return new Promise<boolean>((resolve) => {
    funnel({
      run: async () => {
        if (askedHere.has(marker)) return resolve(false);
        await waitForOrg();
        if (askedHere.has(marker) || !capturing()) return resolve(false);
        askedHere.add(marker);
        let won = false;
        try {
          won = await analyticsMilestoneClaim(marker);
        } catch {
          askedHere.delete(marker);
          return resolve(false);
        }
        if (!won || !capturing()) return resolve(false);
        const clean = sanitize(props);
        safely("capture", () => posthog.capture(event, clean, { send_instantly: true, timestamp: at }));
        resolve(true);
      },
      spend: async () => {
        if (askedHere.has(marker)) return resolve(false);
        askedHere.add(marker);
        await analyticsMilestoneClaim(marker).catch(() => false);
        resolve(false);
      },
    });
    // Held or closed: settled from this caller's point of view.
    if (funnelState() === "held" || funnelState() === "closed") resolve(false);
  });
}

// ---------------------------------------------------------------------------
// Session and identity

/** What the shell knows about the signed-in session. */
export interface SessionFacts {
  /** A usable credential and, for OAuth, an organization. */
  signedIn: boolean;
  authMode: AuthMode | null;
  /** The Cognito `sub`, for an OAuth session. */
  sub: string | null;
  /** The organization this install routes for: the one chosen at sign-in, or
   *  for an API-key account the one the gateway resolved it to. */
  orgId: string | null;
}

let session: SessionFacts | null = null;
/** The `sub` the client is identified as, or null on the install id. */
let identifiedAs: string | null = null;
/** Whether this install has ever been identified (stored in Rust, sticky). */
let everIdentified = false;
/** The org and auth mode last stored by the sign-in window, for the windows that
 *  never read the account themselves. */
let storedOrg: string | null = null;
/** The identified `sub` the backend holds, which every window follows. */
let storedSub: string | null = null;
let storedAuthMode: string | null = null;
let groupedAs: string | null = null;
let orgChoices: number | null = null;
let orgWaiters: Array<() => void> = [];

function adoptStoredIdentity(stored: AnalyticsIdentity): void {
  identifiedAs = stored.identified_sub;
  storedSub = stored.identified_sub;
  everIdentified = stored.ever_identified;
  storedOrg = stored.org_id;
  storedAuthMode = stored.auth_mode;
}

function currentOrg(): string | null {
  return session?.orgId ?? storedOrg;
}

function currentAuthMode(): string | null {
  return session?.authMode ?? storedAuthMode;
}

/** Resolve once the org is known, or at once when there is no org to wait for
 *  (not an API-key install), or after `ORG_WAIT_MS` regardless. */
function waitForOrg(): Promise<void> {
  if (currentOrg() || currentAuthMode() !== "api_key") return Promise.resolve();
  return new Promise((resolve) => {
    const done = () => {
      clearTimeout(timer);
      resolve();
    };
    const timer = setTimeout(() => {
      orgWaiters = orgWaiters.filter((w) => w !== done);
      resolve();
    }, ORG_WAIT_MS);
    orgWaiters.push(done);
  });
}

function orgArrived(): void {
  if (!currentOrg() && currentAuthMode() === "api_key") return;
  const waiters = orgWaiters;
  orgWaiters = [];
  applyGroup();
  for (const w of waiters) w();
}

/**
 * Tell the seam who is signed in. Called by the window that owns sign-in (the
 * main window, in either shell) whenever account, OAuth or activity state
 * changes; cheap and idempotent. The other windows follow the stored identity
 * the backend broadcasts, not their own reads.
 */
export function noteSession(facts: SessionFacts): void {
  session = facts;
  // Not signed in any more is a sign-out: the account's id comes off this
  // client, and the record the other windows follow says so. Without this the
  // next write below carried the old `sub` straight back to disk.
  if (!facts.signedIn && (identifiedAs || storedSub)) {
    storedSub = null;
    syncToStoredIdentity();
    // No client to move (opted out, or no key): the record still must not name
    // the account that left.
    identifiedAs = null;
    lastPersisted = "";
  }
  persistIdentity();
  const state = funnelState();
  if (state === "open") applySessionNow();
  // A pairing that happens while the user has said no is spent like any other
  // milestone, so a later opt-in cannot report it late.
  else if (state === "refused") spendSession();
  orgArrived();
}

/** How many organizations the sign-in offered, for `pairing_completed`'s
 *  `org_count`. Called by the org picker's load. */
export function noteOrgChoices(count: number): void {
  orgChoices = count;
}

let lastPersisted = "";
/** Keep the backend's record of the org and auth mode current, so the tray and
 *  the next launch know them. Local only; nothing is sent. */
function persistIdentity(): void {
  const s = session;
  if (!s || !POSTHOG_KEY_VALUE) return;
  const next: AnalyticsIdentity = {
    identified_sub: identifiedAs,
    ever_identified: everIdentified,
    org_id: s.signedIn ? s.orgId : null,
    auth_mode: s.authMode,
  };
  const key = JSON.stringify(next);
  if (key === lastPersisted) return;
  lastPersisted = key;
  storedOrg = next.org_id;
  storedAuthMode = next.auth_mode;
  void setAnalyticsIdentity(next).catch(() => {});
}

function applyGroup(): void {
  const org = currentOrg();
  if (!org || groupedAs === org || funnelState() !== "open") return;
  safely("group", () => posthog.group("organization", org));
  groupedAs = org;
}

/** The session's effects, once the funnel is open: identity, group, pairing.
 *  A window that owns no session (the tray, the intro) follows the stored
 *  identity instead. */
function applySessionNow(): void {
  const s = session;
  if (s?.signedIn && s.authMode === "oauth" && s.sub) applyIdentity(s.sub);
  else syncToStoredIdentity();
  applyGroup();
  if (s?.signedIn && s.orgId) {
    void trackMilestone("pairing_completed", {
      auth_mode: s.authMode ?? "unknown",
      ...(s.authMode === "oauth" && orgChoices !== null ? { org_count: orgChoices } : {}),
    });
  }
}

/** A "no": the pairing that already happened is spent like any other milestone. */
function spendSession(): void {
  const s = session;
  if (s?.signedIn && s.orgId) void trackMilestone("pairing_completed");
}

/**
 * Move the client onto `sub`, the account the sign-in window sees.
 *
 * - **The install's first identification**: `identify(sub)`. PostHog merges the
 *   anonymous install person (and everything it sent before sign-in) INTO the
 *   `sub` person, which may already exist and be identified - the dashboard's
 *   person, who clicked the download. `identify` rather than `alias`: `alias`
 *   would ask PostHog's server to fold an already-identified id into another
 *   person, which it refuses ("Refused to merge an already identified user",
 *   https://posthog.com/docs/data/ingestion-warnings).
 * - **Any later change of account** (a different `sub`, or the same one after a
 *   sign-out): `reset()` first, which drops the old distinct id for a fresh
 *   random one, then `identify(sub)`. The only id merged into the new person is
 *   that fresh one, which has sent nothing. The install id is never merged a
 *   second time, so account B on A's machine is never attached to A's person.
 *
 * The result is stored in Rust, which tells the other windows, and is what the
 * next launch bootstraps from, so a signed-in launch sends no `$identify`.
 */
function applyIdentity(sub: string): void {
  if (identifiedAs === sub) return;
  if (everIdentified) resetClient();
  safely("identify", () => posthog.identify(sub));
  identifiedAs = sub;
  storedSub = sub;
  everIdentified = true;
  lastPersisted = "";
  persistIdentity();
}

function currentDistinctId(): string | null {
  if (!started) return null;
  try {
    return posthog.get_distinct_id() || null;
  } catch {
    return null;
  }
}

/** Drop the client's identity and groups, keeping its super-properties. */
function resetClient(): void {
  safely("reset", () => posthog.reset());
  safely("register", () => posthog.register(superProps));
  groupedAs = null;
  // `reset` also clears posthog-js's persisted opt-out (`consent.reset()`,
  // `consent.js` ~60), in storage every window shares. Put it back unless the
  // user is sharing, or a later launch would start a client that sends.
  if (consent !== true) safely("opt_out_capturing", () => posthog.opt_out_capturing());
}

/**
 * Follow an identity change the backend broadcast: another window signed in
 * (or out), or a sign-out or reset happened in the backend itself.
 *
 * - A `sub` this window is not on: reset and identify, never merging the
 *   install id (the sign-in window did that once, if it was due).
 * - No `sub` (signed out): reset to a fresh anonymous id. NOT back to the
 *   install id: once identified, the install id belongs to that account's
 *   person, so filing under it would keep sending as the account that left.
 *   `install_id` stays a super-property, so the machine is still visible on
 *   each event.
 */
function followIdentity(next: AnalyticsIdentity): void {
  storedSub = next.identified_sub;
  storedOrg = next.org_id;
  storedAuthMode = next.auth_mode;
  everIdentified = everIdentified || next.ever_identified;
  // A sign-out is followed whatever the funnel's state: it takes the account's
  // id off what this machine sends next, and sends nothing itself.
  if (!storedSub || funnelState() === "open") syncToStoredIdentity();
  orgArrived();
}

/** Put the client on the identity Rust holds, if it is on another one. */
function syncToStoredIdentity(): void {
  if (!started || storedSub === identifiedAs) return;
  resetClient();
  if (storedSub) {
    const sub = storedSub;
    safely("identify", () => posthog.identify(sub));
  }
  identifiedAs = storedSub;
  applyGroup();
}

/**
 * A tool was connected: its config written, or its proxy domain routed. Called
 * on every success; the milestone is once per tool per install.
 *
 * Connecting the Claude row (the `anthropic` domain) also reads whether Claude
 * Desktop keeps local Cowork off: see `reportCoworkSetting`.
 */
export function noteToolConnected(tool: string, surface: "config" | "domain"): void {
  void trackMilestone("tool_connected", { tool, surface }, `tool_connected.${tool}`);
  if (surface === "domain" && tool === "anthropic") void reportCoworkSetting();
}

/**
 * The relay saw a gateway-bound request leave for the gateway, from these tools.
 * The first one is `first_request_proxied`.
 */
export function noteTrafficObserved(tools: readonly (string | null)[]): void {
  const tool = tools.find((t): t is string => typeof t === "string" && t.length > 0);
  void trackMilestone("first_request_proxied", { source: "relay", ...(tool ? { tool } : {}) });
}

/**
 * The gateway named this machine as one it has had traffic from: the fallback
 * signal for `first_request_proxied` where the relay cannot report (Linux).
 */
export function noteGatewayAttributed(): void {
  void trackMilestone("first_request_proxied", { source: "gateway" });
}

// ---------------------------------------------------------------------------
// Connection failures (AG-960)

/**
 * The error contexts that are a step of connecting: a tool, a domain, routing
 * itself, the certificate, or the backend's own restore of those. A failure in
 * one also sends `connection_failed`, beside the `error_shown` every failure
 * sends. The sign-in and pairing steps report through `noteSetupFailure`.
 */
const CONNECTION_CONTEXTS: ReadonlySet<ErrorContext> = new Set<ErrorContext>([
  "connect",
  "provider_toggle",
  "proxy_toggle",
  "trust_ca",
  "restore_routing",
  "provider_restore",
]);

/** How long one window stays quiet about the same failure. */
const FAILURE_REPEAT_MS = 5 * 60 * 1000;
const lastFailure = new Map<string, number>();

/**
 * Send `connection_failed`, instantly rather than batched: the batch timer
 * lives in a webview the OS may throttle while hidden. Held like every AG-960
 * event until the diagnostics question is answered, so a failure before that
 * reaches PostHog when the answer does, not within a minute of happening.
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
  const at = new Date();
  funnel({
    run: async () => {
      if (!capturing()) return;
      safely("capture", () =>
        posthog.capture("connection_failed", clean, { send_instantly: true, timestamp: at }),
      );
    },
  });
}

/**
 * Report the Claude Desktop setting that keeps local Cowork off, if one does.
 * Once per install per setting, since it is a standing condition; the marker
 * is only claimed while the funnel is open.
 */
async function reportCoworkSetting(): Promise<void> {
  if (!POSTHOG_KEY_VALUE) return;
  if (booting) await booting;
  if (starting) await starting;
  funnel({
    run: async () => {
      const detail = await coworkSettingCheck().catch(() => null);
      if (!detail || !capturing()) return;
      const won = await analyticsMilestoneClaim(`cowork_setting_missing.${detail}`).catch(
        () => false,
      );
      if (!won) return;
      reportConnectionFailure("cowork_setting_missing", "connect", { tool: "anthropic", detail });
    },
  });
}

/**
 * A gateway read the shell made failed with a typed `FailureCode`: `rejected`
 * (the gateway refused the credential) is `auth_rejected`, `offline` is
 * `offline`. `signed_out` is not either: it means there was no credential to
 * send, which is a state of the app, not a failure of the connection.
 */
export function noteGatewayFailure(code: string): void {
  if (code === "rejected") reportConnectionFailure("auth_rejected", "gateway");
  else if (code === "offline") reportConnectionFailure("offline", "gateway");
}

/** The sign-in and pairing steps, for `connection_failed`'s `context`. */
export type SetupStep = "sign_in" | "org_list" | "org_select";

/**
 * A step of signing in or pairing failed. Sent beside the `error_shown` the
 * call site already sends, with a reason read from the error the same way a
 * connect's is (a 401 or a `rejected` envelope is `auth_rejected`).
 */
export function noteSetupFailure(err: unknown, step: SetupStep): void {
  reportConnectionFailure(connectionFailureReason(err, "sign_in"), step);
}

/**
 * Record a user-facing failure: a paired `error_shown` event plus a PostHog
 * exception. We send the *classified* title + context, never the raw Tauri
 * error string (it can carry hosts/paths). `props` lets a call site attach
 * extra allowlisted dimensions (e.g. which provider's toggle failed).
 *
 * A failure in a connecting context also sends `connection_failed` with its
 * reason: the backend's typed answer where it had one, otherwise read from the
 * error the same way the title is.
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
  const ctx = sanitize(errorContext());
  send(() => safely("captureException", () => posthog.captureException(err, ctx)));
}
