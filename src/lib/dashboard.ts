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

/**
 * The `client_tool` names a section's desktop-app and website traffic is stored
 * under, for the "View activity" link on a pane with no installed tool behind
 * it (Claude without Claude Code, ChatGPT without Codex).
 *
 * These are what `client_tool` in `crates/core/src/proxy/mod.rs` stamps from the
 * vendor's own headers: `anthropic-client-platform` for Claude's app and
 * claude.ai, `originator` / `oai-*` on chatgpt.com. They are not `ToolId`s, so
 * the app's own feed cannot read them back, but the dashboard filters on them
 * like any other. A section with no entry here (OpenAI API, whose traffic names
 * no app) gets no link: nothing it routes carries a name to filter on.
 */
export const SECTION_SURFACE_CLIENTS: Readonly<Record<string, readonly string[]>> = {
  claude: ["claude-desktop", "claude-web"],
  chatgpt: ["chatgpt", "chatgpt-web"],
};

/**
 * The `client_tool` names an app pane's "View activity" filters Messages by, or
 * `null` when the pane gets no button.
 *
 * A pane with an installed tool links to that tool, which is the slug its own
 * feed is read with. A pane without one links to its section's
 * {@link SECTION_SURFACE_CLIENTS}, and a section with no entry there has no
 * name to filter on. None at all while the gateway does not know this machine:
 * the link is scoped to it, and an unscoped list would be the whole org's.
 */
export function viewActivityApps({
  machineKnown,
  section,
  tool,
}: {
  machineKnown: boolean;
  /** The open pane's section id. */
  section: string;
  /** The section's installed config tool, or null. */
  tool: string | null;
}): readonly string[] | null {
  if (!machineKnown) return null;
  if (tool !== null) return [tool];
  return SECTION_SURFACE_CLIENTS[section] ?? null;
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
  /**
   * The Messages list, filtered to what a Recent activity card draws, for its
   * "View activity" button.
   *
   * `apps` are `client_tool` slugs: what this app stamps on `x-gate-client`
   * for every request it routes, and the values the card's own feed sends as
   * `tool` to `/v1/me/tool-events`. `device` is the install id the feed is
   * scoped by, and `timeRange=24h` is the feed's own window.
   */
  messages: (filter: { apps: readonly string[]; device?: string | null }) => string;
  /** The Security list for one installation, for the Security events card's
   *  "View activity" button. The feed behind that card is scoped by the same id. */
  security: (filter: { device?: string | null }) => string;
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
    messages: ({ apps, device }) => {
      const params = new URLSearchParams();
      if (apps.length > 0) params.set("app", apps.join(","));
      if (device) params.set("device", device);
      params.set("timeRange", "24h");
      return `${origin}/messages?${params}`;
    },
    security: ({ device }) =>
      device ? `${origin}/security?${new URLSearchParams({ device })}` : `${origin}/security`,
  };
}
