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
  | "signed_in"
  | "disconnected"
  | "key_replaced"
  | "proxy_enabled"
  | "proxy_disabled"
  | "provider_toggled"
  | "ca_trusted"
  | "error_shown";

type Props = Record<string, string | number | boolean>;

/**
 * Prop keys allowed on the wire. Anything not listed is dropped before send -
 * the backstop against a host/key/path slipping into an event payload.
 */
const ALLOWED_PROP_KEYS = new Set<string>([
  "has_account",
  "proxy_available",
  "provider",
  "enabled",
  "context",
  "title",
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

export function track(event: AnalyticsEvent, props?: Props): void {
  if (!enabled) return;
  posthog.capture(event, sanitize(props));
}

/**
 * Record a user-facing failure: a paired `error_shown` event plus a PostHog
 * exception. We send the *classified* title + context, never the raw Tauri
 * error string (it can carry hosts/paths).
 */
export function trackError(err: unknown, context: ErrorContext): void {
  if (!enabled) return;
  const { title } = classifyError(err, context);
  track("error_shown", { context, title });
  posthog.captureException(new Error(title), { context });
}

/**
 * Forward an uncaught JS exception (genuine frontend crash) with its real
 * stack. These are our own bugs, not Tauri error strings, so the stack is the
 * point and there's nothing of the user's to redact.
 */
export function captureException(err: unknown): void {
  if (!enabled) return;
  posthog.captureException(err);
}
