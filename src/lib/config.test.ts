import { describe, expect, it } from "vitest";
// `?raw` rather than node:fs: tsconfig sets types to ["vite/client"] only, and
// widening the app's type surface to all of node just to read a file in a test
// is the wrong trade. vite/client already declares `*?raw` as a string.
import capabilitiesRaw from "../../src-tauri/capabilities/default.json?raw";
import { consoleUrlFor, gatewayEnvLabel, GATEWAY_SERVERS } from "./config";

/** Translate a `glob::Pattern` (what tauri-plugin-opener matches with) into a
 * regex. Only the constructs the capability file uses: `*` for any run of
 * characters. Critically, `*` matches zero or more, but a literal `/` in the
 * pattern still has to be present in the input - which is the whole reason
 * `https://app.constellationgate.ai` was rejected while the slashed form was
 * allowed. */
function globToRegExp(pattern: string): RegExp {
  const escaped = pattern.replace(/[.+?^${}()|[\]\\]/g, "\\$&").replace(/\*/g, ".*");
  return new RegExp(`^${escaped}$`);
}

function openerAllowPatterns(): string[] {
  const caps = JSON.parse(capabilitiesRaw) as {
    permissions: (string | { identifier: string; allow?: { url?: string }[] })[];
  };
  const entry = caps.permissions.find(
    (p): p is { identifier: string; allow?: { url?: string }[] } =>
      typeof p === "object" && p.identifier === "opener:allow-open-url",
  );
  return (entry?.allow ?? []).map((a) => a.url).filter((u): u is string => !!u);
}

describe("external URLs are permitted by the opener ACL", () => {
  const patterns = openerAllowPatterns();

  it("finds the opener scope in the capability file", () => {
    // If this fails the rest is vacuous, so assert the fixture is real.
    expect(patterns.length).toBeGreaterThan(0);
  });

  // Every server, not just production: the console URL is now derived from the
  // account's gateway, so a staging user follows a link no test used to cover,
  // and an ACL miss there is a button that silently does nothing.
  it.each(
    GATEWAY_SERVERS.flatMap((server) => [
      [`${server.label} dashboard`, consoleUrlFor(server.url)],
      [`${server.label} api keys`, consoleUrlFor(server.url, "api-keys")],
    ]),
  )("allows the %s URL", (_label, url) => {
    // `openUrl` rejects silently when the ACL blocks it, so a URL that no
    // pattern matches is a button that does nothing.
    const allowed = patterns.some((p) => globToRegExp(p).test(url));
    expect(allowed, `${url} matches none of ${JSON.stringify(patterns)}`).toBe(true);
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

/** The pairing this whole indirection exists for: staging and production are
 *  separate stacks with separate databases, and the app used to offer the
 *  production console from both. A user routing to staging read an empty
 *  production dashboard for a session with nothing on either surface naming
 *  the mismatch. */
describe("the console follows the gateway", () => {
  it("sends each gateway to the console that reads its database", () => {
    expect(consoleUrlFor("https://gateway.constellationgate.ai")).toBe(
      "https://app.constellationgate.ai/",
    );
    expect(consoleUrlFor("https://gateway-staging.constellationgate.ai")).toBe(
      "https://app-staging.constellationgate.ai/",
    );
  });

  it("keeps a path under the right console", () => {
    expect(consoleUrlFor("https://gateway-staging.constellationgate.ai", "api-keys")).toBe(
      "https://app-staging.constellationgate.ai/api-keys",
    );
  });

  it("tolerates a trailing slash on the stored gateway", () => {
    // `save_account` normalises, but the account file is on disk and this is
    // the cheap half of not caring.
    expect(consoleUrlFor("https://gateway-staging.constellationgate.ai/")).toBe(
      "https://app-staging.constellationgate.ai/",
    );
  });

  it("falls back to production for a gateway no picker can produce", () => {
    expect(consoleUrlFor("https://gateway.example.com")).toBe("https://app.constellationgate.ai/");
    expect(consoleUrlFor(null)).toBe("https://app.constellationgate.ai/");
  });
});

describe("the environment badge", () => {
  it("names a non-default environment", () => {
    expect(gatewayEnvLabel("https://gateway-staging.constellationgate.ai")).toBe("staging");
  });

  it("stays silent on production, so the badge means something when it appears", () => {
    expect(gatewayEnvLabel("https://gateway.constellationgate.ai")).toBeNull();
    expect(gatewayEnvLabel(undefined)).toBeNull();
  });
});
