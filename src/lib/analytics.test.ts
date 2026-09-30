import { beforeEach, describe, expect, it, vi } from "vitest";
import type { Mock } from "vitest";

/**
 * Three promises, one channel.
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
 * The install funnel (AG-960): events are filed under the install id from the
 * first one, joined to the account and the org at pairing, milestones are sent
 * once per install, an opt-out is recorded once and before capture stops, and a
 * connection failure carries a reason from a closed list.
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
    identify: vi.fn(),
    group: vi.fn(),
    opt_in_capturing: vi.fn(),
    opt_out_capturing: vi.fn(),
    get_distinct_id: vi.fn(() => "anon-1"),
  },
}));

vi.mock("./api", () => ({
  getPreferences: vi.fn(),
  installId: vi.fn(),
  analyticsMilestoneClaim: vi.fn(),
  coworkSettingCheck: vi.fn(),
}));
vi.mock("@tauri-apps/api/app", () => ({ getVersion: vi.fn(async () => "1.2.3") }));
vi.mock("./platform", () => ({ fetchPlatform: vi.fn(async () => "linux") }));

import posthog from "posthog-js";
import { analyticsMilestoneClaim, coworkSettingCheck, getPreferences, installId } from "./api";
import { classifyError, type ConnectionFailureReason, type ErrorContext } from "./errors";

const INSTALL_ID = "3f0c9a52-7d1e-4b8a-9c2f-1a2b3c4d5e6f";
const SUB = "0b8c1f2e-1111-4222-8333-944455556666";
const ORG = "9d7e6f5a-aaaa-4bbb-8ccc-0123456789ab";

/**
 * The Rust marker store, faked with its one property: a name is won once. Kept
 * across `load()`s on purpose, because a module reload is what a second window
 * (or a relaunch) looks like to this code, and the marker must outlive both.
 */
const claimed = new Set<string>();

/** Fresh module per test: consent state lives in module-level flags. */
async function load() {
  vi.resetModules();
  return import("./analytics");
}

function consentIs(share: boolean) {
  (getPreferences as Mock).mockResolvedValue({ share_diagnostics: share, notifications: true });
}

/** Let pending promise chains (IPC mocks, the boot, a claim) run out. */
async function settle() {
  for (let i = 0; i < 10; i++) await Promise.resolve();
  await new Promise((r) => setTimeout(r, 0));
}

/** The captures of one event name, props only. */
function sent(event: string): unknown[] {
  return vi
    .mocked(posthog.capture)
    .mock.calls.filter(([name]) => name === event)
    .map(([, props]) => props);
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
  claimed.clear();
  (installId as Mock).mockResolvedValue(INSTALL_ID);
  (analyticsMilestoneClaim as Mock).mockImplementation(async (name: string) => {
    if (claimed.has(name)) return false;
    claimed.add(name);
    return true;
  });
  (coworkSettingCheck as Mock).mockResolvedValue(null);
});

describe("initAnalytics consent", () => {
  it("starts the client when the user has not opted out", async () => {
    consentIs(true);
    const { initAnalytics } = await load();

    await initAnalytics();

    expect(posthog.init).toHaveBeenCalledTimes(1);
  });

  /** The point of the whole change. */
  it("never constructs the client when the user has opted out", async () => {
    consentIs(false);
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
    consentIs(false);
    const { initAnalytics, track } = await load();
    await initAnalytics();

    track("app_launched");
    await settle();

    expect(posthog.capture).not.toHaveBeenCalled();
  });

  /** An event tracked while the consent read is in flight waits for the answer,
   *  and a no drops it rather than sending it later. */
  it("drops an event tracked during the boot when the answer is no", async () => {
    consentIs(false);
    const { initAnalytics, track } = await load();
    const boot = initAnalytics();
    track("app_launched");
    await boot;
    await settle();

    expect(posthog.capture).not.toHaveBeenCalled();
  });
});

describe("identity: the install id from the first event (AG-960)", () => {
  it("bootstraps the install id as the distinct id", async () => {
    consentIs(true);
    const { initAnalytics } = await load();

    await initAnalytics();

    const [, config] = vi.mocked(posthog.init).mock.calls[0];
    expect(config).toMatchObject({
      bootstrap: { distinctID: INSTALL_ID },
      person_profiles: "identified_only",
      autocapture: false,
    });
  });

  it("registers version, platform and install id before the first capture", async () => {
    consentIs(true);
    const { initAnalytics, track } = await load();
    await initAnalytics();
    track("app_launched");

    expect(posthog.register).toHaveBeenCalledWith({
      app_version: "1.2.3",
      platform: "linux",
      install_id: INSTALL_ID,
    });
    const registered = vi.mocked(posthog.register).mock.invocationCallOrder[0];
    for (const order of vi.mocked(posthog.capture).mock.invocationCallOrder) {
      expect(registered).toBeLessThan(order);
    }
  });

  /**
   * The regression. The old init fired `register` in an unawaited `then` and
   * never bootstrapped an id, and every event tracked before the client existed
   * was dropped: the first events of a fresh install either vanished or went out
   * under a random browser id with no version, and a storage reset made the same
   * machine a new person.
   */
  it("holds an event tracked during the boot and sends it once identity is set", async () => {
    let answer!: (v: unknown) => void;
    (getPreferences as Mock).mockReturnValue(new Promise((r) => (answer = r)));
    const { initAnalytics, track } = await load();
    const boot = initAnalytics();

    track("app_launched", { has_account: false });
    expect(posthog.capture).not.toHaveBeenCalled();
    answer({ share_diagnostics: true });
    await boot;

    expect(sent("app_launched")).toEqual([{ has_account: false }]);
    const init = vi.mocked(posthog.init).mock.invocationCallOrder[0];
    const register = vi.mocked(posthog.register).mock.invocationCallOrder[0];
    const capture = vi
      .mocked(posthog.capture)
      .mock.calls.findIndex(([name]) => name === "app_launched");
    const captureOrder = vi.mocked(posthog.capture).mock.invocationCallOrder[capture];
    expect(init).toBeLessThan(captureOrder);
    expect(register).toBeLessThan(captureOrder);
    expect(vi.mocked(posthog.init).mock.calls[0][1]).toMatchObject({
      bootstrap: { distinctID: INSTALL_ID },
    });
  });

  it("still starts, unidentified by install, when the install id cannot be read", async () => {
    consentIs(true);
    (installId as Mock).mockRejectedValue(new Error("data dir unreadable"));
    const { initAnalytics } = await load();

    await initAnalytics();

    const [, config] = vi.mocked(posthog.init).mock.calls[0];
    expect(config).not.toHaveProperty("bootstrap");
  });
});

describe("pairing: group and identity", () => {
  it("identifies an OAuth install with the sub and groups it before pairing_completed", async () => {
    consentIs(true);
    const { initAnalytics, noteOrgChoices, noteSession } = await load();
    await initAnalytics();

    noteOrgChoices(3);
    noteSession({ signedIn: true, authMode: "oauth", sub: SUB, orgId: ORG });
    await settle();

    expect(posthog.identify).toHaveBeenCalledWith(SUB);
    expect(posthog.group).toHaveBeenCalledWith("organization", ORG);
    expect(sent("pairing_completed")).toEqual([{ auth_mode: "oauth", org_count: 3 }]);
    const pairing = vi
      .mocked(posthog.capture)
      .mock.calls.findIndex(([name]) => name === "pairing_completed");
    const pairingAt = vi.mocked(posthog.capture).mock.invocationCallOrder[pairing];
    expect(vi.mocked(posthog.identify).mock.invocationCallOrder[0]).toBeLessThan(pairingAt);
    expect(vi.mocked(posthog.group).mock.invocationCallOrder[0]).toBeLessThan(pairingAt);
  });

  /** An API key carries no user: grouped by the org the gateway resolved, never
   *  identified, so it can never be merged into somebody's person. */
  it("groups an API-key install without identifying it", async () => {
    consentIs(true);
    const { initAnalytics, noteSession } = await load();
    await initAnalytics();

    noteSession({ signedIn: true, authMode: "api_key", sub: null, orgId: ORG });
    await settle();

    expect(posthog.identify).not.toHaveBeenCalled();
    expect(posthog.group).toHaveBeenCalledWith("organization", ORG);
    expect(sent("pairing_completed")).toEqual([{ auth_mode: "api_key" }]);
  });

  it("does nothing for a session that is not signed in, and does not repeat itself", async () => {
    consentIs(true);
    const { initAnalytics, noteSession } = await load();
    await initAnalytics();

    noteSession({ signedIn: false, authMode: "oauth", sub: SUB, orgId: null });
    await settle();
    expect(posthog.identify).not.toHaveBeenCalled();

    const paired = { signedIn: true, authMode: "oauth" as const, sub: SUB, orgId: ORG };
    noteSession(paired);
    noteSession(paired);
    noteSession(paired);
    await settle();
    expect(posthog.identify).toHaveBeenCalledTimes(1);
    expect(posthog.group).toHaveBeenCalledTimes(1);
    expect(sent("pairing_completed")).toHaveLength(1);
  });

  /** The session is known before the client exists (the boot read is slower
   *  than the account read); it is applied as soon as the client starts. */
  it("applies a session noted before the client started", async () => {
    consentIs(true);
    const { initAnalytics, noteSession } = await load();
    noteSession({ signedIn: true, authMode: "oauth", sub: SUB, orgId: ORG });
    expect(posthog.identify).not.toHaveBeenCalled();

    await initAnalytics();
    await settle();

    expect(posthog.identify).toHaveBeenCalledWith(SUB);
    expect(sent("pairing_completed")).toHaveLength(1);
  });

  it("never sends the org's name or the account's email", async () => {
    consentIs(true);
    const { initAnalytics, noteSession } = await load();
    await initAnalytics();
    noteSession({ signedIn: true, authMode: "oauth", sub: SUB, orgId: ORG });
    await settle();

    expect(vi.mocked(posthog.group).mock.calls[0]).toEqual(["organization", ORG]);
    expect(vi.mocked(posthog.identify).mock.calls[0]).toEqual([SUB]);
  });
});

describe("milestones: once per install", () => {
  it("sends app_first_launched at the first boot, and never again", async () => {
    consentIs(true);
    let mod = await load();
    await mod.initAnalytics();
    await settle();
    expect(sent("app_first_launched")).toHaveLength(1);

    // A second window, or the next launch: a fresh module, the same store.
    mod = await load();
    await mod.initAnalytics();
    await settle();
    expect(sent("app_first_launched")).toHaveLength(1);
  });

  it("sends each milestone once across repeated calls", async () => {
    consentIs(true);
    const { initAnalytics, noteToolConnected, noteTrafficObserved } = await load();
    await initAnalytics();

    for (let i = 0; i < 5; i++) {
      noteToolConnected("codex", "config");
      noteTrafficObserved(["codex"]);
    }
    noteToolConnected("claude-code", "config");
    await settle();

    expect(sent("tool_connected")).toEqual([
      { tool: "codex", surface: "config" },
      { tool: "claude-code", surface: "config" },
    ]);
    expect(sent("first_request_proxied")).toEqual([{ source: "relay", tool: "codex" }]);
  });

  it("files a gateway-attributed first request under its own source", async () => {
    consentIs(true);
    const { initAnalytics, noteGatewayAttributed, noteTrafficObserved } = await load();
    await initAnalytics();

    noteGatewayAttributed();
    noteTrafficObserved([null]);
    await settle();

    expect(sent("first_request_proxied")).toEqual([{ source: "gateway" }]);
  });

  /** Opted out when it happened: spent, not deferred, so opting back in later
   *  cannot report it weeks late. */
  it("spends a milestone that happens while opted out, and sends nothing", async () => {
    consentIs(false);
    const { initAnalytics, noteToolConnected, setAnalyticsConsent } = await load();
    await initAnalytics();

    noteToolConnected("codex", "config");
    await settle();
    expect(claimed.has("tool_connected.codex")).toBe(true);

    await setAnalyticsConsent(true);
    noteToolConnected("codex", "config");
    await settle();
    expect(sent("tool_connected")).toEqual([]);
  });

  it("leaves a milestone unclaimed when consent is unknown", async () => {
    (getPreferences as Mock).mockRejectedValue(new Error("ipc unavailable"));
    const { initAnalytics, noteToolConnected } = await load();
    await initAnalytics();

    noteToolConnected("codex", "config");
    await settle();

    expect(analyticsMilestoneClaim).not.toHaveBeenCalled();
    expect(posthog.capture).not.toHaveBeenCalled();
  });

  it("sends nothing when the store cannot answer", async () => {
    consentIs(true);
    (analyticsMilestoneClaim as Mock).mockRejectedValue("data dir unwritable");
    const { initAnalytics, noteToolConnected } = await load();
    await initAnalytics();

    noteToolConnected("codex", "config");
    await settle();

    expect(sent("app_first_launched")).toEqual([]);
    expect(sent("tool_connected")).toEqual([]);
  });
});

describe("setAnalyticsConsent", () => {
  it("opts a live client out, and stops sending immediately", async () => {
    consentIs(true);
    const { initAnalytics, setAnalyticsConsent, track } = await load();
    await initAnalytics();
    await settle();
    vi.mocked(posthog.capture).mockClear();

    const done = setAnalyticsConsent(false);
    track("app_launched");
    await done;
    track("app_launched");

    expect(posthog.opt_out_capturing).toHaveBeenCalledTimes(1);
    expect(sent("app_launched")).toEqual([]);
  });

  /**
   * The regression for "opt-out not recorded": the old switch opted out and
   * sent nothing, so an install that said no was indistinguishable from one
   * that stopped being used.
   */
  it("records the opt-out once, instantly, before capture stops", async () => {
    consentIs(true);
    const { initAnalytics, setAnalyticsConsent } = await load();
    await initAnalytics();

    await setAnalyticsConsent(false, "onboarding_skip");

    const calls = vi.mocked(posthog.capture).mock.calls;
    const at = calls.findIndex(([name]) => name === "diagnostics_opted_out");
    expect(at).toBeGreaterThanOrEqual(0);
    expect(calls[at][1]).toEqual({ source: "onboarding_skip" });
    expect(calls[at][2]).toEqual({ send_instantly: true });
    expect(vi.mocked(posthog.capture).mock.invocationCallOrder[at]).toBeLessThan(
      vi.mocked(posthog.opt_out_capturing).mock.invocationCallOrder[0],
    );
  });

  it("does not record a second opt-out after opting back in", async () => {
    consentIs(true);
    const { initAnalytics, setAnalyticsConsent } = await load();
    await initAnalytics();

    await setAnalyticsConsent(false, "settings");
    await setAnalyticsConsent(true, "settings");
    await setAnalyticsConsent(false, "settings");

    expect(sent("diagnostics_opted_out")).toHaveLength(1);
    expect(posthog.opt_out_capturing).toHaveBeenCalledTimes(2);
  });

  it("stops the client even if the marker store never answers", async () => {
    vi.useFakeTimers();
    try {
      consentIs(true);
      (analyticsMilestoneClaim as Mock).mockImplementation(() => new Promise(() => {}));
      const { initAnalytics, setAnalyticsConsent } = await load();
      await initAnalytics();

      const done = setAnalyticsConsent(false);
      await vi.advanceTimersByTimeAsync(2500);
      await done;

      expect(posthog.opt_out_capturing).toHaveBeenCalledTimes(1);
      expect(sent("diagnostics_opted_out")).toEqual([]);
    } finally {
      vi.useRealTimers();
    }
  });

  it("starts the client the first time consent is given in a session", async () => {
    // The opted-out install: nothing was constructed at boot, so turning the
    // switch on has to do the init rather than only opt back in.
    consentIs(false);
    const { initAnalytics, setAnalyticsConsent, track } = await load();
    await initAnalytics();
    expect(posthog.init).not.toHaveBeenCalled();

    const done = setAnalyticsConsent(true);
    track("app_launched");
    await done;

    expect(posthog.init).toHaveBeenCalledTimes(1);
    expect(posthog.opt_in_capturing).not.toHaveBeenCalled();
    expect(sent("app_launched")).toHaveLength(1);
  });

  it("opts back in rather than re-initialising an existing client", async () => {
    consentIs(true);
    const { initAnalytics, setAnalyticsConsent } = await load();
    await initAnalytics();

    await setAnalyticsConsent(false);
    await setAnalyticsConsent(true);

    expect(posthog.init).toHaveBeenCalledTimes(1);
    expect(posthog.opt_in_capturing).toHaveBeenCalledTimes(1);
  });

  it("is safe to call with the value already in force", async () => {
    consentIs(false);
    const { initAnalytics, setAnalyticsConsent } = await load();
    await initAnalytics();

    await setAnalyticsConsent(false);
    await setAnalyticsConsent(false);

    // Nothing to opt out of - the client was never built, and nothing to record.
    expect(posthog.opt_out_capturing).not.toHaveBeenCalled();
    expect(posthog.init).not.toHaveBeenCalled();
    expect(analyticsMilestoneClaim).not.toHaveBeenCalledWith("diagnostics_opted_out");
  });
});

describe("connection_failed: a reason from a closed list", () => {
  async function running() {
    consentIs(true);
    const mod = await load();
    await mod.initAnalytics();
    await settle();
    vi.mocked(posthog.capture).mockClear();
    return mod;
  }

  const CASES: [string, ErrorContext, ConnectionFailureReason][] = [
    [
      "starting the engine: the relay port 45981 is already in use. Another relay host is likely running",
      "proxy_toggle",
      "port_in_use",
    ],
    ["bind 127.0.0.1:8080: Address already in use (os error 48)", "connect", "port_in_use"],
    ["Address already in use (os error 98)", "restore_routing", "port_in_use"],
    [
      "Only one usage of each socket address (protocol/network address/port) is normally permitted. (os error 10048)",
      "connect",
      "port_in_use",
    ],
    ["address in use", "connect", "port_in_use"],
    ["the proxy is not running; turn routing on first", "connect", "routing_off"],
    ["the certificate trust dialog was cancelled", "trust_ca", "ca_trust_declined"],
    ["execution error: User canceled. (-128)", "trust_ca", "ca_trust_declined"],
    ["execution error: User canceled. (-128)", "proxy_toggle", "prompt_declined"],
    ["error sending request: connection refused", "connect", "offline"],
    ["gateway answered 401 Unauthorized", "provider_toggle", "auth_rejected"],
    ["disk quota exceeded writing /Users/x/.codex/config.toml", "connect", "unknown"],
  ];

  for (const [raw, context, reason] of CASES) {
    it(`files "${raw.slice(0, 40)}..." in ${context} as ${reason}`, async () => {
      const { trackError } = await running();

      trackError(raw, context, { tool: "codex" });

      const [props] = sent("connection_failed");
      expect(props).toEqual({ reason, context, tool: "codex" });
      const call = vi
        .mocked(posthog.capture)
        .mock.calls.find(([name]) => name === "connection_failed");
      expect(call?.[2]).toEqual({ send_instantly: true });
    });
  }

  it("never puts the raw message on the wire", async () => {
    const { trackError } = await running();
    trackError("disk quota exceeded writing /Users/x/.codex/config.toml", "connect");
    expect(everythingSent()).not.toContain("/Users/x");
  });

  it("prefers the backend's typed reason over the message", async () => {
    const { trackError } = await running();
    trackError("starting the relay failed", "restore_routing", undefined, "port_in_use");
    expect(sent("connection_failed")).toEqual([
      { reason: "port_in_use", context: "restore_routing" },
    ]);
  });

  it("names a domain as the tool", async () => {
    const { trackError } = await running();
    trackError("connection refused", "provider_toggle", { domain: "anthropic", routed: true });
    expect(sent("connection_failed")).toEqual([
      { reason: "offline", context: "provider_toggle", tool: "anthropic" },
    ]);
  });

  it("is not sent for a failure outside a connecting step", async () => {
    const { trackError } = await running();
    trackError("connection refused", "update");
    trackError("connection refused", "sign_out");
    expect(sent("connection_failed")).toEqual([]);
    expect(sent("error_shown")).toHaveLength(2);
  });

  it("reports the same failure once per window, not on every retry", async () => {
    const { trackError } = await running();
    for (let i = 0; i < 4; i++) trackError("address in use", "connect", { tool: "codex" });
    trackError("address in use", "connect", { tool: "opencode" });
    expect(sent("connection_failed")).toHaveLength(2);
  });

  it("files a refused gateway read as auth_rejected and an unreachable one as offline", async () => {
    const { noteGatewayFailure } = await running();
    noteGatewayFailure("rejected");
    noteGatewayFailure("offline");
    noteGatewayFailure("gateway");
    noteGatewayFailure("no_org");
    expect(sent("connection_failed")).toEqual([
      { reason: "auth_rejected", context: "gateway" },
      { reason: "offline", context: "gateway" },
    ]);
  });

  it("reports a Claude setting that keeps local Cowork off, once per setting", async () => {
    const { noteToolConnected } = await running();
    (coworkSettingCheck as Mock).mockResolvedValue("user");

    noteToolConnected("anthropic", "domain");
    await settle();
    noteToolConnected("anthropic", "domain");
    await settle();

    expect(sent("connection_failed")).toEqual([
      { reason: "cowork_setting_missing", context: "connect", tool: "anthropic", detail: "user" },
    ]);
    expect(sent("tool_connected")).toEqual([{ tool: "anthropic", surface: "domain" }]);
  });

  it("does not read Claude's settings for any other connect", async () => {
    const { noteToolConnected } = await running();
    noteToolConnected("codex", "config");
    noteToolConnected("openai", "domain");
    await settle();
    expect(coworkSettingCheck).not.toHaveBeenCalled();
  });

  it("says nothing when Cowork is not blocked", async () => {
    const { noteToolConnected } = await running();
    noteToolConnected("anthropic", "domain");
    await settle();
    expect(sent("connection_failed")).toEqual([]);
  });

  it("sends nothing at all while opted out", async () => {
    consentIs(false);
    const { initAnalytics, trackError, noteGatewayFailure } = await load();
    await initAnalytics();
    trackError("address in use", "connect");
    noteGatewayFailure("rejected");
    await settle();
    expect(posthog.capture).not.toHaveBeenCalled();
  });
});

describe("track: the event-prop allowlist", () => {
  // The consent gate is the subject of the suite above, not this one: these
  // tests are about what a *running* client is allowed to send, so they grant
  // consent and wait for the client to exist before asserting on it.
  async function running() {
    consentIs(true);
    const mod = await load();
    await mod.initAnalytics();
    await settle();
    vi.mocked(posthog.capture).mockClear();
    return mod;
  }

  it("drops any prop key not on the allowlist", async () => {
    const { track } = await running();
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

  it("never sends the dropped values in any form", async () => {
    const { track } = await running();
    track("proxy_enabled", { source: "toggle", data_dir: "/Users/someone/Library" });
    expect(everythingSent()).not.toContain("/Users/someone/Library");
  });

  it("allows the funnel's own props, and the two it used to drop", async () => {
    const { ALLOWED_PROP_KEYS } = await running();
    for (const key of [
      "auth_mode",
      "org_count",
      "surface",
      "reason",
      "detail",
      "install_id",
      "restarted",
      "inline",
    ]) {
      expect(ALLOWED_PROP_KEYS.has(key), key).toBe(true);
    }
    for (const key of ["email", "org_name", "sub", "name", "message", "path"]) {
      expect(ALLOWED_PROP_KEYS.has(key), key).toBe(false);
    }
  });
});

describe("trackError: raw error strings stay on this machine", () => {
  async function running() {
    consentIs(true);
    const mod = await load();
    await mod.initAnalytics();
    await settle();
    vi.mocked(posthog.capture).mockClear();
    return mod;
  }

  // A raw Tauri error chain of the shape that motivated the classification:
  // it names a host, which must never leave the machine.
  const RAW = "connection refused by https://gateway.internal.example:8443";

  it("sends the classified title and context, not the raw string", async () => {
    const { trackError } = await running();
    trackError(RAW, "proxy_toggle");
    const { title } = classifyError(RAW, "proxy_toggle");
    expect(posthog.capture).toHaveBeenCalledWith("error_shown", {
      context: "proxy_toggle",
      title,
    });
    expect(everythingSent()).not.toContain("gateway.internal.example");
  });

  it("files the exception under the classified title too", async () => {
    const { trackError } = await running();
    trackError(new Error(RAW), "connect");
    const { title } = classifyError(new Error(RAW), "connect");
    const [err] = vi.mocked(posthog.captureException).mock.calls[0];
    expect(err).toBeInstanceOf(Error);
    expect((err as Error).message).toBe(title);
    expect(everythingSent()).not.toContain("gateway.internal.example");
  });

  it("runs extra caller props through the same allowlist", async () => {
    const { trackError } = await running();
    trackError(RAW, "provider_toggle", {
      domain: "anthropic",
      upstream_url: "https://gateway.internal.example",
    });
    const [, props] = vi.mocked(posthog.capture).mock.calls[0];
    expect(props).toHaveProperty("domain", "anthropic");
    expect(props).not.toHaveProperty("upstream_url");
  });
});

describe("the tray's session", () => {
  /** The tray groups what it sends but leaves pairing to the window that owns
   *  sign-in, so the claim cannot be won without the org count. */
  it("groups without reporting pairing_completed", async () => {
    consentIs(true);
    const { initAnalytics, noteSession } = await load();
    await initAnalytics();

    noteSession(
      { signedIn: true, authMode: "oauth", sub: null, orgId: ORG },
      { reportPairing: false },
    );
    await settle();

    expect(posthog.group).toHaveBeenCalledWith("organization", ORG);
    expect(posthog.identify).not.toHaveBeenCalled();
    expect(sent("pairing_completed")).toEqual([]);
    expect(claimed.has("pairing_completed")).toBe(false);
  });
});
