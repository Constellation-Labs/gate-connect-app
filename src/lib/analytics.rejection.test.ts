import { beforeEach, describe, expect, it, vi } from "vitest";

/**
 * Round 4 (AG-960): a REJECTED account read must decide nothing.
 *
 * `get_account` rejects when `account.json` or the secret store cannot be read
 * (a keychain error in `has_api_key`). The screens draw that as signed out, and
 * the analytics seam used to take it the same way: an OAuth install's stored
 * sub was cleared, and the install moved off its id, on one keychain hiccup.
 *
 * This drives the real chain the shells use - `readAccount` over the real
 * `api.ts` over a faked `invoke`, then `sessionFacts`, then `noteSession` -
 * with `get_account` actually rejecting. Only the backend is fake, and each
 * command answers the way the Rust command does: `set_analytics_identity`
 * applies `core::analytics::save_identity_in`'s rules (a sticky
 * `ever_identified`, and a sub only for the live session) and emits the stored
 * record as `analytics-identity-changed`, as `announce_analytics_identity`
 * does.
 */
vi.mock("./config", async (importOriginal) => ({
  ...(await importOriginal<typeof import("./config")>()),
  POSTHOG_KEY_VALUE: "phc_test",
  POSTHOG_HOST: "https://example.invalid",
}));

const ph = vi.hoisted(() => ({ distinctId: "anon-1", identified: false }));
vi.mock("posthog-js", () => ({
  default: {
    init: vi.fn((_k: string, c: { bootstrap?: { distinctID: string; isIdentifiedID?: boolean } }) => {
      if (c.bootstrap) {
        ph.distinctId = c.bootstrap.distinctID;
        ph.identified = c.bootstrap.isIdentifiedID === true;
      }
    }),
    register: vi.fn(),
    capture: vi.fn(),
    captureException: vi.fn(),
    identify: vi.fn((id: string) => {
      ph.distinctId = id;
      ph.identified = true;
    }),
    reset: vi.fn(() => {
      ph.distinctId = "fresh-random";
      ph.identified = false;
    }),
    group: vi.fn(),
    opt_in_capturing: vi.fn(),
    opt_out_capturing: vi.fn(),
    has_opted_out_capturing: vi.fn(() => false),
    is_capturing: vi.fn(() => true),
    get_distinct_id: vi.fn(() => ph.distinctId),
    get_property: vi.fn((k: string) =>
      k === "$user_state" ? (ph.identified ? "identified" : "anonymous") : undefined,
    ),
  },
}));

const bus = vi.hoisted(() => ({ handlers: new Map<string, Array<(e: { payload: unknown }) => void>>() }));
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(async (event: string, handler: (e: { payload: unknown }) => void) => {
    bus.handlers.set(event, [...(bus.handlers.get(event) ?? []), handler]);
    return () => {};
  }),
}));
vi.mock("@tauri-apps/api/app", () => ({ getVersion: vi.fn(async () => "1.2.3") }));

type Identity = {
  identified_sub: string | null;
  ever_identified: boolean;
  org_id: string | null;
  auth_mode: string | null;
};

/** The backend: a command router over `invoke`, the only seam faked. */
const backend = vi.hoisted(() => ({
  identity: null as unknown as Identity,
  account: null as unknown,
  accountFails: false,
  oauth: null as unknown,
  saves: [] as Identity[],
}));
vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(async (cmd: string, args?: Record<string, unknown>) => {
    switch (cmd) {
      case "get_account":
        if (backend.accountFails) throw "reading the secret store: Secret Service unavailable";
        return backend.account;
      case "oauth_status":
        return backend.oauth;
      case "get_preferences":
        return { share_diagnostics: true, share_diagnostics_recorded: true };
      case "install_id":
        return INSTALL_ID;
      case "app_platform":
        return "linux";
      case "analytics_identity":
        return { ...backend.identity };
      case "analytics_milestone_claim":
        return false;
      case "set_analytics_identity": {
        // `save_identity_in`'s rules, then the emit of the stored record.
        const next = args!.identity as Identity;
        const live = (backend.oauth as { sub?: string | null } | null)?.sub ?? null;
        const refused = next.identified_sub !== null && next.identified_sub !== live;
        if (!refused) {
          backend.identity = {
            ...next,
            ever_identified:
              backend.identity.ever_identified || next.ever_identified || !!next.identified_sub,
          };
          backend.saves.push({ ...backend.identity });
        }
        for (const h of bus.handlers.get("analytics-identity-changed") ?? []) {
          h({ payload: { ...backend.identity } });
        }
        if (refused) throw "refusing an analytics identity that is not the live session's";
        return null;
      }
      default:
        return null;
    }
  }),
}));

const INSTALL_ID = "3f0c9a52-7d1e-4b8a-9c2f-1a2b3c4d5e6f";
const SUB = "0b8c1f2e-1111-4222-8333-944455556666";
const ORG = "9d7e6f5a-aaaa-4bbb-8ccc-0123456789ab";

const API_KEY_ACCOUNT = {
  gateway_base_url: "https://gateway.example",
  has_api_key: true,
  auth_mode: "api_key",
  billing_mode: "gate",
  org_id: null,
  org_name: null,
};
const OAUTH_ACCOUNT = { ...API_KEY_ACCOUNT, has_api_key: false, auth_mode: "oauth", org_id: ORG, org_name: "Org" };
const LIVE_OAUTH = { signed_in: true, email: null, sub: SUB, session: "live", expires_at_unix: 4102444800 };

async function settle() {
  for (let i = 0; i < 10; i++) await Promise.resolve();
  await new Promise((r) => setTimeout(r, 0));
}

/** One sign-in window: the real modules, loaded fresh over the same backend. */
async function window() {
  vi.resetModules();
  const api = await import("./api");
  const analytics = await import("./analytics");
  const { sessionFacts } = await import("./analyticsSession");
  await analytics.initAnalytics();
  await settle();
  /** What the shells do on every account read: read, then tell the seam. */
  const readAndNote = async (apiKeyOrgId: string | null = null) => {
    const reading = await api.readAccount();
    const oauth = await api.oauthStatus().catch(() => null);
    analytics.noteSession(
      sessionFacts({ account: reading.account, accountUnread: reading.unread, oauth, apiKeyOrgId }),
    );
    await settle();
    return reading;
  };
  return { analytics, readAndNote };
}

beforeEach(async () => {
  vi.clearAllMocks();
  bus.handlers.clear();
  ph.distinctId = "anon-1";
  ph.identified = false;
  backend.accountFails = false;
  backend.saves = [];
  backend.oauth = { signed_in: false, email: null, sub: null, session: "signed_out", expires_at_unix: 0 };
});

describe("a rejected account read decides nothing about identity", () => {
  it("leaves a paired API-key install on its install id and its org", async () => {
    backend.identity = {
      identified_sub: null,
      ever_identified: false,
      org_id: ORG,
      auth_mode: "api_key",
    };
    backend.account = API_KEY_ACCOUNT;
    const { readAndNote } = await window();
    await readAndNote(ORG);
    expect(ph.distinctId).toBe(INSTALL_ID);

    backend.accountFails = true;
    const reading = await readAndNote(null);

    expect(reading.unread).toBe(true);
    expect(backend.identity.ever_identified).toBe(false);
    expect(backend.identity.org_id).toBe(ORG);
    expect(ph.distinctId).toBe(INSTALL_ID);
    const posthog = (await import("posthog-js")).default;
    expect(posthog.reset).not.toHaveBeenCalled();
  });

  it("leaves an OAuth install on its account", async () => {
    backend.identity = {
      identified_sub: SUB,
      ever_identified: true,
      org_id: ORG,
      auth_mode: "oauth",
    };
    backend.account = OAUTH_ACCOUNT;
    backend.oauth = LIVE_OAUTH;
    const { readAndNote } = await window();
    expect(ph.distinctId).toBe(SUB);

    backend.accountFails = true;
    const reading = await readAndNote();

    expect(reading.unread).toBe(true);
    expect(backend.identity.identified_sub).toBe(SUB);
    expect(ph.distinctId).toBe(SUB);
    const posthog = (await import("posthog-js")).default;
    expect(posthog.reset).not.toHaveBeenCalled();
  });

  /** The control: the same chain with a read that RESOLVED to no account is a
   *  real sign-out for OAuth, so the tests above are not passing on a seam
   *  that ignores every read. */
  it("still signs an OAuth install out when the read resolves to no account", async () => {
    backend.identity = {
      identified_sub: SUB,
      ever_identified: true,
      org_id: ORG,
      auth_mode: "oauth",
    };
    backend.account = OAUTH_ACCOUNT;
    backend.oauth = LIVE_OAUTH;
    const { readAndNote } = await window();

    backend.account = null;
    backend.oauth = { signed_in: false, email: null, sub: null, session: "signed_out", expires_at_unix: 0 };
    const reading = await readAndNote();

    expect(reading.unread).toBe(false);
    expect(backend.identity.identified_sub).toBeNull();
    expect(ph.distinctId).not.toBe(SUB);
  });

  /** And an API-key install that reads as signed out keeps its install id:
   *  nothing ties it to a person, so there is nothing to move off. */
  it("keeps an API-key install on its install id when the read resolves to no account", async () => {
    backend.identity = {
      identified_sub: null,
      ever_identified: false,
      org_id: ORG,
      auth_mode: "api_key",
    };
    backend.account = API_KEY_ACCOUNT;
    const { readAndNote } = await window();

    backend.account = null;
    await readAndNote();

    expect(backend.identity.ever_identified).toBe(false);
    expect(ph.distinctId).toBe(INSTALL_ID);
  });
});
