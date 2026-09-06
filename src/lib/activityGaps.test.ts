import { describe, expect, it } from "vitest";
import { toFailure } from "./activity";
import { failureNotice, mergeNotices, sectionNotice } from "./activityGaps";
import type { GapAction, GapNotice } from "./activityGaps";
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

  it("does not send a machine credential to ask an admin for permission", () => {
    // The gateway raises `attribution` for a credential with no user on it,
    // which is what an org-scoped key is. No role change fixes that, so the copy
    // must not imply one, and the offer is a credential that belongs to a person.
    const n = sectionNotice("Messages", "attribution");
    expect(n.cause).not.toMatch(/role|permission|owner|admin/i);
    expect(n.actions.map((a) => a.kind)).toEqual(["api-keys", "docs"]);
  });

  it("points a role problem at the person who can fix it", () => {
    expect(sectionNotice("Blocked and flagged", "access").cause).toMatch(
      /owner or admin/i,
    );
  });
});

describe("mergeNotices", () => {
  const notice = (subject: string, cause: string, actions: GapAction[] = []): GapNotice => ({
    subject,
    cause,
    actions,
  });
  const keys: GapAction = { kind: "api-keys", label: "Manage API keys" };
  const docs: GapAction = { kind: "docs", label: "Read Gate docs" };
  const retry: GapAction = { kind: "retry", label: "Try again" };

  it("collapses one cause repeated across every section into a single notice", () => {
    // The state that prompted this: a credential with no user attached defeats
    // all five sections, and the Overview printed the same sentence five times
    // with ten identical links under it.
    const same = ["Messages", "Savings", "Tokens saved", "Hourly chart", "Blocked and flagged"].map(
      (s) => notice(s, "The credential in use has no user attached.", [keys, docs]),
    );
    const merged = mergeNotices(same);
    expect(merged).toHaveLength(1);
    expect(merged[0]!.cause).toBe("The credential in use has no user attached.");
    expect(merged[0]!.actions).toEqual([keys, docs]);
  });

  it("attributes a reading-wide cause to the reading, not to a section", () => {
    const same = ["Messages", "Savings"].map((s) => notice(s, "No user attached.", [keys]));
    expect(mergeNotices(same)[0]!.subject).toBe("Activity");
  });

  it("names the sections when the cause covers only some of them", () => {
    // Saying "Activity" here would claim the rest of the pane is broken too.
    const merged = mergeNotices([
      notice("Messages", "No user attached.", [keys]),
      notice("Savings", "No user attached.", [keys]),
      notice("Policies", "Nothing is set up for this yet.", []),
    ]);
    expect(merged).toHaveLength(2);
    expect(merged.map((n) => n.subject)).toContain("Messages and Savings");
  });

  it("keeps notices apart when the same cause offers different actions", () => {
    // Two sections that fail alike but are fixed differently are two notices.
    // Merging them would offer an action that does nothing for one of them.
    const merged = mergeNotices([
      notice("Messages", "Could not be fetched.", [retry]),
      notice("Savings", "Could not be fetched.", [keys]),
    ]);
    expect(merged).toHaveLength(2);
  });

  it("leaves a single notice exactly as it was", () => {
    const one = [notice("Policies", "Nothing is set up for this yet.", [])];
    expect(mergeNotices(one)).toEqual(one);
  });

  it("joins three or more section names readably", () => {
    const merged = mergeNotices([
      notice("A", "same", [keys]),
      notice("B", "same", [keys]),
      notice("C", "same", [keys]),
      notice("D", "other", [keys]),
    ]);
    expect(merged.find((n) => n.cause === "same")!.subject).toBe("A, B and C");
  });

  it("returns nothing for nothing", () => {
    expect(mergeNotices([])).toEqual([]);
  });
});
