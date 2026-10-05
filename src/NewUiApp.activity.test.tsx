import { afterEach, describe, expect, it } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { defaultState } from "../e2e/backend";
import { installFakeTauri } from "../e2e/install";
import { NewUiApp } from "./NewUiApp";
import { TrayApp } from "./TrayApp";

/**
 * Which senders an app pane reads its activity for. The hooks are tested next
 * door; this pins that the pane hands them the section's names rather than its
 * one config tool, on the fake backend the e2e suite uses. The activity
 * commands are not in its table, so they reject - which is fine: only the
 * arguments are under test.
 */
afterEach(cleanup);

const CLAUDE = ["claude-code", "claude-desktop", "claude-web"];
const CHATGPT = ["chatgpt", "chatgpt-web", "codex"];

/** One feed page whose rows all carry `status`. */
function page(status: "success" | "error") {
  return JSON.stringify({
    generatedAt: "2026-10-01T00:00:00.000Z",
    window: { from: "2026-09-30T00:00:00.000Z", to: "2026-10-01T00:00:00.000Z" },
    toolScope: { tool: null, tools: [] },
    events: ["a", "b", "c", "d", "e"].map((id) => ({
      requestId: `${status}-${id}`,
      at: "2026-09-30T23:00:00.000Z",
      status,
      model: "claude-opus-4",
    })),
    nextCursor: null,
  });
}

/**
 * Answer the feed by scope: the fake backend has no handler for it. The call is
 * still passed through so it is recorded.
 */
function answerFeed(answer: (tools: string[]) => string) {
  const internals = (window as any).__TAURI_INTERNALS__;
  const original = internals.invoke;
  internals.invoke = (cmd: string, args: Record<string, unknown> = {}) => {
    const recorded = original(cmd, args);
    if (cmd !== "activity_tool_events") return recorded;
    recorded.catch(() => {});
    return Promise.resolve(answer(args.tools as string[]));
  };
}

function boot(
  edit: (s: ReturnType<typeof defaultState>) => void = () => {},
  feed?: (tools: string[]) => string,
  Shell: () => JSX.Element | null = NewUiApp,
) {
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
  if (feed) answerFeed(feed);
  render(<Shell />);
  return state;
}

const callsTo = (cmd: string) =>
  window.__GATE_E2E__.calls.filter((c) => c.cmd === cmd).map((c) => c.args);

async function openApp(name: RegExp) {
  const row = await screen.findAllByRole("button", { name });
  fireEvent.click(row[0]);
}
const openClaude = () => openApp(/^Claude/);

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

  it("asks for Codex, the desktop app and chatgpt.com together", async () => {
    boot();
    await openApp(/^ChatGPT/);

    await waitFor(() =>
      expect(callsTo("activity_tool_events")).toContainEqual(
        expect.objectContaining({ tools: CHATGPT, installId: "m-1" }),
      ),
    );
    expect(callsTo("activity_overview")).toContainEqual(
      expect.objectContaining({ tools: CHATGPT }),
    );
  });

  it("builds the Gate model warning from the tool's own rows, not the section's", async () => {
    // The section feed answers all successes and the tool alone all errors:
    // the warning has to follow the second. Read off the section, the
    // successes would hide it.
    boot(
      (s) => {
        s.toolModels.choices["claude-code"] = { source: "gate", model_ids: ["claude-opus-4"] };
      },
      (tools) => page(tools.length === 1 ? "error" : "success"),
    );
    await openClaude();

    expect(
      await screen.findByText(/The last few requests from this app have failed/),
    ).toBeTruthy();
  });

  it("does not let section errors raise the Gate model warning", async () => {
    boot(
      (s) => {
        s.toolModels.choices["claude-code"] = { source: "gate", model_ids: ["claude-opus-4"] };
      },
      (tools) => page(tools.length === 1 ? "success" : "error"),
    );
    await openClaude();

    await waitFor(() =>
      expect(callsTo("activity_tool_events")).toContainEqual(
        expect.objectContaining({ tools: ["claude-code"] }),
      ),
    );
    await new Promise((r) => setTimeout(r, 50));
    expect(screen.queryByText(/The last few requests from this app have failed/)).toBeNull();
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

describe("the tray's section rows read what the panes read", () => {
  it("asks for each section's set, and not its config tool on its own", async () => {
    boot(
      (s) => {
        s.windowLabel = "tray";
      },
      undefined,
      TrayApp,
    );

    await waitFor(() =>
      expect(callsTo("activity_overview")).toContainEqual(
        expect.objectContaining({ tools: CLAUDE, installId: "m-1" }),
      ),
    );
    await waitFor(() =>
      expect(callsTo("activity_overview")).toContainEqual(
        expect.objectContaining({ tools: CHATGPT }),
      ),
    );
    const trayReads = callsTo("activity_overview").map((a) => a.tools);
    expect(trayReads).toContainEqual(["opencode"]);
    expect(trayReads).not.toContainEqual(["claude-code"]);
    expect(trayReads).not.toContainEqual(["codex"]);
  });
});
