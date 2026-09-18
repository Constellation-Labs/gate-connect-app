import { describe, expect, it } from "vitest";
import { dashboardLinks, dashboardOrigin } from "./dashboard";

describe("dashboardOrigin", () => {
  it("maps the production gateway to the production dashboard", () => {
    expect(dashboardOrigin("https://gateway.constellationgate.ai")).toBe(
      "https://app.constellationgate.ai",
    );
  });

  it("keeps the environment suffix, which is the whole point", () => {
    // The bug this module exists to fix: a staging gateway used to hand out
    // production dashboard links from five call sites.
    expect(dashboardOrigin("https://gateway-staging.constellationgate.ai")).toBe(
      "https://app-staging.constellationgate.ai",
    );
  });

  it("maps an environment nobody has added yet, without another edit here", () => {
    expect(dashboardOrigin("https://gateway-dev.constellationgate.ai")).toBe(
      "https://app-dev.constellationgate.ai",
    );
  });

  it("tolerates a trailing slash and a path on the gateway URL", () => {
    expect(dashboardOrigin("https://gateway.constellationgate.ai/")).toBe(
      "https://app.constellationgate.ai",
    );
  });

  it.each([
    ["a local dev gateway, which has no dashboard", "http://localhost:3000"],
    ["an https localhost", "https://localhost:3000"],
    ["a host outside constellationgate.ai", "https://gateway.evil.example.com"],
    ["a constellationgate host that is not a gateway", "https://app.constellationgate.ai"],
    ["a deeper subdomain the rule was not written for", "https://gateway.eu.constellationgate.ai"],
    ["a lookalike suffix", "https://gateway.notconstellationgate.ai"],
    ["an unparseable value", "not a url"],
    ["an empty string", ""],
  ])("returns null for %s", (_label, input) => {
    expect(dashboardOrigin(input)).toBeNull();
  });

  it("returns null rather than guessing when there is no account yet", () => {
    // `Account` is null before the first read lands, and on a failed keychain
    // read. Neither is a licence to pick an environment.
    expect(dashboardOrigin(null)).toBeNull();
    expect(dashboardOrigin(undefined)).toBeNull();
  });
});

describe("dashboardLinks", () => {
  const links = dashboardLinks("https://gateway-staging.constellationgate.ai")!;

  it("builds every destination on the matching environment", () => {
    expect(links).toMatchObject({
      root: "https://app-staging.constellationgate.ai/",
      apiKeys: "https://app-staging.constellationgate.ai/api-keys",
      policies: "https://app-staging.constellationgate.ai/policies",
      savings: "https://app-staging.constellationgate.ai/token-savings",
      // Support is the Overview page: the dashboard has no support route, the
      // support control is a floating button in its corner (2026-09-07).
      support: "https://app-staging.constellationgate.ai/overview",
    });
  });

  it("percent-encodes a request id into the message link", () => {
    expect(links.message("req/1 2")).toBe(
      "https://app-staging.constellationgate.ai/messages/req%2F1%202",
    );
  });

  it("gives every link a path, because a bare origin is rejected by the ACL", () => {
    // `glob::Pattern` matches `https://*.constellationgate.ai/*` against the raw
    // string, and a bare origin has no `/` for the literal separator. A link
    // that fails this is a button that silently does nothing.
    const every = [links.root, links.apiKeys, links.policies, links.savings, links.support, links.message("x")];
    for (const url of every) {
      expect(new URL(url).pathname.length, url).toBeGreaterThan(0);
      expect(url.startsWith("https://app-staging.constellationgate.ai/"), url).toBe(true);
    }
  });

  it("is null for a gateway with no dashboard, so callers must handle it", () => {
    expect(dashboardLinks("http://localhost:3000")).toBeNull();
  });
});
