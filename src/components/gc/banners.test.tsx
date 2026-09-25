import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, render, screen } from "@testing-library/react";
import { ReopenAlert, RoutingBanner } from "./banners";

afterEach(cleanup);

/**
 * The pane's reopen card, which is the window's only surface for a pending
 * reopen since the shell banner was removed.
 *
 * Unit-level because the branch that removed the banner took the redundancy
 * with it: e2e proves the card is reachable, and these prove what it says when
 * it gets there - in particular the degraded case, which is the one no frame
 * draws and the one a sweep produces on a real machine.
 */
describe("ReopenAlert", () => {
  const props = {
    name: "Claude Code",
    onReopen: () => {},
  };

  it("names the tool in the sentence, and announces itself", () => {
    render(<ReopenAlert {...props} routeInUse="api.anthropic.com" requestedRoute="gw.example" />);

    // The rail carries the bare phrase, so the card's job is to say which tool
    // - it is the only surface that does on a single-member section.
    expect(screen.getByText("Reopen Claude Code to finish")).toBeTruthy();
    // Raised by a background sweep with nothing the user did behind it. Silence
    // was what the deleted shell banner's `role="status"` used to prevent.
    expect(screen.getByRole("status")).toBeTruthy();
  });

  it("draws both routes when the verdict established both", () => {
    render(<ReopenAlert {...props} routeInUse="api.anthropic.com" requestedRoute="gw.example" />);

    expect(screen.getByText("api.anthropic.com")).toBeTruthy();
    expect(screen.getByText("gw.example")).toBeTruthy();
  });

  /**
   * AG-570 asks for the route in use and the requested route, and the card used
   * to infer them from the config file when the sweep could not say. It was
   * caught being wrong - a tool with no Gate values anywhere for a week, under a
   * card claiming it was still on the gateway - so a missing half now drops the
   * pair rather than guessing at it. Principle 6: a figure is a measurement.
   */
  it.each([
    ["no route in use", { routeInUse: null, requestedRoute: "gw.example" }],
    ["no requested route", { routeInUse: "api.anthropic.com", requestedRoute: null }],
    ["neither", { routeInUse: null, requestedRoute: null }],
  ])("names no endpoint at all when the sweep gave %s", (_label, routes) => {
    render(<ReopenAlert {...props} {...routes} />);

    // The phrase and the action survive; only the claim about traffic goes.
    expect(screen.getByText("Reopen Claude Code to finish")).toBeTruthy();
    expect(screen.getByRole("button", { name: "Close tool" })).toBeTruthy();
    expect(screen.queryByText(/In use:/)).toBeNull();
    expect(screen.queryByText(/Requested:/)).toBeNull();
  });

  it("offers the close, because Gate cannot start a CLI again", () => {
    const onReopen = vi.fn();
    render(<ReopenAlert {...props} onReopen={onReopen} />);

    // "Close tool", not "Reopen Claude Code": the button raises the close
    // confirmation, and a label promising the reopen promised the one thing
    // this flow never does.
    screen.getByRole("button", { name: "Close tool" }).click();

    expect(onReopen).toHaveBeenCalledTimes(1);
  });
});

/**
 * The topbar's fraction, which is the half of the 2026-09-23 change that no
 * test covered.
 *
 * It is also the half that deviates from a drawn frame: `228:85990` draws
 * `Routed · 4 of 4 Apps`, routed over requested. These pin the replacement so
 * a later "match the frame" pass fails a test rather than quietly reverting a
 * decision someone made on purpose.
 */
describe("RoutingBanner's fraction", () => {
  it("counts apps switched on, out of apps available", () => {
    // Two on and both routed, on a rail of eight. The old ratio said "2 of 2".
    render(
      <RoutingBanner protectedCount={2} totalCount={2} availableCount={8} />,
    );
    expect(screen.getByText("Gate is protecting you")).toBeTruthy();
    expect(screen.getByText("2 of 8 Apps on")).toBeTruthy();
  });

  it("keeps counting intent while the state reports the failure", () => {
    // Two on, one routed. The digits do not move - they are coverage, not
    // outcome - and the pill and headline are what say something is wrong.
    // This is the case where the fraction disagrees with the rail's group
    // counters on the numerator; see `routingState`.
    render(
      <RoutingBanner protectedCount={1} totalCount={2} availableCount={8} />,
    );
    expect(screen.getByText("Gate is partly routing your apps")).toBeTruthy();
    expect(screen.getByText("Partly routed")).toBeTruthy();
    expect(screen.getByText("2 of 8 Apps on")).toBeTruthy();
  });

  it("prints 0 of M with nothing switched on, where it used to print nothing", () => {
    // The reading a full rail with nothing on most needs, and the one the old
    // `showsFraction` suppressed along with the meaningless "0 of 0".
    render(
      <RoutingBanner protectedCount={0} totalCount={0} availableCount={8} />,
    );
    expect(screen.getByText("No apps are set to route")).toBeTruthy();
    expect(screen.getByText("0 of 8 Apps on")).toBeTruthy();
  });

  it("says nothing about a ratio when the rail is empty", () => {
    // "0 of 0" is a ratio with both halves meaningless, and that suppression
    // is the one this change kept.
    render(
      <RoutingBanner protectedCount={0} totalCount={0} availableCount={0} />,
    );
    expect(screen.getByText("No apps are set to route")).toBeTruthy();
    expect(screen.queryByText(/of 0 Apps/)).toBeNull();
  });

  it("says 'on', so the ratio cannot be read as routed-of-available", () => {
    // Without the suffix, "2 of 8 Apps" beside a "Routed" pill states that two
    // of eight are routed, which is a different and false claim.
    render(
      <RoutingBanner protectedCount={2} totalCount={2} availableCount={8} />,
    );
    expect(screen.queryByText("2 of 8 Apps")).toBeNull();
  });
});
