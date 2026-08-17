/**
 * Build-time configuration baked into the bundle by Vite.
 *
 * Override per build to target a different environment:
 *   VITE_GATE_DEFAULT_BASE_URL=https://gateway-staging.constellationgate.ai pnpm build
 */

const RAW = import.meta.env.VITE_GATE_DEFAULT_BASE_URL as string | undefined;

export const DEFAULT_GATEWAY_BASE_URL =
  RAW && RAW.trim().length > 0 ? RAW.trim() : "https://gateway.constellationgate.ai";

/**
 * PostHog project API key, injected at build time. Absent ⇒ analytics is a
 * silent no-op (dev builds stay quiet). Set per build to enable:
 *   VITE_POSTHOG_KEY=phc_… pnpm build
 */
const POSTHOG_KEY = import.meta.env.VITE_POSTHOG_KEY as string | undefined;

export const POSTHOG_KEY_VALUE = POSTHOG_KEY?.trim() || "";

/** US Cloud ingestion host (see tauri.conf.json connect-src allowlist). */
export const POSTHOG_HOST = "https://us.i.posthog.com";

/** Gateway servers selectable from Settings → Dev mode. */
export interface GatewayServer {
  label: string;
  url: string;
}

export const GATEWAY_SERVERS: GatewayServer[] = [
  { label: "Production", url: "https://gateway.constellationgate.ai" },
  { label: "Staging", url: "https://gateway-staging.constellationgate.ai" },
  // TEMPORARY (AG-572): a gateway running on this machine
  // (`pnpm --filter @gate/gateway-proxy dev` serves plain HTTP on :3000).
  //
  // `import.meta.env.DEV` is false in every `vite build`, so this entry cannot
  // reach a shipped bundle - the same belt-and-braces as the `debug_assertions`
  // guard on the http://localhost exception in `account.rs`. Without both, there
  // is no way to point the app at a local gateway: FirstRun does not ask for a
  // URL, it uses DEFAULT_GATEWAY_BASE_URL and offers only this list afterwards.
  ...(import.meta.env.DEV
    ? [{ label: "Local (dev)", url: "http://localhost:3000" }]
    : []),
];

/** The Gate dashboard.
 *
 * The trailing slash is load-bearing. `openUrl` is gated by the opener ACL in
 * `src-tauri/capabilities/default.json`, whose pattern is
 * `https://*.constellationgate.ai/*`, and that is matched with `glob::Pattern`
 * against the raw string we pass. A bare origin has no `/` for the pattern's
 * literal separator, so `https://app.constellationgate.ai` is rejected and the
 * link silently does nothing. Verified: the bare form matches `false`, both
 * slashed forms match `true`.
 *
 * Exported as constants so the three call sites cannot drift apart, which is
 * how two of them ended up with the unslashed form. */
export const GATE_DASHBOARD_URL = "https://app.constellationgate.ai/";
export const GATE_API_KEYS_URL = "https://app.constellationgate.ai/api-keys";

/** Where Overview's two "Manage" links go (AG-572).
 *
 * Same trailing-slash discipline as above: these carry a path segment, so the
 * ACL's literal separator is satisfied and `glob::Pattern` matches.
 *
 * Not org-scoped in the URL. AG-572 asks these to open settings "for the
 * selected organization", but the dashboard resolves the active org from its own
 * session, and a client-supplied org id in the path would either be ignored or
 * disagree with what the user is signed into there. If the dashboard ever grows
 * an explicit `?org=` selector these become builders. */
export const GATE_POLICIES_URL = "https://app.constellationgate.ai/policies";
export const GATE_SAVINGS_URL = "https://app.constellationgate.ai/token-savings";
/** Product documentation. Trailing slash for the same opener-allowlist reason
 *  as the dashboard link above; `docs.constellationgate.ai` matches the
 *  `https://*.constellationgate.ai/*` capability pattern, so the plumbing works.
 *
 *  Why it exists: an app that installs a root certificate, runs a local MITM
 *  proxy and writes to the OS secret store shipped with no route to
 *  documentation at all, and a Help section whose two items were "Replay tour"
 *  and "Dev mode". */
export const GATE_DOCS_URL = "https://docs.constellationgate.ai/";

/** Where "Contact support" in the topnav menu goes.
 *
 *  The only outbound link on `constellationnetwork.io` rather than
 *  `constellationgate.ai`, so it needed its own entry in the opener ACL in
 *  `src-tauri/capabilities/default.json`. Without it the menu item looks wired
 *  and does nothing, which is the state it shipped in until now. */
export const GATE_SUPPORT_URL = "https://constellationnetwork.io/support";
