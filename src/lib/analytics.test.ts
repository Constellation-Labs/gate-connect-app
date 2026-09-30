import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { Mock } from "vitest";

/**
 * The analytics seam's promises, pinned.
 *
 * Consent: the preference gates the client; base events (those that predate
 * AG-960) follow "Share diagnostic data", and everything AG-960 added - the
 * identity, the org group, the milestones, `connection_failed` - is held until
 * the diagnostics question has been ANSWERED, released on a yes and spent on a
 * no.
 *
 * Content: props pass an allowlist, errors carry classified titles, failures a
 * reason from a closed list.
 *
 * Identity: the install id from the first event; a stored identified `sub`
 * bootstraps without an `$identify`; the install id is merged into a person at
 * most once; another account or a sign-out resets rather than merges; every
 * window follows the same identity and the same consent.
 *
 * The fakes below keep the two pieces of state that outlive a launch - the
 * posthog-js opt-out it persists in its own storage, and the Rust records (the
 * milestone markers, the analytics identity) - across `load()`, which is what a
 * relaunch or a second window looks like to this module. Without that the
 * relaunch tests would pass against a client that forgot everything.
 */
vi.mock("./config", async (importOriginal) => ({
  ...(await importOriginal<typeof import("./config")>()),
  POSTHOG_KEY_VALUE: "phc_test",
  POSTHOG_HOST: "https://example.invalid",
}));

/** posthog-js as far as these tests need it, with its persisted opt-out. */
const ph = vi.hoisted(() => {
  const state = { optedOut: false, distinctId: "anon-1", identified: false };
  const delivered: { event: string; props: unknown; options: unknown; distinctId: string }[] =
    [];
  return { state, delivered };
});

vi.mock("posthog-js", () => ({
  default: {
    init: vi.fn((_key: string, config: { bootstrap?: { distinctID: string; isIdentifiedID?: boolean } }) => {
      if (config.bootstrap) {
        ph.state.distinctId = config.bootstrap.distinctID;
        ph.state.identified = config.bootstrap.isIdentifiedID === true;
      }
    }),
    register: vi.fn((props: Record<string, unknown>) => {
      if (typeof props.distinct_id === "string") ph.state.distinctId = props.distinct_id;
    }),
    // Like the real client: a capture while opted out is silently dropped.
    capture: vi.fn((event: string, props: unknown, options: unknown) => {
      if (ph.state.optedOut) return;
      ph.delivered.push({ event, props, options, distinctId: ph.state.distinctId });
    }),
    captureException: vi.fn(),
    identify: vi.fn((id: string) => {
      ph.state.distinctId = id;
      ph.state.identified = true;
    }),
    // Like the real one: a new random id, and `consent.reset()` removes the
    // persisted opt-out, so the client is capturing again afterwards.
    reset: vi.fn(() => {
      ph.state.distinctId = "fresh-random";
      ph.state.identified = false;
      ph.state.optedOut = false;
    }),
    group: vi.fn(),
    opt_in_capturing: vi.fn(() => {
      ph.state.optedOut = false;
    }),
    opt_out_capturing: vi.fn(() => {
      ph.state.optedOut = true;
    }),
    has_opted_out_capturing: vi.fn(() => ph.state.optedOut),
    is_capturing: vi.fn(() => !ph.state.optedOut),
    get_distinct_id: vi.fn(() => ph.state.distinctId),
  },
}));

/** Tauri events: every loaded module (window) subscribes; `broadcast` is the
 *  backend's `emit`. */
const bus = vi.hoisted(() => ({
  handlers: new Map<string, Array<(e: { payload: unknown }) => void>>(),
}));
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(async (event: string, handler: (e: { payload: unknown }) => void) => {
    const list = bus.handlers.get(event) ?? [];
    list.push(handler);
    bus.handlers.set(event, list);
    return () => {};
  }),
}));
/** The backend's `emit`. Only ever called with events `src-tauri/src/lib.rs`
 *  really emits, under the names and payload shapes
 *  `analytics.contract.test.ts` pins against its source. */
function broadcast(event: string, payload: unknown) {
  for (const h of bus.handlers.get(event) ?? []) h({ payload });
}

/** What `oauth_sign_out` / `clear_account` do: forget the identity in the
 *  core (keeping `ever_identified`), then `forget_analytics_identity` emits
 *  what is stored. */
function backendSignsOut() {
  rust.identity = {
    identified_sub: null,
    ever_identified: rust.identity.ever_identified,
    org_id: null,
    auth_mode: null,
  };
  broadcast("analytics-identity-changed", { ...rust.identity });
}

/** The Rust records: markers won once, and the stored identity with its sticky
 *  first-identification bit. `set_analytics_identity` broadcasts, as the real
 *  command does. */
const rust = vi.hoisted(() => ({
  claimed: new Set<string>(),
  identity: { identified_sub: null, ever_identified: false, org_id: null, auth_mode: null } as {
    identified_sub: string | null;
    ever_identified: boolean;
    org_id: string | null;
    auth_mode: string | null;
  },
}));

vi.mock("./api", () => ({
  getPreferences: vi.fn(),
  installId: vi.fn(),
  analyticsMilestoneClaim: vi.fn(),
  coworkSettingCheck: vi.fn(),
  analyticsIdentity: vi.fn(async () => ({ ...rust.identity })),
  setAnalyticsIdentity: vi.fn(async (next: typeof rust.identity) => {
    rust.identity = {
      ...next,
      ever_identified: rust.identity.ever_identified || next.ever_identified || !!next.identified_sub,
    };
    broadcast("analytics-identity-changed", { ...rust.identity });
  }),
}));
vi.mock("@tauri-apps/api/app", () => ({ getVersion: vi.fn(async () => "1.2.3") }));
vi.mock("./platform", () => ({ fetchPlatform: vi.fn(async () => "linux") }));

import posthog from "posthog-js";
import { analyticsMilestoneClaim, coworkSettingCheck, getPreferences, installId } from "./api";
import { classifyError, type ConnectionFailureReason, type ErrorContext } from "./errors";

const INSTALL_ID = "3f0c9a52-7d1e-4b8a-9c2f-1a2b3c4d5e6f";
const SUB = "0b8c1f2e-1111-4222-8333-944455556666";
const SUB_B = "7a7a7a7a-2222-4333-8444-955566667777";
const ORG = "9d7e6f5a-aaaa-4bbb-8ccc-0123456789ab";

/** A window: a fresh copy of the module over the same persisted state. */
async function load() {
  vi.resetModules();
  return import("./analytics");
}

/** The two preference fields that decide everything here. The fresh-install
 *  default is `(true, false)`: sharing on, question not answered. */
function prefsAre(share: boolean, recorded = true) {
  (getPreferences as Mock).mockResolvedValue({
    share_diagnostics: share,
    share_diagnostics_recorded: recorded,
    notifications: true,
  });
}

/** Let pending promise chains (IPC mocks, the boot, a claim) run out. */
async function settle() {
  for (let i = 0; i < 10; i++) await Promise.resolve();
  await new Promise((r) => setTimeout(r, 0));
}

/** What actually left: captures the client delivered, props only, by name. */
function sent(event: string): unknown[] {
  return ph.delivered.filter((d) => d.event === event).map((d) => d.props);
}

const fetchMock = vi.fn(async () => new Response("{}"));

/** Bodies POSTed straight to the capture endpoint (the opt-out record). */
function posted(): { event: string; distinct_id: string; properties: Record<string, unknown> }[] {
  return fetchMock.mock.calls.map((c) => JSON.parse((c as unknown as [string, RequestInit])[1].body as string));
}

function everythingSent(): string {
  return JSON.stringify([
    ...ph.delivered,
    ...vi.mocked(posthog.captureException).mock.calls.map((args) =>
      args.map((a) => (a instanceof Error ? `${a.name}: ${a.message}` : a)),
    ),
    ...posted(),
  ]);
}

beforeEach(() => {
  vi.clearAllMocks();
  ph.state.optedOut = false;
  ph.state.distinctId = "anon-1";
  ph.state.identified = false;
  ph.delivered.length = 0;
  bus.handlers.clear();
  rust.claimed.clear();
  rust.identity = { identified_sub: null, ever_identified: false, org_id: null, auth_mode: null };
  (installId as Mock).mockResolvedValue(INSTALL_ID);
  (analyticsMilestoneClaim as Mock).mockImplementation(async (name: string) => {
    if (rust.claimed.has(name)) return false;
    rust.claimed.add(name);
    return true;
  });
  (coworkSettingCheck as Mock).mockResolvedValue(null);
  vi.stubGlobal("fetch", fetchMock);
});
afterEach(() => {
  vi.unstubAllGlobals();
  vi.useRealTimers();
});

const PAIRED_OAUTH = { signedIn: true, authMode: "oauth" as const, sub: SUB, orgId: ORG };

describe("initAnalytics consent", () => {
  it("starts the client when the user has not opted out", async () => {
    prefsAre(true);
    const { initAnalytics } = await load();
    await initAnalytics();
    expect(posthog.init).toHaveBeenCalledTimes(1);
  });

  it("never constructs the client when the user has opted out", async () => {
    prefsAre(false);
    const { initAnalytics } = await load();
    await initAnalytics();
    expect(posthog.init).not.toHaveBeenCalled();
  });

  it("does not collect when consent could not be read", async () => {
    (getPreferences as Mock).mockRejectedValue(new Error("ipc unavailable"));
    const { initAnalytics } = await load();
    await initAnalytics();
    expect(posthog.init).not.toHaveBeenCalled();
  });

  it("drops an event tracked during the boot when the answer is no", async () => {
    prefsAre(false);
    const { initAnalytics, track } = await load();
    const boot = initAnalytics();
    track("app_launched");
    await boot;
    await settle();
    expect(ph.delivered).toEqual([]);
  });
});

describe("item 1: nothing AG-960 added leaves before the question is answered", () => {
  /** The real fresh-install default, and the order the new UI asks in: sign-in,
   *  org, device, confirmation, and only then the diagnostics step. */
  it("holds identity, group and every milestone while the answer is missing", async () => {
    prefsAre(true, false);
    const { initAnalytics, noteSession, noteToolConnected, track, trackError } = await load();
    await initAnalytics();

    track("app_launched", { has_account: false });
    noteSession(PAIRED_OAUTH);
    noteToolConnected("codex", "config");
    trackError("address in use", "connect", { tool: "codex" });
    await settle();

    // The base event keeps its pre-AG-960 rule.
    expect(sent("app_launched")).toEqual([{ has_account: false }]);
    expect(posthog.identify).not.toHaveBeenCalled();
    expect(posthog.group).not.toHaveBeenCalled();
    for (const e of ["app_first_launched", "pairing_completed", "tool_connected", "connection_failed"]) {
      expect(sent(e), e).toEqual([]);
    }
    // Holding is not spending: no marker is claimed yet.
    expect(analyticsMilestoneClaim).not.toHaveBeenCalled();
  });

  it("releases what it held, in order and with identity first, on a yes", async () => {
    prefsAre(true, false);
    const { initAnalytics, noteSession, noteToolConnected, setAnalyticsConsent, trackError } =
      await load();
    await initAnalytics();
    noteSession(PAIRED_OAUTH);
    noteToolConnected("codex", "config");
    trackError("address in use", "connect", { tool: "codex" });
    await settle();

    await setAnalyticsConsent(true, "onboarding");
    await settle();

    expect(posthog.identify).toHaveBeenCalledWith(SUB);
    expect(posthog.group).toHaveBeenCalledWith("organization", ORG);
    expect(sent("app_first_launched")).toHaveLength(1);
    expect(sent("pairing_completed")).toEqual([{ auth_mode: "oauth" }]);
    expect(sent("tool_connected")).toEqual([{ tool: "codex", surface: "config" }]);
    expect(sent("connection_failed")).toEqual([
      { reason: "port_in_use", context: "connect", tool: "codex" },
    ]);
    // Every released event went out under the account, grouped.
    const FUNNEL = ["app_first_launched", "pairing_completed", "tool_connected", "connection_failed"];
    for (const d of ph.delivered.filter((d) => FUNNEL.includes(d.event))) {
      expect(d.distinctId, d.event).toBe(SUB);
    }
    const identifyAt = vi.mocked(posthog.identify).mock.invocationCallOrder[0];
    const firstReleased = vi
      .mocked(posthog.capture)
      .mock.calls.findIndex(([e]) => e === "app_first_launched");
    expect(identifyAt).toBeLessThan(vi.mocked(posthog.capture).mock.invocationCallOrder[firstReleased]);
    // Stamped when they happened: the first launch sorts before the pairing.
    const stamp = (e: string) =>
      (ph.delivered.find((d) => d.event === e)!.options as { timestamp: Date }).timestamp.getTime();
    expect(stamp("app_first_launched")).toBeLessThanOrEqual(stamp("pairing_completed"));
  });

  it("spends the held milestones unsent, and drops the rest, on a no", async () => {
    prefsAre(true, false);
    const { initAnalytics, noteSession, noteToolConnected, setAnalyticsConsent, trackError } =
      await load();
    await initAnalytics();
    noteSession(PAIRED_OAUTH);
    noteToolConnected("codex", "config");
    trackError("address in use", "connect", { tool: "codex" });
    await settle();

    await setAnalyticsConsent(false, "onboarding_skip");
    await settle();

    expect(posthog.identify).not.toHaveBeenCalled();
    expect(posthog.group).not.toHaveBeenCalled();
    for (const e of ["app_first_launched", "pairing_completed", "tool_connected", "connection_failed"]) {
      expect(sent(e), e).toEqual([]);
    }
    expect([...rust.claimed].sort()).toEqual(
      ["app_first_launched", "diagnostics_opted_out", "pairing_completed", "tool_connected.codex"].sort(),
    );
    // And a later opt-in cannot report them late.
    await setAnalyticsConsent(true, "settings");
    await settle();
    expect(sent("app_first_launched")).toEqual([]);
    expect(sent("pairing_completed")).toEqual([]);
  });

  it("does not bootstrap an identified sub before the question is answered", async () => {
    prefsAre(true, false);
    rust.identity = { identified_sub: null, ever_identified: false, org_id: ORG, auth_mode: "oauth" };
    const { initAnalytics } = await load();
    await initAnalytics();
    expect(vi.mocked(posthog.init).mock.calls[0][1]).toMatchObject({
      bootstrap: { distinctID: INSTALL_ID },
    });
  });
});

describe("item 2: a persisted opt-out does not outlive a new yes", () => {
  it("lifts a stale posthog-js opt-out once sharing is on, so captures are delivered", async () => {
    // Launch one: opted out in Settings. posthog-js persists that.
    prefsAre(true);
    let mod = await load();
    await mod.initAnalytics();
    await mod.setAnalyticsConsent(false, "settings");
    expect(ph.state.optedOut).toBe(true);

    // Launch two: still off, so no client. Then back on in Settings.
    prefsAre(false);
    mod = await load();
    await mod.initAnalytics();
    await mod.setAnalyticsConsent(true, "settings");
    mod.track("app_launched");
    mod.noteToolConnected("codex", "config");
    await settle();

    expect(posthog.opt_in_capturing).toHaveBeenCalled();
    expect(sent("app_launched")).toHaveLength(1);
    expect(sent("tool_connected")).toEqual([{ tool: "codex", surface: "config" }]);
  });

  it("never claims a marker into a client that is not delivering", async () => {
    prefsAre(true);
    ph.state.optedOut = true;
    // A client that refuses to opt back in: the claim must not be spent.
    vi.mocked(posthog.opt_in_capturing).mockImplementationOnce(() => {});
    const { initAnalytics, noteToolConnected } = await load();
    await initAnalytics();
    noteToolConnected("codex", "config");
    await settle();

    expect(rust.claimed.has("tool_connected.codex")).toBe(false);
    expect(rust.claimed.has("app_first_launched")).toBe(false);
  });
});

describe("item 3: one identity per account, merged at most once", () => {
  it("bootstraps a stored identified sub without sending $identify", async () => {
    prefsAre(true);
    rust.identity = { identified_sub: SUB, ever_identified: true, org_id: ORG, auth_mode: "oauth" };
    const { initAnalytics, noteSession } = await load();
    await initAnalytics();
    noteSession(PAIRED_OAUTH);
    await settle();

    expect(vi.mocked(posthog.init).mock.calls[0][1]).toMatchObject({
      bootstrap: { distinctID: SUB, isIdentifiedID: true },
    });
    expect(posthog.identify).not.toHaveBeenCalled();
    expect(posthog.reset).not.toHaveBeenCalled();
  });

  it("merges the install id into the first account only, and records it", async () => {
    prefsAre(true);
    const { initAnalytics, noteSession } = await load();
    await initAnalytics();
    noteSession(PAIRED_OAUTH);
    await settle();

    expect(posthog.reset).not.toHaveBeenCalled();
    expect(posthog.identify).toHaveBeenCalledWith(SUB);
    expect(rust.identity).toMatchObject({ identified_sub: SUB, ever_identified: true, org_id: ORG });
  });

  /** The two-account regression: B must not be merged onto A's install person. */
  it("resets before identifying a second account on the same install", async () => {
    prefsAre(true);
    rust.identity = { identified_sub: SUB, ever_identified: true, org_id: ORG, auth_mode: "oauth" };
    const { initAnalytics, noteSession } = await load();
    await initAnalytics();
    noteSession({ ...PAIRED_OAUTH, sub: SUB_B });
    await settle();

    expect(posthog.reset).toHaveBeenCalledTimes(1);
    expect(posthog.identify).toHaveBeenCalledWith(SUB_B);
    expect(vi.mocked(posthog.reset).mock.invocationCallOrder[0]).toBeLessThan(
      vi.mocked(posthog.identify).mock.invocationCallOrder[0],
    );
    expect(rust.identity.identified_sub).toBe(SUB_B);
  });

  /**
   * M1. The backend's `oauth_sign_out` forgets the identity and emits it
   * (`forget_analytics_identity`, pinned in `src-tauri/src/lib.rs`'s tests and
   * by `analytics.contract.test.ts`). Once identified, the install id belongs
   * to that account's person, so the client must NOT go back to it.
   */
  it("moves to a fresh anonymous id on a sign-out, not back to the install id", async () => {
    prefsAre(true);
    rust.identity = { identified_sub: SUB, ever_identified: true, org_id: ORG, auth_mode: "oauth" };
    const { initAnalytics, track } = await load();
    await initAnalytics();
    vi.mocked(posthog.register).mockClear();

    backendSignsOut();
    track("app_launched");

    expect(posthog.reset).toHaveBeenCalledTimes(1);
    expect(posthog.identify).not.toHaveBeenCalled();
    const last = ph.delivered.at(-1)!.distinctId;
    expect(last).not.toBe(INSTALL_ID);
    expect(last).not.toBe(SUB);
    for (const [props] of vi.mocked(posthog.register).mock.calls) {
      expect(props).not.toHaveProperty("distinct_id");
    }
    // The machine is still visible on each event.
    expect(posthog.register).toHaveBeenCalledWith(expect.objectContaining({ install_id: INSTALL_ID }));
  });

  it("does not bootstrap the install id on the next launch of a signed-out install", async () => {
    prefsAre(true);
    rust.identity = { identified_sub: null, ever_identified: true, org_id: null, auth_mode: null };
    const { initAnalytics } = await load();
    await initAnalytics();
    expect(vi.mocked(posthog.init).mock.calls[0][1]).not.toHaveProperty("bootstrap");
  });

  /** H1, webview half: the sign-in window sees the session end before (or
   *  without) the backend's announcement, and must not write the old account
   *  back to disk. */
  it("treats a session that is no longer signed in as a sign-out, and records it", async () => {
    prefsAre(true);
    const { initAnalytics, noteSession } = await load();
    await initAnalytics();
    noteSession(PAIRED_OAUTH);
    await settle();
    expect(rust.identity.identified_sub).toBe(SUB);

    noteSession({ signedIn: false, authMode: "oauth", sub: null, orgId: ORG });
    await settle();

    expect(rust.identity.identified_sub).toBeNull();
    expect(posthog.reset).toHaveBeenCalledTimes(1);
    expect(ph.state.distinctId).not.toBe(SUB);
  });

  /** H1, backend half: every window follows the backend's sign-out. */
  it("moves every window off the account when the backend signs out", async () => {
    prefsAre(true);
    rust.identity = { identified_sub: SUB, ever_identified: true, org_id: ORG, auth_mode: "oauth" };
    const tray = await load();
    await tray.initAnalytics();
    const main = await load();
    await main.initAnalytics();

    backendSignsOut();
    await settle();

    // Both windows reset, and the stored record stays forgotten: nothing wrote
    // the old sub back.
    expect(posthog.reset).toHaveBeenCalledTimes(2);
    expect(rust.identity.identified_sub).toBeNull();
    main.noteSession({ signedIn: false, authMode: "oauth", sub: null, orgId: ORG });
    await settle();
    expect(rust.identity.identified_sub).toBeNull();
  });

  /** M3. `reset` deletes posthog-js's persisted opt-out in shared storage. */
  it("keeps an opted-out install opted out across a reset", async () => {
    prefsAre(true);
    rust.identity = { identified_sub: SUB, ever_identified: true, org_id: ORG, auth_mode: "oauth" };
    const { initAnalytics, setAnalyticsConsent } = await load();
    await initAnalytics();
    await setAnalyticsConsent(false, "settings");
    expect(ph.state.optedOut).toBe(true);

    backendSignsOut();

    expect(posthog.reset).toHaveBeenCalled();
    expect(ph.state.optedOut).toBe(true);
  });

  it("re-signing in after a sign-out resets rather than merging the install id again", async () => {
    prefsAre(true);
    rust.identity = { identified_sub: null, ever_identified: true, org_id: null, auth_mode: null };
    const { initAnalytics, noteSession } = await load();
    await initAnalytics();
    noteSession(PAIRED_OAUTH);
    await settle();
    expect(vi.mocked(posthog.reset).mock.invocationCallOrder[0]).toBeLessThan(
      vi.mocked(posthog.identify).mock.invocationCallOrder[0],
    );
  });

  /** The tray never reads the OAuth session; it follows the stored identity. */
  it("keeps the tray on the same identity and group as the window", async () => {
    prefsAre(true);
    const tray = await load();
    await tray.initAnalytics();
    const main = await load();
    await main.initAnalytics();

    main.noteSession(PAIRED_OAUTH);
    await settle();
    // Only the tray's own reaction: reset and identify, never merging the install.
    vi.mocked(posthog.capture).mockClear();
    ph.delivered.length = 0;
    tray.noteToolConnected("codex", "config");
    await settle();

    expect(sent("tool_connected")).toEqual([{ tool: "codex", surface: "config" }]);
    expect(ph.delivered[0].distinctId).toBe(SUB);
    expect(posthog.group).toHaveBeenCalledWith("organization", ORG);
    // The fake client is shared, so the distinct id above cannot tell the two
    // windows apart; the calls can. The window identified once (merging the
    // install), and the tray followed on its own with a reset first.
    expect(vi.mocked(posthog.identify).mock.calls).toEqual([[SUB], [SUB]]);
    expect(posthog.reset).toHaveBeenCalledTimes(1);
    expect(vi.mocked(posthog.reset).mock.invocationCallOrder[0]).toBeGreaterThan(
      vi.mocked(posthog.identify).mock.invocationCallOrder[0],
    );
  });
});

describe("item 4: consent follows the user into every window", () => {
  it("starts a window that booted opted out when the answer changes elsewhere", async () => {
    prefsAre(false);
    const tray = await load();
    await tray.initAnalytics();
    tray.noteToolConnected("codex", "config");
    await settle();
    expect(sent("tool_connected")).toEqual([]);

    // `set_share_diagnostics` broadcasts the new answer to every window.
    broadcast("analytics-consent-changed", { share_diagnostics: true, recorded: true });
    await settle();
    tray.noteToolConnected("claude-code", "config");
    await settle();

    expect(sent("tool_connected")).toEqual([{ tool: "claude-code", surface: "config" }]);
  });

  it("stops a live window when the user opts out elsewhere, without recording twice", async () => {
    prefsAre(true);
    const tray = await load();
    await tray.initAnalytics();
    broadcast("analytics-consent-changed", { share_diagnostics: false, recorded: true });
    tray.track("app_launched");
    await settle();

    expect(sent("app_launched")).toEqual([]);
    expect(posted()).toEqual([]);
  });

  it("releases a window's held events when the answer is given elsewhere", async () => {
    prefsAre(true, false);
    const tray = await load();
    await tray.initAnalytics();
    tray.noteToolConnected("codex", "config");
    await settle();
    expect(sent("tool_connected")).toEqual([]);

    broadcast("analytics-consent-changed", { share_diagnostics: true, recorded: true });
    await settle();
    expect(sent("tool_connected")).toEqual([{ tool: "codex", surface: "config" }]);
  });
});

describe("milestones", () => {
  it("sends app_first_launched once across windows and launches", async () => {
    prefsAre(true);
    let mod = await load();
    await mod.initAnalytics();
    await settle();
    mod = await load();
    await mod.initAnalytics();
    await settle();
    expect(sent("app_first_launched")).toHaveLength(1);
  });

  it("sends each milestone once, instantly", async () => {
    prefsAre(true);
    const { initAnalytics, noteToolConnected, noteTrafficObserved } = await load();
    await initAnalytics();
    for (let i = 0; i < 5; i++) {
      noteToolConnected("codex", "config");
      noteTrafficObserved(["codex"]);
    }
    await settle();

    expect(sent("tool_connected")).toEqual([{ tool: "codex", surface: "config" }]);
    expect(sent("first_request_proxied")).toEqual([{ source: "relay", tool: "codex" }]);
    for (const d of ph.delivered.filter((d) => d.event !== "$none")) {
      expect(d.options, d.event).toMatchObject({ send_instantly: true });
    }
  });

  it("files a gateway-attributed first request under its own source", async () => {
    prefsAre(true);
    const { initAnalytics, noteGatewayAttributed, noteTrafficObserved } = await load();
    await initAnalytics();
    noteGatewayAttributed();
    noteTrafficObserved([null]);
    await settle();
    expect(sent("first_request_proxied")).toEqual([{ source: "gateway" }]);
  });

  it("leaves a milestone unclaimed when consent is unknown", async () => {
    (getPreferences as Mock).mockRejectedValue(new Error("ipc unavailable"));
    const { initAnalytics, noteToolConnected } = await load();
    await initAnalytics();
    noteToolConnected("codex", "config");
    await settle();
    expect(analyticsMilestoneClaim).not.toHaveBeenCalled();
  });

  it("sends nothing when the store cannot answer", async () => {
    prefsAre(true);
    (analyticsMilestoneClaim as Mock).mockRejectedValue("data dir unwritable");
    const { initAnalytics, noteToolConnected } = await load();
    await initAnalytics();
    noteToolConnected("codex", "config");
    await settle();
    expect(sent("tool_connected")).toEqual([]);
  });
});

describe("item 11: an API-key install waits for its org", () => {
  const API_KEY_NO_ORG = { signedIn: true, authMode: "api_key" as const, sub: null, orgId: null };

  it("holds a milestone until the org lands, then sends it grouped", async () => {
    prefsAre(true);
    const { initAnalytics, noteSession, noteToolConnected } = await load();
    await initAnalytics();
    noteSession(API_KEY_NO_ORG);
    noteToolConnected("codex", "config");
    await settle();
    expect(sent("tool_connected")).toEqual([]);
    expect(rust.claimed.has("tool_connected.codex")).toBe(false);

    noteSession({ ...API_KEY_NO_ORG, orgId: ORG });
    await settle();
    expect(sent("tool_connected")).toEqual([{ tool: "codex", surface: "config" }]);
    const grouped = vi.mocked(posthog.group).mock.invocationCallOrder[0];
    const at = vi.mocked(posthog.capture).mock.calls.findIndex(([e]) => e === "tool_connected");
    expect(grouped).toBeLessThan(vi.mocked(posthog.capture).mock.invocationCallOrder[at]);
    expect(posthog.identify).not.toHaveBeenCalled();
  });

  it("gives up waiting after a bound and sends it ungrouped", async () => {
    vi.useFakeTimers();
    prefsAre(true);
    const { initAnalytics, noteSession, noteToolConnected, ORG_WAIT_MS } = await load();
    const boot = initAnalytics();
    await vi.advanceTimersByTimeAsync(0);
    await boot;
    noteSession(API_KEY_NO_ORG);
    noteToolConnected("codex", "config");
    await vi.advanceTimersByTimeAsync(ORG_WAIT_MS + 1000);
    expect(sent("tool_connected")).toEqual([{ tool: "codex", surface: "config" }]);
  });
});

describe("item 7 and 10: the opt-out record", () => {
  it("is sent once, under the account without merging the install, with the org", async () => {
    // The onboarding Skip: signed in, question never answered, so the install
    // was never identified - and must not be merged by the record either.
    prefsAre(true, false);
    const { initAnalytics, noteSession, setAnalyticsConsent } = await load();
    await initAnalytics();
    noteSession(PAIRED_OAUTH);
    await setAnalyticsConsent(false, "onboarding_skip");
    await settle();

    expect(posthog.identify).not.toHaveBeenCalled();
    expect(posted()).toEqual([
      {
        api_key: "phc_test",
        event: "diagnostics_opted_out",
        distinct_id: SUB,
        properties: { source: "onboarding_skip", $groups: { organization: ORG } },
      },
    ]);
  });

  it("files an API-key install's record under the install id", async () => {
    prefsAre(true);
    const { initAnalytics, noteSession, setAnalyticsConsent } = await load();
    await initAnalytics();
    noteSession({ signedIn: true, authMode: "api_key", sub: null, orgId: ORG });
    await setAnalyticsConsent(false, "settings");
    await settle();
    expect(posted()[0]).toMatchObject({ distinct_id: INSTALL_ID, properties: { source: "settings" } });
  });

  it("is recorded once per install, not per opt-out", async () => {
    prefsAre(true);
    const { initAnalytics, setAnalyticsConsent } = await load();
    await initAnalytics();
    await setAnalyticsConsent(false, "settings");
    await setAnalyticsConsent(true, "settings");
    await setAnalyticsConsent(false, "settings");
    await settle();
    expect(posted()).toHaveLength(1);
  });

  it("stops capture at once and still sends the record when the claim answers late", async () => {
    prefsAre(true);
    let answer!: (won: boolean) => void;
    (analyticsMilestoneClaim as Mock).mockImplementation((name: string) =>
      name === "diagnostics_opted_out" ? new Promise<boolean>((r) => (answer = r)) : Promise.resolve(false),
    );
    const { initAnalytics, setAnalyticsConsent, track } = await load();
    await initAnalytics();

    const done = setAnalyticsConsent(false, "settings");
    track("app_launched");
    expect(posthog.opt_out_capturing).toHaveBeenCalledTimes(1);
    await done;
    await new Promise((r) => setTimeout(r, 2500));
    answer(true);
    await settle();

    expect(sent("app_launched")).toEqual([]);
    expect(posted()).toHaveLength(1);
  }, 10_000);

  it("records nothing for an install that was never sharing", async () => {
    prefsAre(false);
    const { initAnalytics, setAnalyticsConsent } = await load();
    await initAnalytics();
    await setAnalyticsConsent(false, "settings");
    expect(posted()).toEqual([]);
    expect(posthog.init).not.toHaveBeenCalled();
  });
});

describe("connection_failed: a reason from a closed list", () => {
  async function running() {
    prefsAre(true);
    const mod = await load();
    await mod.initAnalytics();
    await settle();
    ph.delivered.length = 0;
    return mod;
  }

  const CASES: [string, ErrorContext, ConnectionFailureReason][] = [
    ["starting the engine: the relay port 45981 is already in use.", "proxy_toggle", "port_in_use"],
    ["bind 127.0.0.1:8080: Address already in use (os error 48)", "connect", "port_in_use"],
    ["Address already in use (os error 98)", "restore_routing", "port_in_use"],
    [
      "Only one usage of each socket address (protocol/network address/port) is normally permitted. (os error 10048)",
      "connect",
      "port_in_use",
    ],
    ["the proxy is not running; turn routing on first", "connect", "routing_off"],
    ["the certificate trust dialog was cancelled", "trust_ca", "ca_trust_declined"],
    ["execution error: User canceled. (-128)", "trust_ca", "ca_trust_declined"],
    ["execution error: User canceled. (-128)", "proxy_toggle", "prompt_declined"],
    ["error sending request: connection refused", "connect", "offline"],
    ["gateway answered 401 Unauthorized", "provider_toggle", "auth_rejected"],
    ["disk quota exceeded writing /Users/x/.codex/config.toml", "connect", "unknown"],
    // Item 12: numbers inside ports and ids are not a status code or a cancel.
    ["listener 127.0.0.1:40199 closed unexpectedly", "connect", "unknown"],
    ["session 5a401b7c failed to start", "provider_toggle", "unknown"],
    ["request c0a8-128e failed", "trust_ca", "unknown"],
  ];

  for (const [raw, context, reason] of CASES) {
    it(`files "${raw.slice(0, 40)}" in ${context} as ${reason}`, async () => {
      const { trackError } = await running();
      trackError(raw, context, { tool: "codex" });
      await settle();
      expect(sent("connection_failed")).toEqual([{ reason, context, tool: "codex" }]);
      expect(ph.delivered.find((d) => d.event === "connection_failed")!.options).toMatchObject({
        send_instantly: true,
      });
    });
  }

  it("never puts the raw message on the wire", async () => {
    const { trackError } = await running();
    trackError("disk quota exceeded writing /Users/x/.codex/config.toml", "connect");
    await settle();
    expect(everythingSent()).not.toContain("/Users/x");
  });

  it("prefers the backend's typed reason over the message", async () => {
    const { trackError } = await running();
    trackError("starting the relay failed", "restore_routing", undefined, "port_in_use");
    await settle();
    expect(sent("connection_failed")).toEqual([{ reason: "port_in_use", context: "restore_routing" }]);
  });

  it("is not sent for a failure outside a connecting step", async () => {
    const { trackError } = await running();
    trackError("connection refused", "update");
    await settle();
    expect(sent("connection_failed")).toEqual([]);
    expect(sent("error_shown")).toHaveLength(1);
  });

  it("reports the same failure once per window, not on every retry", async () => {
    const { trackError } = await running();
    for (let i = 0; i < 4; i++) trackError("address in use", "connect", { tool: "codex" });
    trackError("address in use", "connect", { tool: "opencode" });
    await settle();
    expect(sent("connection_failed")).toHaveLength(2);
  });

  /** Item 5: no credential to send is a state of the app, not a refusal. */
  it("does not file a signed-out read at all", async () => {
    const { noteGatewayFailure } = await running();
    noteGatewayFailure("signed_out");
    await settle();
    expect(sent("connection_failed")).toEqual([]);
  });

  it("files only a refused or unreachable gateway read, not a signed-out one", async () => {
    const { noteGatewayFailure } = await running();
    noteGatewayFailure("rejected");
    noteGatewayFailure("offline");
    noteGatewayFailure("signed_out");
    noteGatewayFailure("gateway");
    await settle();
    expect(sent("connection_failed")).toEqual([
      { reason: "auth_rejected", context: "gateway" },
      { reason: "offline", context: "gateway" },
    ]);
  });

  /** Item 6: the sign-in and pairing steps. */
  it("files sign-in and pairing failures with typed reasons", async () => {
    const { noteSetupFailure } = await running();
    noteSetupFailure("gateway /v1/me/orgs returned 401 Unauthorized: {}", "org_list");
    noteSetupFailure(JSON.stringify({ code: "rejected", message: "refused" }), "org_select");
    noteSetupFailure(JSON.stringify({ code: "offline", message: "dns" }), "sign_in");
    noteSetupFailure("authorization failed (access_denied)", "sign_in");
    noteSetupFailure("keychain write failed", "org_select");
    await settle();
    expect(sent("connection_failed")).toEqual([
      { reason: "auth_rejected", context: "org_list" },
      { reason: "auth_rejected", context: "org_select" },
      { reason: "offline", context: "sign_in" },
      { reason: "sign_in_not_completed", context: "sign_in" },
      { reason: "unknown", context: "org_select" },
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
  });

  it("does not read Claude's settings for any other connect", async () => {
    const { noteToolConnected } = await running();
    noteToolConnected("codex", "config");
    noteToolConnected("openai", "domain");
    await settle();
    expect(coworkSettingCheck).not.toHaveBeenCalled();
  });

  it("sends nothing at all while opted out", async () => {
    prefsAre(false);
    const { initAnalytics, trackError, noteGatewayFailure } = await load();
    await initAnalytics();
    trackError("address in use", "connect");
    noteGatewayFailure("rejected");
    await settle();
    expect(ph.delivered).toEqual([]);
  });
});

describe("identity at start", () => {
  it("bootstraps the install id as the distinct id", async () => {
    prefsAre(true);
    const { initAnalytics } = await load();
    await initAnalytics();
    expect(vi.mocked(posthog.init).mock.calls[0][1]).toMatchObject({
      bootstrap: { distinctID: INSTALL_ID },
      person_profiles: "identified_only",
      autocapture: false,
    });
  });

  it("holds an event tracked during the boot and sends it once identity is set", async () => {
    let answer!: (v: unknown) => void;
    (getPreferences as Mock).mockReturnValue(new Promise((r) => (answer = r)));
    const { initAnalytics, track } = await load();
    const boot = initAnalytics();
    track("app_launched", { has_account: false });
    expect(ph.delivered).toEqual([]);
    answer({ share_diagnostics: true, share_diagnostics_recorded: true });
    await boot;

    expect(sent("app_launched")).toEqual([{ has_account: false }]);
    expect(ph.delivered[0].distinctId).toBe(INSTALL_ID);
    expect(vi.mocked(posthog.register).mock.invocationCallOrder[0]).toBeLessThan(
      vi.mocked(posthog.capture).mock.invocationCallOrder[0],
    );
    expect(posthog.register).toHaveBeenCalledWith({
      app_version: "1.2.3",
      platform: "linux",
      install_id: INSTALL_ID,
    });
  });
});

describe("track: the event-prop allowlist", () => {
  async function running() {
    prefsAre(true);
    const mod = await load();
    await mod.initAnalytics();
    await settle();
    ph.delivered.length = 0;
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
    expect(sent("tool_toggled")).toEqual([{ tool: "codex", routed: true }]);
  });

  it("allows the funnel's own props, and the two it used to drop", async () => {
    const { ALLOWED_PROP_KEYS } = await running();
    for (const key of ["auth_mode", "org_count", "surface", "reason", "detail", "install_id", "restarted", "inline"]) {
      expect(ALLOWED_PROP_KEYS.has(key), key).toBe(true);
    }
    for (const key of ["email", "org_name", "sub", "name", "message", "path"]) {
      expect(ALLOWED_PROP_KEYS.has(key), key).toBe(false);
    }
  });
});

describe("trackError: raw error strings stay on this machine", () => {
  const RAW = "connection refused by https://gateway.internal.example:8443";

  it("sends the classified title and context, not the raw string", async () => {
    prefsAre(true);
    const { initAnalytics, trackError } = await load();
    await initAnalytics();
    trackError(RAW, "proxy_toggle");
    await settle();
    const { title } = classifyError(RAW, "proxy_toggle");
    expect(sent("error_shown")).toContainEqual({ context: "proxy_toggle", title });
    expect(everythingSent()).not.toContain("gateway.internal.example");
  });
});

describe("the held queue", () => {
  it("holds a repeating signal once, so it cannot crowd out the rest", async () => {
    prefsAre(true, false);
    const { initAnalytics, noteSession, noteTrafficObserved, setAnalyticsConsent } = await load();
    await initAnalytics();
    for (let i = 0; i < 300; i++) noteTrafficObserved(["codex"]);
    noteSession(PAIRED_OAUTH);
    await setAnalyticsConsent(true, "onboarding");
    await settle();
    expect(sent("first_request_proxied")).toHaveLength(1);
    expect(sent("pairing_completed")).toHaveLength(1);
  });

  it("spends a pairing that happens after a no, so an opt-in cannot report it late", async () => {
    prefsAre(false);
    const { initAnalytics, noteSession, setAnalyticsConsent } = await load();
    await initAnalytics();
    noteSession(PAIRED_OAUTH);
    await settle();
    expect(rust.claimed.has("pairing_completed")).toBe(true);
    await setAnalyticsConsent(true, "settings");
    await settle();
    expect(sent("pairing_completed")).toEqual([]);
  });

  it("puts a waiting tray on the account identity once the answer is yes", async () => {
    prefsAre(true, false);
    const tray = await load();
    await tray.initAnalytics();
    const main = await load();
    await main.initAnalytics();
    main.noteSession(PAIRED_OAUTH);
    tray.noteToolConnected("codex", "config");
    await settle();

    await main.setAnalyticsConsent(true, "onboarding");
    broadcast("analytics-consent-changed", { share_diagnostics: true, recorded: true });
    await settle();

    const toolConnected = ph.delivered.find((d) => d.event === "tool_connected");
    expect(toolConnected?.distinctId).toBe(SUB);
    // Both windows moved onto the account: the window by identifying, the tray
    // by following the stored identity.
    expect(vi.mocked(posthog.identify).mock.calls).toEqual([[SUB], [SUB]]);
  });
});
