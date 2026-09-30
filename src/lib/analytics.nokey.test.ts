import { describe, expect, it, vi } from "vitest";

/**
 * A build with no PostHog key (every dev build, `pnpm app:local`) has no
 * destination, so nothing may leave and nothing may be spent - including the
 * opt-out record, which goes by `fetch` rather than through the client and so
 * did not inherit the client's own no-key no-op (AG-960, review round 2).
 */
vi.mock("./config", async (importOriginal) => ({
  ...(await importOriginal<typeof import("./config")>()),
  POSTHOG_KEY_VALUE: "",
}));
vi.mock("posthog-js", () => ({
  default: { init: vi.fn(), capture: vi.fn(), identify: vi.fn(), group: vi.fn() },
}));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn(async () => () => {}) }));
vi.mock("./api", () => ({
  getPreferences: vi.fn(async () => ({ share_diagnostics: true, share_diagnostics_recorded: true })),
  installId: vi.fn(async () => "install-1"),
  analyticsMilestoneClaim: vi.fn(async () => true),
  coworkSettingCheck: vi.fn(async () => null),
  analyticsIdentity: vi.fn(),
  setAnalyticsIdentity: vi.fn(async () => undefined),
}));

import posthog from "posthog-js";
import { analyticsMilestoneClaim, setAnalyticsIdentity } from "./api";
import { initAnalytics, noteSession, setAnalyticsConsent } from "./analytics";

describe("a build with no PostHog key", () => {
  it("posts no opt-out record, spends no marker, and sends nothing", async () => {
    const fetchMock = vi.fn(async () => new Response("{}"));
    vi.stubGlobal("fetch", fetchMock);
    await initAnalytics();
    noteSession({ signedIn: true, authMode: "oauth", sub: "sub-a", orgId: "org-1" });

    // Answered yes at onboarding, then switched off in Settings: a real
    // transition from sharing to not, which is what records an opt-out.
    await setAnalyticsConsent(true, "onboarding");
    await setAnalyticsConsent(false, "settings");
    await new Promise((r) => setTimeout(r, 0));

    expect(fetchMock).not.toHaveBeenCalled();
    expect(analyticsMilestoneClaim).not.toHaveBeenCalled();
    expect(setAnalyticsIdentity).not.toHaveBeenCalled();
    expect(posthog.init).not.toHaveBeenCalled();
    vi.unstubAllGlobals();
  });
});
