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
const doc = readFileSync(resolve(root, "docs/analytics-events.md"), "utf8").replace(/\r\n/g, "\n");
const source = readFileSync(resolve(root, "src/lib/analytics.ts"), "utf8").replace(/\r\n/g, "\n");

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

  it("documents the limits the reviews asked to be written down", () => {
    // A CLI-first install is judged legacy, a group turns on a person profile
    // for an API-key install, and the org wait has a stated fallback.
    expect(doc).toMatch(/CLI-first install is judged legacy/);
    expect(doc).toMatch(/setting a group turns on\s+person processing/);
    expect(doc).toMatch(/If no org arrives\s+within the bound, the milestone is sent without one/);
    // And the one-minute promise says when it does not hold.
    expect(doc).toMatch(/one-minute latency holds only once the diagnostics question has been\s+answered/);
  });

  it("aggregates the install funnel by organization", () => {
    const funnel = doc.slice(doc.indexOf("## Building the install funnel"));
    expect(funnel).toMatch(/Aggregating by:\s+organization/);
  });

  /** No server-side alias ties an API-key install to anyone,
   *  so neither the inventory nor the disclosure may say one does. */
  it("claims no server-side link of the device id, and discloses the opt-out note", () => {
    const dialogs = readFileSync(resolve(root, "src/components/gc/dialogs.tsx"), "utf8").replace(/\r\n/g, "\n");
    expect(doc).not.toContain("$create_alias");
    expect(dialogs).not.toContain("$create_alias");
    expect(dialogs).not.toMatch(/key&rsquo;s account/);
    expect(dialogs).not.toMatch(/Automatic collection is anonymous\./);
    // The copy names the switch, and the no and the Skip.
    expect(dialogs).toMatch(/Once you sign in, and while\s+sharing is on/);
    // Once per install, so the copy says "the first time".
    expect(dialogs).toMatch(/The first time you say no, or turn sharing off later, one final note/);
    // The gateway does store the device id with requests.
    expect(dialogs).toMatch(/stored with each request your account sends/);
    expect(dialogs).not.toMatch(/does not use it to tie this device/);
    const setup = readFileSync(resolve(root, "src/components/gc/setup.tsx"), "utf8").replace(/\r\n/g, "\n");
    // The onboarding step says what the yes ties events to.
    expect(setup).toMatch(/tied to your organization and, if you signed in with\s+Constellation, your account\./);
    // And that a no or a Skip still sends its one final note.
    expect(setup).toMatch(/Saying no or skipping still sends one\s+final note saying so, tied the same way\./);
    const upload = readFileSync(resolve(root, "src/lib/diagnosticsUpload.ts"), "utf8").replace(/\r\n/g, "\n");
    expect(upload).not.toMatch(/anonymous posture/);
  });

  it("contains no em dash", () => {
    expect(doc.includes(String.fromCharCode(0x2014))).toBe(false);
  });
});
