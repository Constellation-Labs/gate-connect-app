import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, render, screen } from "@testing-library/react";
import { SessionConsentDialog } from "./dialogs";
import type { GroupMember } from "../../lib/groups";

const noop = () => {};

afterEach(cleanup);

/**
 * The question asked before a signed-in session is routed.
 *
 * It had no component test at all, which `docs/review-feat-routing-taxonomy-code.md`
 * flagged: "a regression that routes but does not ask" has nothing standing in
 * its way. These hold the two things the dialog owes a reader - what it is
 * about to do, and that the answer is reversible - rather than the exact
 * sentences, so the copy can be edited without rewriting the suite.
 */
function member(overrides: Partial<GroupMember> = {}): GroupMember {
  return {
    key: "claude-web",
    kind: "proxy",
    name: "chat",
    routed: false,
    desired: false,
    attention: null,
    client: "claude",
    scope: "host",
    credential: "additive",
    cascade: false,
    domain: {
      slug: "claude-web",
      display_name: "Claude",
      hosts: ["claude.ai"],
      upstream_url: "https://claude.ai",
      rewrite_prefixes: [],
      passthrough_prefixes: [],
      enabled: false,
      supported: true,
      client: "claude",
      credential: "additive",
      scope: "host",
    },
    ...overrides,
  } as GroupMember;
}

function consent(surfaces: GroupMember[] = [member()]) {
  return render(
    <SessionConsentDialog
      name="Claude"
      surfaces={surfaces}
      onDismiss={noop}
      onConfirm={noop}
    />,
  );
}

describe("SessionConsentDialog", () => {
  it("names the host, which is the only part the reader can recognise", () => {
    consent();

    // Not the surface's label: those are the rail's one-word names, so the
    // ChatGPT dialog read "This also routes chat and subscription".
    // Twice: the subtitle names it, and the wide-reach sentence names it again.
    expect(screen.getAllByText(/claude\.ai/).length).toBeGreaterThan(0);
    expect(screen.queryByText(/routes chat\b/)).toBeNull();
  });

  it("says the decision is reversible, and how", () => {
    // AG-901. It used to say "Asked once per app. Turning Claude off later does
    // not bring this question back" - two facts about the dialog which, read
    // together, sound like consent that cannot be withdrawn. The switch IS the
    // withdrawal, and that is the sentence a person deciding needs.
    consent();

    expect(screen.getByText(/turn Claude off whenever you like/)).toBeTruthy();
    expect(screen.getByText(/routing stops/)).toBeTruthy();
  });

  it("says the question is asked once, once", () => {
    // The paragraph used to say "You are asked this once" and then "Gate will
    // not ask this question again": one fact, stated twice in three lines.
    consent();

    const said = screen.getByText(/asked this once/).textContent ?? "";
    expect(said.match(/ask/g)?.length).toBe(2);
    expect(said).not.toMatch(/ask this question again/);
  });

  it("does not offer a setting that would undo it, because there is none", () => {
    // `accept_session_routing` never un-records and `account::clear` leaves
    // `preferences.json` in place, so a reset does not clear it either. Copy
    // pointing at Settings would be a promise the app does not keep.
    consent();

    expect(screen.queryByText(/in Settings/i)).toBeNull();
  });

  it("warns that a host route is wider than the app it is named for", () => {
    consent();

    expect(screen.getByText(/everything on this machine/)).toBeTruthy();
    // The taxonomy's word for it, which named a mechanism instead of a
    // consequence.
    expect(screen.queryByText(/matched on host/)).toBeNull();
  });

  it("leaves the wide-reach sentence out where nothing is host-scoped", () => {
    consent([member({ scope: "client", domain: undefined })]);

    expect(screen.queryByText(/everything on this machine/)).toBeNull();
  });

  it("puts the keyboard on the safe answer", () => {
    // Principle 5: the primary starts routing a signed-in session.
    consent();

    expect(document.activeElement?.textContent).toBe("Not now");
  });

  it("asks, rather than routing, until the primary is clicked", () => {
    const onConfirm = vi.fn();
    render(
      <SessionConsentDialog
        name="Claude"
        surfaces={[member()]}
        onDismiss={noop}
        onConfirm={onConfirm}
      />,
    );

    expect(onConfirm).not.toHaveBeenCalled();
    screen.getByRole("button", { name: "Route Claude" }).click();
    expect(onConfirm).toHaveBeenCalledOnce();
  });
});
