import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, render, screen, fireEvent } from "@testing-library/react";
import type { ProviderState, Tool, ProxyDomain } from "../lib/api";
import { buildGroups } from "../lib/groups";
import { Routes } from "./Routes";

function makeTool(
  slug: string,
  name: string,
  status: Tool["status"],
  upstream = "Anthropic",
): Tool {
  return {
    slug,
    name,
    upstream_provider_name: upstream,
    default_upstream_url: "https://api.anthropic.com",
    requires_upstream_credential: false,
    status,
  };
}

function makeDomain(overrides: Partial<ProxyDomain> = {}): ProxyDomain {
  return {
    slug: "anthropic",
    display_name: "Claude Desktop / Cowork",
    hosts: ["api.anthropic.com"],
    upstream_url: "https://api.anthropic.com",
    rewrite_prefixes: ["/v1/messages"],
    passthrough_prefixes: [],
    enabled: true,
    supported: true,
    ...overrides,
  };
}

/** Mirrors the real catalog: Claude Code and Codex are claimed; OpenCode and
 * OpenClaw deliberately are not, so they land in "Other tools". */
const CATALOG: ProviderState[] = [
  {
    slug: "anthropic",
    display_name: "Claude",
    subtitle: "",
    enabled: false,
    available: true,
    tool_slugs: ["claude-code"],
    domain_slugs: ["anthropic"],
  },
  {
    slug: "openai",
    display_name: "OpenAI",
    subtitle: "",
    enabled: false,
    available: true,
    tool_slugs: ["codex"],
    domain_slugs: ["openai"],
  },
];

function renderRoutes(
  {
    tools = [] as Tool[],
    domains = [] as ProxyDomain[],
    proxyOn = true,
    caTrusted = true,
  } = {},
  props: Partial<React.ComponentProps<typeof Routes>> = {},
) {
  const groups = buildGroups(CATALOG, tools, domains, { proxyOn, caTrusted });
  render(
    <Routes
      groups={groups}
      busy={false}
      onBack={vi.fn()}
      onToggleGroup={vi.fn()}
      onToggleTool={vi.fn()}
      onSetDomain={vi.fn()}
      onTrustCa={vi.fn()}
      proxyOn={proxyOn}
      onEnableRouting={vi.fn()}
      envExportSeparable={true}
      envExportOn={true}
      onToggleEnvExport={vi.fn()}
      {...props}
    />,
  );
  return groups;
}

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

describe("Routes ledger", () => {
  it("collapses tools and apps into one row per family", () => {
    renderRoutes({
      tools: [
        makeTool("claude-code", "Claude Code", { kind: "connected" }),
        makeTool("hermes", "Hermes", { kind: "not_installed" }),
        makeTool("codex", "Codex", { kind: "detected" }, "OpenAI"),
      ],
      domains: [makeDomain()],
    });
    // Families, not tool rows; not_installed stays out entirely.
    expect(screen.getByText("Claude")).toBeTruthy();
    expect(screen.getByText("OpenAI")).toBeTruthy();
    expect(screen.queryByText("Claude Code")).toBeNull();
    expect(screen.queryByText("Hermes")).toBeNull();
    // Twice each: the visible sub-line, and the sr-only description the stretch
    // button points at (the visible copy truncates at 360px).
    expect(screen.getAllByText(/2 of 2 routing/)).toHaveLength(2);
    expect(screen.getAllByText(/0 of 1 routing/)).toHaveLength(2);
  });

  it("gives the multi-provider tools an honest home, not a wrong family", () => {
    renderRoutes({
      // The real backend calls their upstream "your existing providers".
      tools: [
        makeTool("opencode", "OpenCode", { kind: "detected" }, "your existing providers"),
        makeTool("openclaw", "OpenClaw", { kind: "detected" }, "your existing providers"),
      ],
    });
    expect(screen.getByText("Other tools")).toBeTruthy();
    expect(screen.queryByText(/existing providers/)).toBeNull();
  });

  it("reports a partly-routed family without rounding up", () => {
    renderRoutes({
      tools: [makeTool("claude-code", "Claude Code", { kind: "connected" })],
      // Its sibling app row is switched off, so the family is half on.
      domains: [makeDomain({ enabled: false })],
    });
    expect(screen.getByText("Partly routed")).toBeTruthy();
    expect(screen.getByText("1 of 2 routing")).toBeTruthy();
  });

  it("names an exception in the sub-line instead of hijacking the family pill", () => {
    renderRoutes({
      tools: [makeTool("claude-code", "Claude Code", { kind: "error", message: "bad json" })],
      domains: [makeDomain()],
    });
    // The family pill still answers "is this routing?": the app row beside the
    // failed tool is carrying traffic, so the family reads partly routed and the
    // failure is named below rather than taking the pill.
    expect(document.getElementById("group-desc-anthropic")?.textContent).toContain(
      "Partly routed",
    );
    expect(screen.getAllByText(/Claude Code failed/).length).toBeGreaterThan(0);
  });

  it("says Error, not Not routed, when a family is dark because a tool failed", () => {
    renderRoutes({
      tools: [makeTool("claude-code", "Claude Code", { kind: "error", message: "bad json" })],
    });
    // Grey "Not routed" is what this pill says for a switch the user set; a
    // failure must not borrow it.
    expect(screen.getByText("Error")).toBeTruthy();
    expect(screen.queryByText("Not routed")).toBeNull();
  });

  it("gives a failure its own ink, not the grey every exception used to share", () => {
    renderRoutes({
      tools: [makeTool("claude-code", "Claude Code", { kind: "error", message: "bad json" })],
      domains: [makeDomain()],
    });
    // The pill was reality's only voice on a row whose switch reports intent in
    // saturated indigo. The sentence carries severity too now.
    expect(screen.getByText("Claude Code failed").className).toContain("text-gc-error-deep");
  });

  it("keeps a hand-written setup in the quieter ink", () => {
    renderRoutes({
      tools: [makeTool("codex", "Codex", { kind: "drifted", reason: "r" }, "OpenAI")],
    });
    const drift = screen.getByText("Codex set up elsewhere");
    expect(drift.className).toContain("text-gc-ink-2");
    expect(drift.className).not.toContain("error");
  });

  it("routes a whole family with one flip", () => {
    const onToggleGroup = vi.fn();
    renderRoutes(
      { tools: [makeTool("claude-code", "Claude Code", { kind: "detected" })] },
      { onToggleGroup },
    );
    fireEvent.click(screen.getByRole("switch", { name: "Route Claude through Gate" }));
    expect(onToggleGroup).toHaveBeenCalledWith("anthropic", true);
  });

  it("expands the family in place instead of pushing a screen", () => {
    renderRoutes({
      tools: [makeTool("claude-code", "Claude Code", { kind: "connected" })],
      domains: [makeDomain()],
    });
    const row = screen.getByRole("button", { name: "Claude details" });
    expect(row.getAttribute("aria-expanded")).toBe("false");
    // Members are not rendered until the family is opened.
    expect(screen.queryByText("Claude Desktop / Cowork")).toBeNull();

    fireEvent.click(row);
    expect(row.getAttribute("aria-expanded")).toBe("true");
    // The member list, and the mechanism only this level shows.
    expect(screen.getByText("Claude Desktop / Cowork")).toBeTruthy();
    expect(screen.getByText("config file")).toBeTruthy();
    // Still inside the same row, so the panel never changed.
    expect(row.closest('[role="listitem"]')!.textContent).toContain(
      "Claude Desktop / Cowork",
    );
  });

  it("collapses the open family when another one opens", () => {
    renderRoutes({
      tools: [
        makeTool("claude-code", "Claude Code", { kind: "connected" }),
        makeTool("codex", "Codex", { kind: "connected" }, "OpenAI"),
      ],
    });
    fireEvent.click(screen.getByRole("button", { name: "Claude details" }));
    expect(screen.getByText("Claude Code")).toBeTruthy();
    // Two open families in a 360px panel put the second one's members below the
    // fold with no way to see both.
    fireEvent.click(screen.getByRole("button", { name: "OpenAI details" }));
    expect(screen.queryByText("Claude Code")).toBeNull();
    expect(screen.getByText("Codex")).toBeTruthy();
  });

  it("closes the family again on a second click", () => {
    renderRoutes({
      tools: [makeTool("claude-code", "Claude Code", { kind: "connected" })],
      domains: [makeDomain()],
    });
    const row = screen.getByRole("button", { name: "Claude details" });
    fireEvent.click(row);
    fireEvent.click(row);
    expect(row.getAttribute("aria-expanded")).toBe("false");
    expect(screen.queryByText("Claude Desktop / Cowork")).toBeNull();
  });

  it("returns to Home from its own header", () => {
    const onBack = vi.fn();
    renderRoutes({ tools: [makeTool("claude-code", "Claude Code", { kind: "connected" })] }, { onBack });
    fireEvent.click(screen.getByRole("button", { name: "Back" }));
    expect(onBack).toHaveBeenCalledTimes(1);
  });

  it("titles the panel with the question its rows answer", () => {
    renderRoutes({ tools: [makeTool("claude-code", "Claude Code", { kind: "connected" })] });
    // The same words as the door on Home, so the navigation reads as one move
    // rather than two unrelated labels.
    expect(
      screen.getByRole("heading", { name: "What routes through Gate" }),
    ).toBeTruthy();
  });

  it("does not dead-end when the last tool is uninstalled while it is open", () => {
    // Reopening the popover re-reads the ledger, so this is reachable.
    renderRoutes();
    expect(screen.getByText(/Nothing is installed to route/)).toBeTruthy();
    expect(screen.queryByRole("list")).toBeNull();
  });
});

describe("Routes accessibility", () => {
  it("gives the row's whole sentence to the drill-in button, pill state included", () => {
    renderRoutes({
      tools: [makeTool("claude-code", "Claude Code", { kind: "error", message: "bad json" })],
      domains: [makeDomain()],
    });
    const row = screen.getByRole("button", { name: "Claude details" });
    const described = document.getElementById(row.getAttribute("aria-describedby")!);
    // Without this the row announced "Claude details, button" and nothing else:
    // the count, the pill and the failure were all in pointer-events-none spans
    // under the stretch button.
    expect(described?.textContent).toContain("Partly routed");
    expect(described?.textContent).toContain("1 of 2 routing");
    expect(described?.textContent).toContain("Claude Code failed");
  });

  it("exposes the ledger as a list", () => {
    renderRoutes({ tools: [makeTool("claude-code", "Claude Code", { kind: "connected" })] });
    expect(screen.getByRole("list")).toBeTruthy();
    expect(screen.getAllByRole("listitem").length).toBeGreaterThan(0);
  });

  it("announces the family's new state after a flip", () => {
    // A row's pill and sub-line both change on a flip and neither announces: a
    // description is read when focus arrives, not when it changes.
    renderRoutes({ tools: [makeTool("claude-code", "Claude Code", { kind: "connected" })] });
    const live = document.querySelector('[aria-live="polite"]')!;
    expect(live.textContent).toBe("");
    fireEvent.click(screen.getByRole("switch", { name: "Route Claude through Gate" }));
    expect(live.textContent).toContain("Claude");
    expect(live.textContent).toContain("Routed");
  });
});

describe("Routes command-line tools switch", () => {
  const NAME = "Route command-line tools through Gate";

  it("toggles the shell-environment channel without touching any family", () => {
    const onToggleEnvExport = vi.fn();
    const onToggleGroup = vi.fn();
    renderRoutes({ tools: [makeTool("claude-code", "Claude Code", { kind: "connected" })] }, {
      onToggleEnvExport,
      onToggleGroup,
    });
    fireEvent.click(screen.getByRole("switch", { name: NAME }));
    expect(onToggleEnvExport).toHaveBeenCalledTimes(1);
    // It spans every family, so it must never flip one as a side effect.
    expect(onToggleGroup).not.toHaveBeenCalled();
  });

  it("reflects the backend's choice rather than the master's state", () => {
    renderRoutes({ proxyOn: true }, { envExportOn: false });
    expect(screen.getByRole("switch", { name: NAME }).getAttribute("aria-checked")).toBe("false");
  });

  it("is absent where the channel cannot be separated from routing", () => {
    // Linux: the environment.d drop-in *is* the system proxy, so a switch here
    // could not honour itself. Better no control than one that lies.
    renderRoutes({}, { envExportSeparable: false });
    expect(screen.queryByRole("switch", { name: NAME })).toBeNull();
  });

  it("renders after the families, not among them", () => {
    renderRoutes({ tools: [makeTool("claude-code", "Claude Code", { kind: "connected" })] });
    const family = screen.getByRole("switch", { name: "Route Claude through Gate" });
    const sub = screen.getByRole("switch", { name: NAME });
    expect(family.compareDocumentPosition(sub) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    // Not a row: it belongs to no family, so it must sit outside the list.
    expect(sub.closest("[role='listitem']")).toBeNull();
  });

  it("carries its instruction in ink that clears AA", () => {
    // ink-4 measured 3.97:1 on white, the only text in the app below 4.5:1,
    // and this is two sentences about a machine-wide change to git and curl.
    renderRoutes();
    const copy = screen.getByText(/for your whole/);
    expect(copy.className).toContain("text-gc-ink-3");
    expect(copy.className).not.toContain("text-gc-ink-4");
  });
});

describe("Routes initial expansion", () => {
  it("arrives with the family the Home row named already open", () => {
    renderRoutes(
      { tools: [makeTool("claude-code", "Claude Code", { kind: "connected" })] },
      { initialOpen: "anthropic" },
    );
    expect(
      screen.getByRole("button", { name: "Claude details" }).getAttribute("aria-expanded"),
    ).toBe("true");
  });

  it("stays collapsed when the user came from the heading", () => {
    renderRoutes({ tools: [makeTool("claude-code", "Claude Code", { kind: "connected" })] });
    expect(
      screen.getByRole("button", { name: "Claude details" }).getAttribute("aria-expanded"),
    ).toBe("false");
  });
});
