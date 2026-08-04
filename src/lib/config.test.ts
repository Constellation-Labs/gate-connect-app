import { describe, expect, it } from "vitest";
// `?raw` rather than node:fs: tsconfig sets types to ["vite/client"] only, and
// widening the app's type surface to all of node just to read a file in a test
// is the wrong trade. vite/client already declares `*?raw` as a string.
import capabilitiesRaw from "../../src-tauri/capabilities/default.json?raw";
import { GATE_API_KEYS_URL, GATE_DASHBOARD_URL } from "./config";

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

  it.each([
    ["dashboard", GATE_DASHBOARD_URL],
    ["api keys", GATE_API_KEYS_URL],
  ])("allows the %s URL", (_label, url) => {
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
