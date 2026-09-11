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
  /** The console that reads the same database as this gateway.
   *
   *  Carried on the server entry rather than derived by string surgery, and
   *  rather than living as its own constant, because the two facts have to
   *  move together: staging and production are separate stacks with separate
   *  databases, so a console paired with the wrong gateway shows a permanently
   *  empty dashboard and says nothing about why.
   *
   *  The trailing slash is load-bearing. `openUrl` is gated by the opener ACL
   *  in `src-tauri/capabilities/default.json`, whose pattern is
   *  `https://*.constellationgate.ai/*`, matched with `glob::Pattern` against
   *  the raw string we pass. A bare origin has no `/` for the pattern's
   *  literal separator, so `https://app.constellationgate.ai` is rejected and
   *  the link silently does nothing. Verified: the bare form matches `false`,
   *  both slashed forms match `true`. `consoleUrlFor` appends paths straight
   *  onto this, so the slash has to live here. */
  consoleUrl: string;
}

export const GATEWAY_SERVERS: GatewayServer[] = [
  {
    label: "Production",
    url: "https://gateway.constellationgate.ai",
    consoleUrl: "https://app.constellationgate.ai/",
  },
  {
    label: "Staging",
    url: "https://gateway-staging.constellationgate.ai",
    consoleUrl: "https://app-staging.constellationgate.ai/",
  },
];

/** The console for a gateway, plus an optional path under it.
 *
 *  Why this is a function of the account and not a constant: the app used to
 *  hardcode the production console on every link it offers, so an app switched
 *  to staging in Dev mode sent the user to a dashboard reading a different
 *  database. The user then sat in front of an empty Activity page with routing
 *  on and traffic flowing, and nothing on either surface named the mismatch.
 *
 *  An unrecognised gateway falls back to production. Nothing in the UI can
 *  produce one - both first-run and Settings pick from GATEWAY_SERVERS - so
 *  this is the "account file was hand-edited" case, and the old behaviour is
 *  the least surprising answer to it. */
export function consoleUrlFor(gatewayBaseUrl: string | null | undefined, path = ""): string {
  const normalized = (gatewayBaseUrl ?? "").trim().replace(/\/+$/, "");
  const server = GATEWAY_SERVERS.find((s) => s.url === normalized) ?? GATEWAY_SERVERS[0];
  return `${server.consoleUrl}${path}`;
}

/** Product documentation. Trailing slash for the same opener-allowlist reason
 *  as the console links above; `docs.constellationgate.ai` matches the
 *  `https://*.constellationgate.ai/*` capability pattern, so the plumbing works.
 *
 *  Why it exists: an app that installs a root certificate, runs a local MITM
 *  proxy and writes to the OS secret store shipped with no route to
 *  documentation at all, and a Help section whose two items were "Replay tour"
 *  and "Dev mode". */
export const GATE_DOCS_URL = "https://docs.constellationgate.ai/";
