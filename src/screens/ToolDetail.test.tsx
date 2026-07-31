import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, render, screen, fireEvent, waitFor } from "@testing-library/react";
import type { Tool } from "../lib/api";
import { ToolDetail } from "./ToolDetail";

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

function makeTool(status: Tool["status"]): Tool {
  return {
    slug: "claude_code",
    name: "Claude Code",
    upstream_provider_name: "Anthropic",
    default_upstream_url: "https://api.anthropic.com",
    requires_upstream_credential: false,
    status,
  };
}

function renderDetail(status: Tool["status"], props: Partial<React.ComponentProps<typeof ToolDetail>> = {}) {
  render(
    <ToolDetail
      tool={makeTool(status)}
      busy={false}
      onSetRouted={vi.fn(() => Promise.resolve())}
      onBack={vi.fn()}
      {...props}
    />,
  );
}

describe("ToolDetail switch", () => {
  it("reflects a connected tool as on and disconnects on toggle", async () => {
    const onSetRouted = vi.fn(() => Promise.resolve());
    renderDetail({ kind: "connected" }, { onSetRouted });
    const sw = screen.getByRole("switch", { name: "Route Claude Code through Gate" });
    expect(sw.getAttribute("aria-checked")).toBe("true");
    fireEvent.click(sw);
    await waitFor(() => expect(onSetRouted).toHaveBeenCalledWith(false));
  });

  it("connects a detected tool on toggle and shows the restart hint", async () => {
    const onSetRouted = vi.fn(() => Promise.resolve());
    renderDetail({ kind: "detected" }, { onSetRouted });
    const sw = screen.getByRole("switch");
    expect(sw.getAttribute("aria-checked")).toBe("false");
    fireEvent.click(sw);
    await waitFor(() => expect(onSetRouted).toHaveBeenCalledWith(true));
    expect(await screen.findByText(/Restart Claude Code/)).toBeTruthy();
  });

  it("shows a classified error when the toggle fails, with the raw detail", async () => {
    const onSetRouted = vi.fn(() =>
      Promise.reject("No upstream Anthropic credential saved. Add one before connecting."),
    );
    renderDetail({ kind: "detected" }, { onSetRouted });
    fireEvent.click(screen.getByRole("switch"));
    expect(await screen.findByText("Couldn't connect this tool")).toBeTruthy();
    expect(screen.queryByText(/Restart Claude Code/)).toBeNull();
  });

  it("marks the switch busy without ejecting keyboard focus", () => {
    const onSetRouted = vi.fn(() => Promise.resolve());
    renderDetail({ kind: "connected" }, { busy: true, onSetRouted });
    const sw = screen.getByRole("switch");
    expect(sw.getAttribute("aria-busy")).toBe("true");
    expect(sw).toHaveProperty("disabled", false);
    fireEvent.click(sw);
    expect(onSetRouted).not.toHaveBeenCalled();
  });
});

describe("ToolDetail status body", () => {
  it("shows the full untruncated error message", () => {
    renderDetail({ kind: "error", message: "failed to parse ~/.claude/settings.json: unexpected EOF" });
    expect(screen.getByText(/unexpected EOF/)).toBeTruthy();
  });

  it("shows a drifted setup's reason and adoption copy", () => {
    renderDetail({ kind: "drifted", reason: "env_key GATE_API_KEY present" });
    expect(screen.getByText(/env_key GATE_API_KEY present/)).toBeTruthy();
    expect(screen.getByText(/Turning the switch on replaces that configuration/)).toBeTruthy();
  });
});
