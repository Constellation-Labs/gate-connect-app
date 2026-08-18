import { beforeEach, describe, expect, it, vi } from "vitest";
import type { Mock } from "vitest";

/**
 * Consent, not event shapes.
 *
 * The channel is real - PostHog, started at boot - and until now it had no user
 * opt-out at all, while Settings offered a "Share diagnostic data" switch that
 * only recorded a preference. A switch that implies control it does not have is
 * worse than no switch, so these tests pin that the preference actually gates the
 * client.
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

/** Fresh module per test: consent state lives in module-level flags. */
async function load() {
  vi.resetModules();
  return import("./analytics");
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
