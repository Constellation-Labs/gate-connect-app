/**
 * Where the Gate dashboard lives for the gateway this install is pointed at.
 *
 * **Why this is derived rather than a constant.** The dashboard used to be four
 * hardcoded `app.constellationgate.ai` URLs in `config.ts` while the *gateway*
 * was switchable twice over - at build time through
 * `VITE_GATE_DEFAULT_BASE_URL`, and at runtime through Settings -> Dev mode
 * (`GATEWAY_SERVERS`). So a developer or tester pointed at staging got
 * production dashboard links from every call site, and `pnpm app:local`
 * defaults to staging, which made that the normal dev state rather than an edge
 * case. Adding a second build-time flag would not have fixed it: a flag is
 * still wrong the moment somebody switches server in Dev mode, which is the
 * same shape of bug one level along.
 *
 * The source of truth is therefore `Account.gateway_base_url` - what the
 * backend actually talks to - and every dashboard link is computed from it.
 *
 * **An unrecognised gateway yields `null`, and callers must handle that.**
 * A local gateway (`http://localhost:3000`) has no dashboard at all, and a host
 * this function does not recognise might have one anywhere. Guessing would put
 * a user's traffic on one environment and their key management on another, with
 * nothing on screen saying so - the same failure CLAUDE.md records for a
 * dismissed keychain prompt silently moving a staging developer back to
 * production. AG-598 asks a dashboard action to open "when Gate Connect has the
 * required identifiers"; `null` is that condition being unmet.
 */

import type { ClassifiedError } from "./errors";

/** The active gateway has no dashboard, so a dashboard action has nowhere to go.
 *
 * Reachable on a local dev gateway, and before the first account read lands.
 * Lives here rather than in either shell because both the window and the tray
 * dispatch dashboard actions and must not describe this differently.
 *
 * It names the gateway's absence rather than blaming the click: the click was
 * reasonable, and the configuration is the thing that explains it. */
export const NO_DASHBOARD: ClassifiedError = {
  title: "This gateway has no dashboard",
  hint: "Gate Connect is pointed at a gateway with no dashboard to open. Switch to a Gate server in Settings to reach it.",
  raw: "no dashboard origin for the active gateway",
};

/** Host suffix the dashboard is derived for. Anything else is not ours to map. */
const GATE_SUFFIX = ".constellationgate.ai";

/**
 * The dashboard origin for a gateway base URL, or `null` when there is not one.
 *
 * The mapping is the first host label: `gateway` -> `app`, keeping whatever
 * environment suffix follows it, so `gateway-staging` -> `app-staging` and a
 * future `gateway-dev` -> `app-dev` without another edit here.
 *
 * Everything else returns `null` on purpose - a plain `localhost` gateway, an
 * `http://` origin, a host outside `constellationgate.ai`, and a
 * `constellationgate.ai` host whose first label is not `gateway`. The last one
 * is the conservative case: it could be a dashboard host already, or something
 * with no dashboard, and this cannot tell.
 */
export function dashboardOrigin(gatewayBaseUrl: string | null | undefined): string | null {
  if (!gatewayBaseUrl) return null;
  let url: URL;
  try {
    url = new URL(gatewayBaseUrl);
  } catch {
    return null;
  }
  // The opener ACL only permits https, so an http gateway could not open a
  // derived link even if one existed.
  if (url.protocol !== "https:") return null;
  if (!url.hostname.endsWith(GATE_SUFFIX)) return null;

  const [first, ...rest] = url.hostname.slice(0, -GATE_SUFFIX.length).split(".");
  // Only a bare environment label maps; a deeper subdomain is not something
  // this rule was written for.
  if (rest.length > 0) return null;
  if (first !== "gateway" && !first.startsWith("gateway-")) return null;

  return `https://app${first.slice("gateway".length)}${GATE_SUFFIX}`;
}

/** Every dashboard destination the app links to, for one gateway. */
export interface DashboardLinks {
  /** The dashboard itself. */
  root: string;
  /** API key management, where setup sends a user to fetch a key. */
  apiKeys: string;
  /** Policy management (AG-572's Overview "Manage" link). */
  policies: string;
  /** Token-savings settings (AG-572's second "Manage" link). */
  savings: string;
  /**
   * Contact support (AG-598). The dashboard's Overview page, because that is
   * where the support floating action button lives - there is no dedicated
   * support route. Settled 2026-09-07; it replaced a
   * `constellationnetwork.io/support` address that 404'd.
   */
  support: string;
  /** One request's detail in the security feed, by request id. */
  message: (requestId: string) => string;
}

/**
 * Build every dashboard link for a gateway, or `null` when it has no dashboard.
 *
 * **Trailing paths are load-bearing, not cosmetic.** `openUrl` is gated by the
 * opener ACL in `src-tauri/capabilities/default.json`, whose pattern
 * `https://*.constellationgate.ai/*` is matched with `glob::Pattern` against
 * the raw string passed in. A bare origin has no `/` for the pattern's literal
 * separator and is rejected, which is a link that silently does nothing. So
 * `root` keeps its trailing slash and every other entry carries a path
 * segment. `config.test.ts` asserts this for every gateway in
 * `GATEWAY_SERVERS`.
 */
export function dashboardLinks(gatewayBaseUrl: string | null | undefined): DashboardLinks | null {
  const origin = dashboardOrigin(gatewayBaseUrl);
  if (origin === null) return null;
  return {
    root: `${origin}/`,
    apiKeys: `${origin}/api-keys`,
    policies: `${origin}/policies`,
    savings: `${origin}/token-savings`,
    support: `${origin}/overview`,
    message: (requestId: string) => `${origin}/messages/${encodeURIComponent(requestId)}`,
  };
}
