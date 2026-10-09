import { afterEach, describe, expect, it } from "vitest";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { defaultState } from "../e2e/backend";
import { installFakeTauri } from "../e2e/install";
import { NewUiApp } from "./NewUiApp";

/**
 * The Model selection card's warning, drawn in the pane's alert slot with a
 * title per cause. The App frames draw every alert above the stat tiles and none
 * inside the card, so the note has to come before the card's heading.
 *
 * Titles name the product, not the rail's row label: Claude Code's row reads
 * "CLI", and a title built from it said "CLI’s requests are being refused".
 */
afterEach(cleanup);

const MODEL = "anthropic/claude-opus-5";

/** Claude Code connected and on a Gate model that the catalogue still serves. */
function onGateModels() {
  const state = defaultState();
  state.proxy.running = true;
  state.proxy.port = 45980;
  state.proxy.ca_trusted = true;
  state.tools.find((t) => t.slug === "claude-code")!.status = { kind: "connected" };
  state.toolModels.choices["claude-code"] = { source: "gate", model_ids: [MODEL] };
  state.toolModels.paidAckUnix = 1787740800;
  state.toolModels.catalogue = [
    { id: MODEL, owned_by: "anthropic", name: "Claude Opus 5", tags: ["tool-use"] },
  ];
  state.toolModels.credits = {
    plan: "pro",
    paygEnabled: true,
    balanceCents: 1025,
    lowBalanceThresholdCents: 500,
    autoTopupArmed: false,
  };
  return state;
}

async function openClaude() {
  render(<NewUiApp />);
  const [railRow] = await screen.findAllByRole("button", { name: /^Claude/ });
  fireEvent.click(railRow);
}

const follows = (a: Node, b: Node) =>
  (a.compareDocumentPosition(b) & Node.DOCUMENT_POSITION_FOLLOWING) !== 0;

/**
 * The note under `title`, checked to sit above the Model selection card and
 * below the "isn't protected" note, and to be the only place `body` is drawn:
 * a card that still drew its own copy would show the sentence twice.
 *
 * The default fixture leaves Claude Desktop unrouted, so the partly protected
 * note is always there to order against.
 */
async function noteTitled(title: string, body: string) {
  const heading = await screen.findByText(title);
  const note = heading.closest("[role=status]")!;
  expect(note.className).toContain("bg-amber-50");
  const card = await screen.findByText("Model selection");
  expect(follows(note, card)).toBe(true);
  const statusNote = screen.getByText("Claude isn’t fully protected").closest("[role=status]")!;
  expect(follows(statusNote, note)).toBe(true);
  const drawn = [...document.querySelectorAll("[role=status]")].filter((n) =>
    n.textContent?.includes(body),
  );
  expect(drawn).toEqual([note]);
  return note;
}

describe("the model warning", () => {
  it("titles a cause from the credit standing, in the pane's alert slot", async () => {
    const state = onGateModels();
    state.toolModels.credits.paygEnabled = false;
    installFakeTauri(state);
    await openClaude();

    await noteTitled("Pay-as-you-go is off", "Pay-as-you-go is off for this organization");
  });

  it("titles a config that could not be read", async () => {
    const state = onGateModels();
    state.toolModels.problem = {
      "claude-code": {
        state: "not_applied",
        message: "Gate Connect could not read Claude Code's config.",
      },
    };
    installFakeTauri(state);
    await openClaude();

    await noteTitled(
      "Gate Connect can’t read Claude Code’s config",
      "Gate Connect could not read Claude Code's config.",
    );
  });

  it("titles a tool that could not be put back on its own model", async () => {
    const state = onGateModels();
    state.toolModels.problem = {
      "claude-code": {
        state: "drifted",
        message: "Claude Code was moved off its Gate models, so its requests are refused.",
      },
    };
    installFakeTauri(state);
    await openClaude();

    await noteTitled("Claude Code’s requests are being refused", "its requests are refused");
  });

  it("draws nothing while the model is healthy", async () => {
    installFakeTauri(onGateModels());
    await openClaude();

    await screen.findByText("Model selection");
    expect(screen.queryByText("Pay-as-you-go is off")).toBeNull();
    expect(screen.queryByText(/requests are/)).toBeNull();
  });

  it("names the product in the back-on-App-default notice, as the titles do", async () => {
    const state = onGateModels();
    state.toolModels.left = { "claude-code": "claude-sonnet-5" };
    installFakeTauri(state);
    await openClaude();

    expect(
      await screen.findByText(
        "You switched Claude Code to claude-sonnet-5 in Claude Code, so it is back on App default.",
      ),
    ).toBeTruthy();
  });
});
