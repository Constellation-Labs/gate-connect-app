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
  // A gateway running on this machine, for development only (AG-572)
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

/** Every dashboard URL is DERIVED, not constant - see `lib/dashboard.ts`.
 *
 * `GATE_DASHBOARD_URL`, `GATE_API_KEYS_URL`, `GATE_POLICIES_URL` and
 * `GATE_SAVINGS_URL` used to live here, hardcoded to
 * `app.constellationgate.ai`, while the gateway above is switchable at build
 * time AND at runtime through Settings -> Dev mode. So every one of them was
 * wrong for anybody not on production, and `pnpm app:local` defaults to
 * staging. `dashboardLinks(account.gateway_base_url)` replaced them on
 * 2026-09-07; the trailing-slash discipline they documented moved with them and
 * is asserted in `config.test.ts` for every gateway in `GATEWAY_SERVERS`.
 *
 * `GATE_SUPPORT_URL` is gone the same way. It pointed at
 * `constellationnetwork.io/support`, which 404'd, and needed its own opener-ACL
 * entry for being the one outbound link off `constellationgate.ai`. Support is
 * now the dashboard's own Overview page (that is where the support floating
 * action button lives), so it is `dashboardLinks(...).support` and the bespoke
 * ACL entry is deleted - the allowlist got smaller.
 *
 * Docs stay a constant below: documentation is not per-environment. */

/** Product documentation. Trailing slash for the same opener-allowlist reason
 *  the dashboard links carry a path; `docs.constellationgate.ai` matches the
 *  `https://*.constellationgate.ai/*` capability pattern, so the plumbing works.
 *
 *  Why it exists: an app that installs a root certificate, runs a local MITM
 *  proxy and writes to the OS secret store shipped with no route to
 *  documentation at all, and a Help section whose two items were "Replay tour"
 *  and "Dev mode". */
export const GATE_DOCS_URL = "https://docs.constellationgate.ai/";
