import { describe, expect, it } from "vitest";
import { NO_ORG, orgLabel } from "./orgLabel";

/**
 * One fact, one answer, whichever surface is asking.
 *
 * The bug this pins: three call sites resolved the organization name
 * independently and disagreed from identical state. On an api-key account
 * `Account.org_name` is null by design - it records the org the user *picked*,
 * which only OAuth does - so every chain fell through, and the window's rail
 * showed the real name off the activity reading while the tray's footer said
 * "No organization" and the setup pane showed the gateway URL.
 *
 * These tests are written as "both surfaces, same state, same string" rather
 * than as three separate expectations, because agreeing is the requirement.
 */
describe("orgLabel", () => {
  it("prefers the account's own name", () => {
    expect(orgLabel({ org_name: "Acme Engineering" }, "From The Reading")).toBe(
      "Acme Engineering",
    );
  });

  it("falls back to the reading when the account names no org", () => {
    // The api-key case, which is the one that was broken.
    expect(orgLabel({ org_name: null }, "Matheus Reis's organization")).toBe(
      "Matheus Reis's organization",
    );
  });

  it("gives the window and the tray the same answer from the same state", () => {
    const account = { org_name: null };
    const reading = "Matheus Reis's organization";

    // The window has the live overview; the tray has the cached one. Same
    // figure from the same endpoint, so the same string has to come out.
    const window = orgLabel(account, reading);
    const tray = orgLabel(account, reading);

    expect(tray).toBe(window);
    expect(tray).not.toBe(NO_ORG);
  });

  it("says No organization when nothing names one, and nothing cleverer", () => {
    expect(orgLabel({ org_name: null }, null)).toBe(NO_ORG);
    expect(orgLabel({ org_name: null }, undefined)).toBe(NO_ORG);
    expect(orgLabel(null, null)).toBe(NO_ORG);
    expect(orgLabel(undefined, undefined)).toBe(NO_ORG);
  });

  it("treats an empty name as no name rather than printing a blank", () => {
    // `??` would have let "" through and drawn an empty footer, which reads as
    // a layout bug rather than as a missing reading.
    expect(orgLabel({ org_name: "" }, "From The Reading")).toBe("From The Reading");
    expect(orgLabel({ org_name: "" }, "")).toBe(NO_ORG);
  });
});
