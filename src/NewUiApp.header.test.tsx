import { afterEach, describe, expect, it } from "vitest";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { defaultState } from "../e2e/backend";
import { installFakeTauri } from "../e2e/install";
import { NewUiApp } from "./NewUiApp";

/**
 * The size the app pane header draws its mark at, which `NewUiApp` chooses and
 * `AppPane` only places: 24 in the 44px tile, per the new design, while the
 * rail keeps 16 for the same mark.
 */
afterEach(cleanup);

describe("the app pane header's mark", () => {
  it("draws at 24 while the rail's stays at 16", async () => {
    installFakeTauri(defaultState());
    render(<NewUiApp />);

    const [railRow] = await screen.findAllByRole("button", { name: /^Claude/ });
    expect(railRow.querySelector("svg")?.getAttribute("width")).toBe("16");

    fireEvent.click(railRow);

    const header = (await screen.findByRole("heading", { level: 1, name: "Claude Desktop" })).closest(
      "header",
    )!;
    const mark = header.querySelector("span[aria-hidden] svg")!;
    expect(mark.getAttribute("width")).toBe("24");
    expect(mark.getAttribute("height")).toBe("24");
  });
});
