import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, render, screen } from "@testing-library/react";
import { ReopenAlert } from "./banners";

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
