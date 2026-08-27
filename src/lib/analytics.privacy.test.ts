import { describe, it, expect, vi, beforeEach } from "vitest";
import posthog from "posthog-js";
import { initAnalytics, track, trackError } from "./analytics";
import { classifyError } from "./errors";

// These tests pin the privacy promises analytics.ts makes in its header
// comment, not PostHog plumbing. Separate from `analytics.test.ts`, which pins
// the consent gate: that suite reloads the module per test to reset the
// consent flags, and this one needs a single statically-imported instance.
// The load-bearing guarantees:
//   1. event props pass an allowlist, so a sensitive value (gateway host,
//      API key, path) can't ride along on an event by accident
//   2. error events carry the *classified* title + context, never the raw
//      Tauri error string (it can carry hosts/paths)

vi.mock("posthog-js", () => ({
  default: {
    init: vi.fn(),
    register: vi.fn(),
    capture: vi.fn(),
    captureException: vi.fn(),
    get_distinct_id: vi.fn(() => "device-id"),
  },
}));

// A build-time key must be present or the whole module stays a no-op and
// nothing below would exercise the seam.
vi.mock("./config", async (importOriginal) => ({
  ...(await importOriginal<typeof import("./config")>()),
  POSTHOG_KEY_VALUE: "phc_test",
}));

vi.mock("@tauri-apps/api/app", () => ({
  getVersion: vi.fn(async () => "0.0.0-test"),
}));

vi.mock("./platform", async (importOriginal) => ({
  ...(await importOriginal<typeof import("./platform")>()),
  fetchPlatform: vi.fn(async () => "macos" as const),
}));

// `initAnalytics` is consent-gated, and an unreadable preference reads as "no",
// so without this the client never starts and every assertion below would pass
// against a channel that sent nothing. Consent itself is pinned in
// `analytics.test.ts`; here it is only a precondition.
vi.mock("./api", () => ({
  getPreferences: vi.fn(async () => ({
    routing_health_notifications: true,
    share_diagnostics: true,
    share_diagnostics_recorded: true,
    device_name: null,
  })),
}));

/** Every argument PostHog received, flattened to one string, so a test can
 *  assert a secret appears nowhere in anything we sent. */
function everythingSent(): string {
  const mocked = vi.mocked(posthog);
  return JSON.stringify([
    ...mocked.capture.mock.calls,
    ...mocked.captureException.mock.calls.map((args) =>
      args.map((a) => (a instanceof Error ? `${a.name}: ${a.message}` : a)),
    ),
  ]);
}

beforeEach(async () => {
  vi.clearAllMocks();
  await initAnalytics();
});

describe("track: the event-prop allowlist", () => {
  it("drops any prop key not on the allowlist", () => {
    track("tool_toggled", {
      tool: "codex",
      routed: true,
      gateway_host: "gateway.internal.example",
      api_key: "sk-secret-value",
    });
    expect(posthog.capture).toHaveBeenCalledTimes(1);
    const [event, props] = vi.mocked(posthog.capture).mock.calls[0];
    expect(event).toBe("tool_toggled");
    expect(props).toEqual({ tool: "codex", routed: true });
  });

  it("never sends the dropped values in any form", () => {
    track("proxy_enabled", { source: "toggle", data_dir: "/Users/someone/Library" });
    expect(everythingSent()).not.toContain("/Users/someone/Library");
  });
});

describe("trackError: raw error strings stay on this machine", () => {
  // A raw Tauri error chain of the shape that motivated the classification:
  // it names a host, which must never leave the machine.
  const RAW = "connection refused by https://gateway.internal.example:8443";

  it("sends the classified title and context, not the raw string", () => {
    trackError(RAW, "proxy_toggle");
    const { title } = classifyError(RAW, "proxy_toggle");
    expect(posthog.capture).toHaveBeenCalledWith("error_shown", {
      context: "proxy_toggle",
      title,
    });
    expect(everythingSent()).not.toContain("gateway.internal.example");
  });

  it("files the exception under the classified title too", () => {
    trackError(new Error(RAW), "connect");
    const { title } = classifyError(new Error(RAW), "connect");
    const [err] = vi.mocked(posthog.captureException).mock.calls[0];
    expect(err).toBeInstanceOf(Error);
    expect((err as Error).message).toBe(title);
    expect(everythingSent()).not.toContain("gateway.internal.example");
  });

  it("runs extra caller props through the same allowlist", () => {
    trackError(RAW, "provider_toggle", {
      domain: "anthropic",
      upstream_url: "https://gateway.internal.example",
    });
    const [, props] = vi.mocked(posthog.capture).mock.calls[0];
    expect(props).toHaveProperty("domain", "anthropic");
    expect(props).not.toHaveProperty("upstream_url");
  });
});
