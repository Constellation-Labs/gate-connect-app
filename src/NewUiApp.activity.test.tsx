import { afterEach, describe, expect, it } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { defaultState } from "../e2e/backend";
import { installFakeTauri } from "../e2e/install";
import { NewUiApp } from "./NewUiApp";

/**
 * Which senders an app pane reads its activity for. The hooks are tested next
 * door; this pins that the pane hands them the section's names rather than its
 * one config tool, on the fake backend the e2e suite uses. The activity
 * commands are not in its table, so they reject - which is fine: only the
 * arguments are under test.
 */
afterEach(cleanup);

const CLAUDE = ["claude-code", "claude-desktop", "claude-web"];

function boot(edit: (s: ReturnType<typeof defaultState>) => void = () => {}) {
  const state = defaultState();
  // A machine the gateway knows, so the per-machine reads fire at all.
  state.installations = {
    installations: [
      {
        installId: "m-1",
        label: "m-1",
        current: true,
        lastSeenAt: "2026-10-01T00:00:00.000Z",
        requests: 1,
      },
    ],
    current: "m-1",
  };
  edit(state);
  installFakeTauri(state);
  render(<NewUiApp />);
  return state;
}

const callsTo = (cmd: string) =>
  window.__GATE_E2E__.calls.filter((c) => c.cmd === cmd).map((c) => c.args);

async function openClaude() {
  const row = await screen.findAllByRole("button", { name: /^Claude/ });
  fireEvent.click(row[0]);
}

describe("an app pane reads its whole section", () => {
  it("asks for Claude Code, the desktop app and claude.ai together", async () => {
    boot();
    await openClaude();

    await waitFor(() =>
      expect(callsTo("activity_tool_events")).toContainEqual(
        expect.objectContaining({ tools: CLAUDE, installId: "m-1" }),
      ),
    );
    expect(callsTo("activity_overview")).toContainEqual(
      expect.objectContaining({ tools: CLAUDE, installId: "m-1" }),
    );
    // No Gate model, so no tool-only read for the warning.
    expect(callsTo("activity_tool_events")).not.toContainEqual(
      expect.objectContaining({ tools: ["claude-code"] }),
    );
  });

  it("reads the section even with no config tool installed", async () => {
    boot((s) => {
      s.tools = s.tools.map((t) =>
        t.slug === "claude-code" ? { ...t, status: { kind: "not_installed" } } : t,
      );
    });
    await openClaude();

    await waitFor(() =>
      expect(callsTo("activity_tool_events")).toContainEqual(
        expect.objectContaining({ tools: CLAUDE }),
      ),
    );
    expect(screen.queryByText("Shows in the Overview, not per app")).toBeNull();
  });

  it("reads the config tool alone for the Gate model warning", async () => {
    boot((s) => {
      s.toolModels.choices["claude-code"] = { source: "gate", model_ids: ["claude-opus-4"] };
    });
    await openClaude();

    await waitFor(() =>
      expect(callsTo("activity_tool_events")).toContainEqual(
        expect.objectContaining({ tools: ["claude-code"], installId: "m-1" }),
      ),
    );
  });

  it("re-reads when any of the section's senders sent traffic", async () => {
    boot();
    await openClaude();
    await waitFor(() => expect(callsTo("activity_tool_events").length).toBeGreaterThan(0));
    const before = callsTo("activity_tool_events").length;

    window.__GATE_E2E__.emit("traffic-observed", ["claude-web"]);
    await waitFor(() =>
      expect(callsTo("activity_tool_events").length).toBeGreaterThan(before),
    );
    expect(callsTo("activity_tool_events").at(-1)).toEqual(
      expect.objectContaining({ tools: CLAUDE }),
    );

    // Another section's sender does not.
    const settled = callsTo("activity_tool_events").length;
    window.__GATE_E2E__.emit("traffic-observed", ["codex"]);
    await new Promise((r) => setTimeout(r, 50));
    expect(callsTo("activity_tool_events").length).toBe(settled);
  });
});
