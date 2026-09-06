import { beforeEach, describe, expect, it, vi } from "vitest";
import type { Mock } from "vitest";

/**
 * Two promises, one channel.
 *
 * Consent: the channel is real - PostHog, started at boot - and until recently it
 * had no user opt-out at all, while Settings offered a "Share diagnostic data"
 * switch that only recorded a preference. A switch that implies control it does
 * not have is worse than no switch, so these tests pin that the preference
 * actually gates the client.
 *
 * Content: what a running client may send. Event props pass an allowlist, so a
 * sensitive value (gateway host, API key, path) cannot ride along on an event by
 * accident, and error events carry the *classified* title + context, never the
 * raw Tauri error string (it can carry hosts/paths).
 *
 * A build-time key is required for any of it to run, so the module is loaded with
 * one injected. Without that every path no-ops and the tests would pass while
 * proving nothing.
 */
vi.mock("./config", async (importOriginal) => ({
  ...(await importOriginal<typeof import("./config")>()),
  POSTHOG_KEY_VALUE: "phc_test",
  POSTHOG_HOST: "https://example.invalid",
}));

vi.mock("posthog-js", () => ({
  default: {
    init: vi.fn(),
    register: vi.fn(),
    capture: vi.fn(),
    captureException: vi.fn(),
    opt_in_capturing: vi.fn(),
    opt_out_capturing: vi.fn(),
    get_distinct_id: vi.fn(() => "anon-1"),
  },
}));

vi.mock("./api", () => ({ getPreferences: vi.fn() }));
vi.mock("@tauri-apps/api/app", () => ({ getVersion: vi.fn(async () => "1.2.3") }));
vi.mock("./platform", () => ({ fetchPlatform: vi.fn(async () => "linux") }));

import posthog from "posthog-js";
import { getPreferences } from "./api";
import { initAnalytics, track, trackError } from "./analytics";
import { classifyError } from "./errors";

/** Fresh module per test: consent state lives in module-level flags. */
async function load() {
  vi.resetModules();
  return import("./analytics");
}

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

beforeEach(() => {
  vi.clearAllMocks();
});

describe("initAnalytics consent", () => {
  it("starts the client when the user has not opted out", async () => {
    (getPreferences as Mock).mockResolvedValue({
      share_diagnostics: true,
      routing_health_notifications: true,
    });
    const { initAnalytics } = await load();

    await initAnalytics();

    expect(posthog.init).toHaveBeenCalledTimes(1);
  });

  /** The point of the whole change. */
  it("never constructs the client when the user has opted out", async () => {
    (getPreferences as Mock).mockResolvedValue({
      share_diagnostics: false,
      routing_health_notifications: true,
    });
    const { initAnalytics } = await load();

    await initAnalytics();

    expect(posthog.init).not.toHaveBeenCalled();
  });

  /**
   * Consent that cannot be confirmed is not consent. `preferences::load()` is
   * infallible in Rust, so this only happens when the IPC itself fails - and the
   * safe direction is silence.
   */
  it("does not collect when consent could not be read", async () => {
    (getPreferences as Mock).mockRejectedValue(new Error("ipc unavailable"));
    const { initAnalytics } = await load();

    await initAnalytics();

    expect(posthog.init).not.toHaveBeenCalled();
  });

  it("sends nothing after an opted-out start", async () => {
    (getPreferences as Mock).mockResolvedValue({
      share_diagnostics: false,
      routing_health_notifications: true,
    });
    const { initAnalytics, track } = await load();
    await initAnalytics();

    track("app_launched");

    expect(posthog.capture).not.toHaveBeenCalled();
  });
});

describe("setAnalyticsConsent", () => {
  it("opts a live client out, and stops sending immediately", async () => {
    (getPreferences as Mock).mockResolvedValue({
      share_diagnostics: true,
      routing_health_notifications: true,
    });
    const { initAnalytics, setAnalyticsConsent, track } = await load();
    await initAnalytics();

    setAnalyticsConsent(false);
    track("app_launched");

    expect(posthog.opt_out_capturing).toHaveBeenCalledTimes(1);
    expect(posthog.capture).not.toHaveBeenCalled();
  });

  it("starts the client the first time consent is given in a session", async () => {
    // The opted-out install: nothing was constructed at boot, so turning the
    // switch on has to do the init rather than only opt back in.
    (getPreferences as Mock).mockResolvedValue({
      share_diagnostics: false,
      routing_health_notifications: true,
    });
    const { initAnalytics, setAnalyticsConsent, track } = await load();
    await initAnalytics();
    expect(posthog.init).not.toHaveBeenCalled();

    setAnalyticsConsent(true);
    track("app_launched");

    expect(posthog.init).toHaveBeenCalledTimes(1);
    expect(posthog.opt_in_capturing).not.toHaveBeenCalled();
    expect(posthog.capture).toHaveBeenCalledTimes(1);
  });

  it("opts back in rather than re-initialising an existing client", async () => {
    (getPreferences as Mock).mockResolvedValue({
      share_diagnostics: true,
      routing_health_notifications: true,
    });
    const { initAnalytics, setAnalyticsConsent } = await load();
    await initAnalytics();

    setAnalyticsConsent(false);
    setAnalyticsConsent(true);

    expect(posthog.init).toHaveBeenCalledTimes(1);
    expect(posthog.opt_in_capturing).toHaveBeenCalledTimes(1);
  });

  it("is safe to call with the value already in force", async () => {
    (getPreferences as Mock).mockResolvedValue({
      share_diagnostics: false,
      routing_health_notifications: true,
    });
    const { initAnalytics, setAnalyticsConsent } = await load();
    await initAnalytics();

    setAnalyticsConsent(false);
    setAnalyticsConsent(false);

    // Nothing to opt out of - the client was never built.
    expect(posthog.opt_out_capturing).not.toHaveBeenCalled();
    expect(posthog.init).not.toHaveBeenCalled();
  });
});

describe("track: the event-prop allowlist", () => {
  // The consent gate is the subject of the suite above, not this one: these
  // tests are about what a *running* client is allowed to send, so they grant
  // consent and wait for the client to exist before asserting on it.
  beforeEach(async () => {
    (getPreferences as Mock).mockResolvedValue({
      share_diagnostics: true,
      routing_health_notifications: true,
    });
    await initAnalytics();
  });

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
  // The consent gate is the subject of the suite above, not this one: these
  // tests are about what a *running* client is allowed to send, so they grant
  // consent and wait for the client to exist before asserting on it.
  beforeEach(async () => {
    (getPreferences as Mock).mockResolvedValue({
      share_diagnostics: true,
      routing_health_notifications: true,
    });
    await initAnalytics();
  });

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
