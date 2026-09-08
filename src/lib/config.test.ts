import { describe, expect, it } from "vitest";
// `?raw` rather than node:fs: tsconfig sets types to ["vite/client"] only, and
// widening the app's type surface to all of node just to read a file in a test
// is the wrong trade. vite/client already declares `*?raw` as a string.
import capabilitiesRaw from "../../src-tauri/capabilities/default.json?raw";
import trayCapabilitiesRaw from "../../src-tauri/capabilities/tray.json?raw";
import { GATEWAY_SERVERS, GATE_DOCS_URL } from "./config";
import { dashboardLinks } from "./dashboard";

/** Translate a `glob::Pattern` (what tauri-plugin-opener matches with) into a
 * regex. Only the constructs the capability files use: `*` for a run of
 * characters.
 *
 * Two properties this has to get right, both of which have bitten:
 *
 * - `*` matches zero or more, but a literal `/` in the pattern still has to be
 *   present in the input - which is why `https://app.constellationgate.ai` was
 *   rejected while the slashed form was allowed.
 * - **`*` DOES cross a `/`.** Unlike the path-glob semantics the crate is
 *   usually reached for, `glob::Pattern::new("https://*.constellationgate.ai/*")`
 *   matches `https://app-staging.constellationgate.ai/messages/req_123`.
 *   Verified 2026-09-07 by running the real crate against these exact patterns,
 *   because the answer decides whether every `/messages/:id` link in the app is
 *   permitted - and a `[^/]*` reading says they are not. So `*` is `.*` here. */
function globToRegExp(pattern: string): RegExp {
  const escaped = pattern.replace(/[.+?^${}()|[\]\\]/g, "\\$&").replace(/\*/g, ".*");
  return new RegExp(`^${escaped}$`);
}

function openerAllowPatterns(raw: string): string[] {
  const caps = JSON.parse(raw) as {
    permissions: (string | { identifier: string; allow?: { url?: string }[] })[];
  };
  const entry = caps.permissions.find(
    (p): p is { identifier: string; allow?: { url?: string }[] } =>
      typeof p === "object" && p.identifier === "opener:allow-open-url",
  );
  return (entry?.allow ?? []).map((a) => a.url).filter((u): u is string => !!u);
}

describe("external URLs are permitted by the opener ACL", () => {
  const patterns = openerAllowPatterns(capabilitiesRaw);

  it("finds the opener scope in the capability file", () => {
    // If this fails the rest is vacuous, so assert the fixture is real.
    expect(patterns.length).toBeGreaterThan(0);
  });

  /** Every dashboard destination for one gateway, flattened for the assertion
   *  below. `message` is a builder, so it needs a sample id. */
  function everyLinkFor(gatewayBaseUrl: string): [string, string][] {
    const links = dashboardLinks(gatewayBaseUrl);
    if (links === null) return [];
    return [
      ["dashboard", links.root],
      ["api keys", links.apiKeys],
      ["policies", links.policies],
      ["savings", links.savings],
      ["support", links.support],
      ["message detail", links.message("req_123")],
    ];
  }

  // Every gateway the app can be pointed at, not just production. The dashboard
  // host is derived now (`lib/dashboard.ts`), so a staging or future
  // environment produces URLs this file has never seen - and an ACL that only
  // covered production would turn every one of them into a dead button. The
  // wildcard pattern already covers them; this is what keeps that true.
  const gateways = GATEWAY_SERVERS.map((s) => s.url).filter(
    (url) => dashboardLinks(url) !== null,
  );

  it("has a gateway with a dashboard to check", () => {
    // Guards against the filter above emptying and taking the assertions with
    // it, which would pass silently.
    expect(gateways.length).toBeGreaterThan(0);
  });

  it.each(gateways.flatMap((g) => everyLinkFor(g).map(([label, url]) => [g, label, url])))(
    "%s: allows the %s URL",
    (_gateway, _label, url) => {
      // `openUrl` rejects silently when the ACL blocks it, so a URL that no
      // pattern matches is a button that does nothing.
      const allowed = patterns.some((p) => globToRegExp(p).test(url as string));
      expect(allowed, `${url} matches none of ${JSON.stringify(patterns)}`).toBe(true);
    },
  );

  it("allows the docs URL", () => {
    expect(patterns.some((p) => globToRegExp(p).test(GATE_DOCS_URL))).toBe(true);
  });

  it("no longer carries the bespoke support entry", () => {
    // Support moved onto the dashboard's own Overview page (2026-09-07), so the
    // one-off `constellationnetwork.io/support` permission is not needed. A
    // regression that re-adds it widens what a compromised renderer may open.
    expect(patterns.some((p) => p.includes("constellationnetwork.io"))).toBe(false);
  });

  it("still rejects the unslashed origin, which is why the slash is required", () => {
    // Documents the actual failure rather than trusting the fix: the bare
    // origin has no `/` for the pattern's literal separator.
    const bare = "https://app.constellationgate.ai";
    expect(patterns.some((p) => globToRegExp(p).test(bare))).toBe(false);
  });

  it("does not allow an unrelated host", () => {
    expect(patterns.some((p) => globToRegExp(p).test("https://evil.example.com/"))).toBe(false);
  });
});

/**
 * The tray popover has its OWN capability file, and it is deliberately narrower
 * than `default`'s any-subdomain wildcard - so it has to be checked separately.
 *
 * This is the gap that let a real bug ship to a manual test: `tray.json` allowed
 * the two literal hosts `app.constellationgate.ai` and
 * `docs.constellationgate.ai`, and once dashboard URLs became environment-aware
 * the popover started asking for `app-staging.constellationgate.ai`, which its
 * own scope refused. On screen that is "Not allowed to open url ..." from a menu
 * item that looks perfectly wired. The suite passed throughout, because it only
 * ever parsed `default.json`.
 */
describe("the tray popover's opener ACL covers what its menu opens", () => {
  const patterns = openerAllowPatterns(trayCapabilitiesRaw);

  it("finds the opener scope in the tray capability file", () => {
    expect(patterns.length).toBeGreaterThan(0);
  });

  /** What `TrayMenu` can open: the dashboard root, support, and docs. */
  function trayUrlsFor(gatewayBaseUrl: string): [string, string][] {
    const links = dashboardLinks(gatewayBaseUrl);
    const rows: [string, string][] = [["docs", GATE_DOCS_URL]];
    if (links) {
      rows.push(["dashboard", links.root], ["support", links.support]);
    }
    return rows;
  }

  const gateways = GATEWAY_SERVERS.map((s) => s.url).filter(
    (url) => dashboardLinks(url) !== null,
  );

  it.each(gateways.flatMap((g) => trayUrlsFor(g).map(([label, url]) => [g, label, url])))(
    "%s: allows the %s URL",
    (_gateway, _label, url) => {
      const allowed = patterns.some((p) => globToRegExp(p).test(url as string));
      expect(allowed, `${url} matches none of ${JSON.stringify(patterns)}`).toBe(true);
    },
  );

  it("stays narrower than the default capability's any-subdomain wildcard", () => {
    // The narrowness is the point of a separate file (see its description), so
    // widening it to `https://*.constellationgate.ai/*` should be a deliberate
    // decision rather than the quiet fix for a failing row above. The gateway
    // itself is the case that matters: the popover never opens it in a browser.
    const gatewayUrl = "https://gateway-staging.constellationgate.ai/";
    expect(patterns.some((p) => globToRegExp(p).test(gatewayUrl))).toBe(false);
  });
});
