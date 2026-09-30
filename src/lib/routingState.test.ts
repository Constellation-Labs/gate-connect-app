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
    expect(routingState(0, 3).headline).toBe("Gate Connect is not routing your apps");
    expect(routingState(1, 3).headline).toBe("Gate Connect is partly routing your apps");
  });

  /** The card's loss: it called this "Not protected", reporting a fault the
   *  user caused on purpose. */
  it("separates nothing-asked-for from nothing-routed", () => {
    expect(kind(0, 0)).toBe("none-requested");
    expect(routingState(0, 0).headline).toBe("No apps are routed");
    expect(routingState(0, 0).label).not.toBe("Not protected");
  });

  it("prints a fraction whenever the rail has apps to divide by", () => {
    // An empty rail still has no ratio worth printing. Everything else does,
    // including the state where nothing is switched on: that used to be
    // suppressed along with it, because the fraction divided by intent and
    // "0 of 0" was meaningless. It divides by availability now, so "0 of 8"
    // is a real reading and the one a person with a full rail and nothing on
    // most needs to see.
    expect(showsFraction(0)).toBe(false);
    for (const available of [1, 3, 8]) {
      expect(showsFraction(available)).toBe(true);
    }
  });

  it("is green only when routed, amber for a failure, grey for nothing asked", () => {
    expect(routingState(3, 3).tone).toBe("green");
    // Switched on and not (fully) routing: something is wrong, and the tile
    // says so.
    expect(routingState(1, 3).tone).toBe("amber");
    expect(routingState(0, 3).tone).toBe("amber");
    // Nothing switched on is not a fault, and since the 2026-09-29 redraw it has
    // its own tone (`1390:14026`): grey, with a CircleOff glyph.
    expect(routingState(0, 0).tone).toBe("grey");
    expect(routingState(0, 0).icon).toBe("circleOff");
  });

  it("pairs the shield with the tone, so no surface can mismatch them", () => {
    expect(routingState(3, 3).icon).toBe("shieldCheck");
    expect(routingState(1, 3).icon).toBe("shieldBan");
  });

  /**
   * Every headline has to fit the tray's 360px card beside a 36px tile on one
   * line, or the two surfaces are back to needing separate copy. The headlines
   * said "Gate" for that reason until the 2026-09-29 redraw put "Gate Connect"
   * on every window banner; the longest of them, "Gate Connect is partly
   * routing your apps", measures 269px at 14px Medium (`1404:18358`) and the
   * card has 280 beside its tile, so the bound moved with the copy rather
   * than the copy being cut to the bound.
   */
  it("keeps every headline short enough for the narrow surface", () => {
    for (const [r, t] of [
      [3, 3],
      [1, 3],
      [0, 3],
      [0, 0],
    ] as const) {
      const { headline } = routingState(r, t);
      expect(headline.length).toBeLessThanOrEqual(40);
    }
  });

  /** A reading can only arrive from the two counts, so a nonsensical pair has
   *  to land somewhere rather than throw at the user. */
  it("treats more routed than requested as routed, not as a fourth thing", () => {
    expect(kind(4, 3)).toBe("routed");
    expect(kind(1, 0)).toBe("none-requested");
  });
});
