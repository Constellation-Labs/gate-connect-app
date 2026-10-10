import { afterEach, describe, expect, it } from "vitest";
import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
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
/** Feed reads answered so far, keyed by the scope asked for. */
const answered = new Map<string, number>();

function answerFeed(answer: (tools: string[]) => string) {
  answered.clear();
  const internals = (window as any).__TAURI_INTERNALS__;
  const original = internals.invoke;
  internals.invoke = (cmd: string, args: Record<string, unknown> = {}) => {
    const recorded = original(cmd, args);
    if (cmd !== "activity_tool_events") return recorded;
    recorded.catch(() => {});
    const tools = args.tools as string[];
    const key = tools.join(",");
    return Promise.resolve(answer(tools)).finally(() =>
      answered.set(key, (answered.get(key) ?? 0) + 1),
    );
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
      await screen.findByText(/Recent requests on Gate models failed/),
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

    // Both reads answered, and React has drawn what they said.
    await waitFor(() => {
      expect(answered.get("claude-code")).toBeGreaterThan(0);
      expect(answered.get(CLAUDE.join(","))).toBeGreaterThan(0);
    });
    await act(async () => {
      await Promise.resolve();
    });
    expect(screen.queryByText(/Recent requests on Gate models failed/)).toBeNull();
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

    // Another section's sender does not. The listener handles reports in
    // order, so a Claude report after it is the sync point: exactly one new
    // read means the Codex one asked for nothing.
    const settled = callsTo("activity_tool_events").length;
    window.__GATE_E2E__.emit("traffic-observed", ["codex"]);
    window.__GATE_E2E__.emit("traffic-observed", ["claude-desktop"]);
    await waitFor(() =>
      expect(callsTo("activity_tool_events").length).toBeGreaterThan(settled),
    );
    expect(callsTo("activity_tool_events").length).toBe(settled + 1);
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

describe("the tray's section row draws the section's figures", () => {
  /** An overview body whose message counter is `messages`. */
  function overview(messages: number) {
    return JSON.stringify({
      generatedAt: new Date().toISOString(),
      window: { from: "2026-09-30T00:00:00.000Z", to: "2026-10-01T00:00:00.000Z" },
      org: { orgId: "org-1", name: "Constellation Labs" },
      counters: {
        blockedOrFlagged: { state: "ok", value: 0 },
        needsReview: { state: "ok", value: 0 },
        requestsRouted: { state: "ok", value: messages },
        tokensSaved: { state: "ok", fraction: 0, amount: 0, currency: "USD" },
      },
      requestsByHour: { state: "ok", buckets: [] },
      policies: { state: "ok", rows: [] },
      tokenSavings: { state: "ok", rows: [] },
    });
  }
  const alert = (id: string, tool: string | null) => ({
    id,
    requestId: `req-${id}`,
    at: "2026-09-30T23:00:00Z",
    action: "block" as const,
    category: "credential",
    tool,
    model: "claude-opus-4",
    provider: "anthropic",
  });

  it("counts the whole section's messages and alerts on the Claude row", async () => {
    const state = defaultState();
    state.windowLabel = "tray";
    state.installations = {
      installations: [
        { installId: "m-1", label: "m-1", current: true, lastSeenAt: "", requests: 1 },
      ],
      current: "m-1",
    };
    state.securityFeed = {
      state: "live",
      events: [
        alert("1", "claude-code"),
        alert("2", "claude-web"),
        alert("3", "claude-desktop"),
        alert("4", "codex"),
        alert("5", null),
      ],
    };
    installFakeTauri(state);
    // The section's set reads 7; the CLI alone would read 1. Every other row
    // answers 0 so nothing else is waiting on a read.
    const internals = (window as any).__TAURI_INTERNALS__;
    const original = internals.invoke;
    internals.invoke = (cmd: string, args: Record<string, unknown> = {}) => {
      const recorded = original(cmd, args);
      if (cmd === "activity_cached_tool_overviews") {
        recorded.catch(() => {});
        return Promise.resolve({});
      }
      if (cmd !== "activity_overview") return recorded;
      recorded.catch(() => {});
      const tools = (args.tools as string[] | undefined) ?? [];
      const key = tools.join(",");
      return Promise.resolve(
        overview(key === CLAUDE.join(",") ? 7 : key === "claude-code" ? 1 : 0),
      );
    };
    render(<TrayApp />);

    const row = (await screen.findByRole("switch", { name: "Claude" })).closest("li")!;
    await waitFor(() => expect(row.textContent).toContain("7 messages"));
    // Claude Code, claude.ai and the desktop app; not Codex's, not the
    // unattributed one.
    expect(row.textContent).toContain("3 alerts");
  });
});
