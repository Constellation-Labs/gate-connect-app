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
/** Product documentation. Trailing slash for the same opener-allowlist reason
 *  as the dashboard link above; `docs.constellationgate.ai` matches the
 *  `https://*.constellationgate.ai/*` capability pattern, so the plumbing works.
 *
 *  Why it exists: an app that installs a root certificate, runs a local MITM
 *  proxy and writes to the OS secret store shipped with no route to
 *  documentation at all, and a Help section whose two items were "Replay tour"
 *  and "Dev mode". */
export const GATE_DOCS_URL = "https://docs.constellationgate.ai/";
