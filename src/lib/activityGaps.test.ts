import { describe, expect, it } from "vitest";
import { toFailure } from "./activity";
import { failureNotice, sectionNotice } from "./activityGaps";
import type { FailureCode } from "./activity";
import type { UnavailableReason } from "./activity";

describe("toFailure", () => {
  it("reads the command's envelope", () => {
    expect(
      toFailure('{"code":"offline","message":"calling the gateway: dns error"}'),
    ).toEqual({ code: "offline", message: "calling the gateway: dns error" });
  });

  it("falls back to unknown for anything that is not the envelope", () => {
    // An unregistered command on an older binary, or a plugin error.
    expect(toFailure("command activity_overview not found")).toEqual({
      code: "unknown",
      message: "command activity_overview not found",
    });
    expect(toFailure(new Error("boom")).code).toBe("unknown");
  });

  it("refuses a code it does not know rather than trusting it", () => {
    // A newer core crate could add a code this bundle has no copy for. Treating
    // it as unknown shows a generic cause; passing it through would show none.
    const f = toFailure('{"code":"teapot","message":"short and stout"}');
    expect(f.code).toBe("unknown");
    expect(f.message).toContain("teapot");
  });
});

describe("failureNotice", () => {
  const codes: FailureCode[] = [
    "offline",
    "signed_out",
    "no_org",
    "rejected",
    "gateway",
    "unknown",
  ];

  it("names a cause and offers at least one action for every code", () => {
    for (const code of codes) {
      const n = failureNotice({ code, message: "detail" });
      expect(n.cause.length).toBeGreaterThan(0);
      expect(n.actions.length).toBeGreaterThan(0);
    }
  });

  it("never mentions routing, which is a different fact entirely", () => {
    // AG-576's own conflation: an unreachable gateway is not a routing switch
    // the user left off, and saying so sends them to fix nothing.
    for (const code of codes) {
      const n = failureNotice({ code, message: "detail" });
      expect(`${n.cause} ${n.actions.map((a) => a.label).join(" ")}`).not.toMatch(
        /routing|protected/i,
      );
    }
  });

  it("does not offer a retry for causes a retry cannot fix", () => {
    expect(failureNotice({ code: "no_org", message: "" }).actions).not.toContainEqual(
      expect.objectContaining({ kind: "retry" }),
    );
    expect(failureNotice({ code: "rejected", message: "" }).actions).not.toContainEqual(
      expect.objectContaining({ kind: "retry" }),
    );
  });

  it("keeps the underlying detail out of the user-facing copy", () => {
    const n = failureNotice({
      code: "gateway",
      message: "gateway /v1/me/activity returned 500: {}",
    });
    expect(n.cause).not.toContain("500");
    expect(n.cause).not.toContain("/v1/me/activity");
  });
});

describe("sectionNotice", () => {
  const reasons: UnavailableReason[] = [
    "connectivity",
    "access",
    "attribution",
    "not_configured",
    "definition_pending",
  ];

  it("names the section and a cause for every reason", () => {
    for (const reason of reasons) {
      const n = sectionNotice("Blocked and flagged", reason);
      expect(n.subject).toBe("Blocked and flagged");
      expect(n.cause.length).toBeGreaterThan(0);
    }
  });

  it("offers nothing when nothing the user can reach would help", () => {
    // A role is granted by someone else, and an undefined measure is ours to
    // define. A button for either would be a dead end dressed as a remedy.
    expect(sectionNotice("Blocked and flagged", "access").actions).toEqual([]);
    expect(sectionNotice("Needs review", "definition_pending").actions).toEqual([]);
  });

  it("points a role problem at the person who can fix it", () => {
    expect(sectionNotice("Blocked and flagged", "access").cause).toMatch(
      /owner or admin/i,
    );
  });
});
