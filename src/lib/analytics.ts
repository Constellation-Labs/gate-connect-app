/**
 * Analytics seam. The rest of the app calls `track` / `trackError` here and
 * never touches posthog-js directly, so the whole thing no-ops cleanly when no
 * build-time key is configured.
 *
 * Privacy posture for a credentials product: manual events only - no
 * autocapture, no session recording, no auto-pageviews - and anonymous-only
 * (we never `identify`). Event props pass through an allowlist so a sensitive
 * value (gateway host, API key) can't ride along by accident, and Tauri-side
 * errors are classified before send rather than shipping the raw string.
 */
import posthog from "posthog-js";
import { getVersion } from "@tauri-apps/api/app";
import { POSTHOG_KEY_VALUE, POSTHOG_HOST } from "./config";
import { fetchPlatform } from "./platform";
import { classifyError, type ErrorContext } from "./errors";

/** The only event names we ever emit. */
export type AnalyticsEvent =
  | "app_launched"
  | "popover_opened"
  | "signed_in"
  | "workspace_forgotten"
  | "key_replaced"
  | "proxy_enabled"
  | "proxy_disabled"
  | "provider_toggled"
  | "domain_toggled"
  | "tool_toggled"
  | "group_toggled"
  | "ca_trusted"
  | "ca_untrusted"
  | "tour_completed"
  | "tour_skipped"
  | "update_shown"
  | "update_installed"
  | "update_dismissed"
  | "agents_closed"
  | "stale_agents_shown"
  | "routing_notice_shown"
  | "quit_warning_shown"
  | "oauth_offer_shown"
  | "oauth_offer_accepted"
  | "quit_confirmed"
  | "launch_at_login_toggled"
  | "error_shown";

type Props = Record<string, string | number | boolean>;

/**
 * Prop keys allowed on the wire. Anything not listed is dropped before send -
 * the backstop against a host/key/path slipping into an event payload.
 */
const ALLOWED_PROP_KEYS = new Set<string>([
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
  "tool_count",
  "integrations_disabled",
]);

let enabled = false;

function sanitize(props?: Props): Props | undefined {
  if (!props) return undefined;
  const out: Props = {};
  for (const [k, v] of Object.entries(props)) {
    if (ALLOWED_PROP_KEYS.has(k)) out[k] = v;
  }
  return out;
}

/**
 * Initialize PostHog. No-op (analytics stays disabled) when no build-time key
 * is present, so dev builds and unconfigured releases send nothing. App version
 * and platform ride along as super-properties - both coarse and non-identifying
 * - so events are attributable to a build.
 */
export function initAnalytics(): void {
  if (!POSTHOG_KEY_VALUE) return;
  posthog.init(POSTHOG_KEY_VALUE, {
    api_host: POSTHOG_HOST,
    autocapture: false,
    capture_pageview: false,
    capture_pageleave: false,
    disable_session_recording: true,
    person_profiles: "identified_only",
  });
  enabled = true;

  void Promise.all([getVersion().catch(() => "unknown"), fetchPlatform()]).then(
    ([app_version, platform]) => {
      posthog.register({ app_version, platform });
    },
  );
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

export function track(event: AnalyticsEvent, props?: Props): void {
  if (!enabled) return;
  safely("capture", () => posthog.capture(event, sanitize(props)));
}

/**
 * Record a user-facing failure: a paired `error_shown` event plus a PostHog
 * exception. We send the *classified* title + context, never the raw Tauri
 * error string (it can carry hosts/paths). `props` lets a call site attach
 * extra allowlisted dimensions (e.g. which provider's toggle failed).
 */
export function trackError(err: unknown, context: ErrorContext, props?: Props): void {
  if (!enabled) return;
  const { title } = classifyError(err, context);
  track("error_shown", { ...props, context, title });
  safely("captureException", () => posthog.captureException(new Error(title), { context }));
}

/**
 * Forward an uncaught JS exception (genuine frontend crash) with its real
 * stack. These are our own bugs, not Tauri error strings, so the stack is the
 * point and there's nothing of the user's to redact.
 */
export function captureException(err: unknown): void {
  if (!enabled) return;
  safely("captureException", () => posthog.captureException(err));
}
