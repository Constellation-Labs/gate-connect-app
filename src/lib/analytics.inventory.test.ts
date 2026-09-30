import { describe, expect, it } from "vitest";
import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { ALLOWED_PROP_KEYS } from "./analytics";
import { CONNECTION_FAILURE_REASONS } from "./errors";

/**
 * The analytics inventory (`docs/analytics-events.md`) and the code that sends
 * the events must not drift. AG-960's acceptance criteria require every event
 * and property to be listed there, and a list that was true once is a list
 * somebody trusts after it stops being true.
 *
 * Read from source text rather than from a runtime export for the event names:
 * `AnalyticsEvent` is a type and has no runtime value, and a parallel array kept
 * beside it would be a third copy to drift. The union is parsed out of
 * `analytics.ts` itself.
 */
const root = resolve(__dirname, "../..");
const doc = readFileSync(resolve(root, "docs/analytics-events.md"), "utf8");
const source = readFileSync(resolve(root, "src/lib/analytics.ts"), "utf8");

/** The backticked first-column names of the table under `## <heading>`. */
function tableNames(heading: string): string[] {
  const start = doc.indexOf(`\n## ${heading}\n`);
  expect(start, `the doc has a "${heading}" section`).toBeGreaterThanOrEqual(0);
  const rest = doc.slice(start + heading.length + 5);
  const end = rest.search(/\n## /);
  const section = end === -1 ? rest : rest.slice(0, end);
  return [...section.matchAll(/^\| `([a-z_]+)` \|/gm)].map((m) => m[1]);
}

function unionMembers(): string[] {
  const m = source.match(/export type AnalyticsEvent =([\s\S]*?);/);
  expect(m, "analytics.ts declares AnalyticsEvent").not.toBeNull();
  return [...m![1].matchAll(/\|\s*"([a-z_]+)"/g)].map((x) => x[1]);
}

const sorted = (xs: Iterable<string>) => [...new Set(xs)].sort();

describe("docs/analytics-events.md stays in step with the code", () => {
  it("lists exactly the events AnalyticsEvent allows", () => {
    const events = unionMembers();
    expect(events.length).toBeGreaterThan(20);
    expect(sorted(tableNames("Event inventory"))).toEqual(sorted(events));
  });

  it("lists exactly the props the allowlist lets through", () => {
    expect(sorted(tableNames("Properties"))).toEqual(sorted(ALLOWED_PROP_KEYS));
  });

  it("lists exactly the connection failure reasons", () => {
    expect(sorted(tableNames("Connection failure reasons"))).toEqual(
      sorted(CONNECTION_FAILURE_REASONS),
    );
  });

  it("names the funnel's steps in order", () => {
    const funnel = doc.slice(doc.indexOf("## Building the install funnel"));
    const steps = ["setup_download_clicked", "app_first_launched", "pairing_completed", "first_gateway_request"];
    const at = steps.map((s) => funnel.indexOf(`\`${s}\``));
    for (const i of at) expect(i).toBeGreaterThanOrEqual(0);
    expect([...at].sort((a, b) => a - b)).toEqual(at);
  });

  it("contains no em dash", () => {
    expect(doc.includes(String.fromCharCode(0x2014))).toBe(false);
  });
});
