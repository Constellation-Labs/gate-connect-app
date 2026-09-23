import { describe, expect, it } from "vitest";
import { routingState, showsFraction, type RoutingStateKind } from "./routingState";

/**
 * AG-913. The topbar banner and the tray's routing card answer the same question
 * and used to answer it in different words, each losing a distinction the other
 * kept.
 */
describe("routingState", () => {
  const kind = (routed: number, requested: number): RoutingStateKind =>
    routingState(routed, requested).kind;

  it("is routed only when everything asked for is", () => {
    expect(kind(3, 3)).toBe("routed");
    expect(kind(2, 3)).toBe("partly");
  });

  /** The banner's loss: it folded this into "partly", and none of three is not
   *  partly of anything. */
  it("separates none-routed from partly routed", () => {
    expect(kind(0, 3)).toBe("none-routed");
    expect(routingState(0, 3).headline).toBe("Gate is not routing your apps");
    expect(routingState(1, 3).headline).toBe("Gate is partly routing your apps");
  });

  /** The card's loss: it called this "Not protected", reporting a fault the
   *  user caused on purpose. */
  it("separates nothing-asked-for from nothing-routed", () => {
    expect(kind(0, 0)).toBe("none-requested");
    expect(routingState(0, 0).headline).toBe("No apps are set to route");
    expect(routingState(0, 0).label).not.toBe("Not protected");
  });

  it("prints a fraction for every state but the empty denominator", () => {
    expect(showsFraction(routingState(0, 0))).toBe(false);
    for (const [r, t] of [
      [3, 3],
      [1, 3],
      [0, 3],
    ] as const) {
      expect(showsFraction(routingState(r, t))).toBe(true);
    }
  });

  it("is green only when routed, because there is no third tone drawn", () => {
    expect(routingState(3, 3).tone).toBe("green");
    for (const [r, t] of [
      [1, 3],
      [0, 3],
      [0, 0],
    ] as const) {
      expect(routingState(r, t).tone).toBe("amber");
    }
  });

  it("pairs the shield with the tone, so no surface can mismatch them", () => {
    expect(routingState(3, 3).icon).toBe("shieldCheck");
    expect(routingState(1, 3).icon).toBe("shieldBan");
  });

  /**
   * Every headline has to fit the tray's 360px card beside a 36px tile, which
   * is why they say "Gate" rather than "Gate Connect". A longer one would wrap
   * the card and the two surfaces would be back to needing separate copy.
   */
  it("keeps every headline short enough for the narrow surface", () => {
    for (const [r, t] of [
      [3, 3],
      [1, 3],
      [0, 3],
      [0, 0],
    ] as const) {
      const { headline } = routingState(r, t);
      expect(headline.length).toBeLessThanOrEqual(34);
      expect(headline).not.toMatch(/Gate Connect/);
    }
  });

  /** A reading can only arrive from the two counts, so a nonsensical pair has
   *  to land somewhere rather than throw at the user. */
  it("treats more routed than requested as routed, not as a fourth thing", () => {
    expect(kind(4, 3)).toBe("routed");
    expect(kind(1, 0)).toBe("none-requested");
  });
});
