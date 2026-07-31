import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, render, screen, fireEvent } from "@testing-library/react";
import type { Mock } from "vitest";
import type { Platform } from "../lib/platform";
import type { Tool, ProxyDomain } from "../lib/api";
import { Home } from "./Home";

// The CA-trust card swaps the trust-store name by platform; drive it by
// mocking usePlatform rather than the async Tauri lookup.
vi.mock("../lib/platform", () => ({ usePlatform: vi.fn() }));
import { usePlatform } from "../lib/platform";

// Home reads launch-at-login state for its keep-routing tip; a pending
// promise keeps the tip hidden and avoids post-test state updates.
vi.mock("../lib/api", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../lib/api")>();
  return {
    ...actual,
    launchAtLoginStatus: vi.fn(() => new Promise(() => {})),
  };
});

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

function renderHome(props: Partial<React.ComponentProps<typeof Home>> = {}, platform: Platform = "macos") {
  (usePlatform as Mock).mockReturnValue(platform);
  render(
    <Home
      workspace="gateway.constellationgate.ai"
      proxyOn={true}
      caTrusted={true}
      showProxy={true}
      tools={[]}
      domains={[]}
      busy={false}
      error={null}
      restartHint={false}
      onDismissRestartHint={vi.fn()}
      relaunchHint={false}
      onDismissRelaunchHint={vi.fn()}
      startupRoutingHint={false}
      onDismissStartupRoutingHint={vi.fn()}
      onCloseAgents={vi.fn()}
      staleAgentsHint={false}
      onDismissStaleAgents={vi.fn()}
      onToggleProxy={vi.fn()}
      onTrustCa={vi.fn()}
      onToggleGroup={vi.fn()}
      onOpenGroup={vi.fn()}
      onOpenSettings={vi.fn()}
      {...props}
    />,
  );
}

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

describe("Home CA-trust card", () => {
  it("appears when routing is on with the CA untrusted, naming the keychain on macOS", () => {
    renderHome({ caTrusted: false, domains: [makeDomain()] });
    expect(screen.getByText(/certificate your keychain trusts/)).toBeTruthy();
    expect(screen.getByText(/created on this machine/)).toBeTruthy();
  });

  it("names the certificate store on Windows", () => {
    renderHome({ caTrusted: false, domains: [makeDomain()] }, "windows");
    expect(screen.getByText(/certificate your certificate store trusts/)).toBeTruthy();
  });

  it("calls onTrustCa from the accent button", () => {
    const onTrustCa = vi.fn();
    renderHome({ caTrusted: false, domains: [makeDomain()], onTrustCa });
    fireEvent.click(screen.getByText("Trust certificate"));
    expect(onTrustCa).toHaveBeenCalledTimes(1);
  });

  it("is absent while routing is off or the CA is trusted", () => {
    renderHome({ proxyOn: false, caTrusted: false, domains: [makeDomain()] });
    expect(screen.queryByText("Trust certificate")).toBeNull();
    cleanup();
    renderHome({ caTrusted: true });
    expect(screen.queryByText("Trust certificate")).toBeNull();
  });
});

describe("Home master toggle", () => {
  it("calls onToggleProxy", () => {
    const onToggleProxy = vi.fn();
    renderHome({ onToggleProxy });
    fireEvent.click(screen.getByRole("switch", { name: "Route through Gate" }));
    expect(onToggleProxy).toHaveBeenCalledTimes(1);
  });

  it("says the certificate blocks coverage when partial", () => {
    renderHome({ caTrusted: false, domains: [makeDomain()] });
    expect(screen.getByText("On · certificate not trusted yet")).toBeTruthy();
  });

  it("counts what is routing against everything routable", () => {
    renderHome({
      tools: [
        makeTool("claude-code", "Claude Code", { kind: "connected" }),
        makeTool("opencode", "OpenCode", { kind: "detected" }),
        makeTool("hermes", "Hermes", { kind: "not_installed" }),
      ],
      // One enabled app row plus one available-but-off row.
      domains: [makeDomain(), makeDomain({ slug: "openai", display_name: "OpenAI apps", enabled: false })],
    });
    // 1 routed tool + 1 routed app, out of 2 installed tools + 2 domains.
    expect(screen.getByText("On · 2 of 4 routing")).toBeTruthy();
  });
});

describe("Home model-family ledger", () => {
  it("collapses tools and apps into one row per family", () => {
    renderHome({
      tools: [
        makeTool("claude-code", "Claude Code", { kind: "connected" }),
        makeTool("opencode", "OpenCode", { kind: "detected" }),
        makeTool("hermes", "Hermes", { kind: "not_installed" }),
        makeTool("codex", "Codex", { kind: "detected" }, "OpenAI"),
      ],
      domains: [makeDomain()],
    });
    // Two families, not four tool rows; not_installed stays out.
    expect(screen.getByText("Claude")).toBeTruthy();
    expect(screen.getByText("OpenAI")).toBeTruthy();
    expect(screen.queryByText("Claude Code")).toBeNull();
    expect(screen.queryByText("Hermes")).toBeNull();
    // Claude: Claude Code + Cowork routing, OpenCode not.
    expect(screen.getByText(/2 of 3 routing/)).toBeTruthy();
    expect(screen.getByText(/0 of 1 routing/)).toBeTruthy();
  });

  it("reports a partly-routed family without rounding up", () => {
    renderHome({
      tools: [
        makeTool("claude-code", "Claude Code", { kind: "connected" }),
        makeTool("opencode", "OpenCode", { kind: "detected" }),
      ],
    });
    expect(screen.getByText("Partly routed")).toBeTruthy();
  });

  it("names an exception in the sub-line instead of hijacking the pill", () => {
    renderHome({
      tools: [
        makeTool("claude-code", "Claude Code", { kind: "connected" }),
        makeTool("openclaw", "OpenClaw", { kind: "error", message: "bad json" }),
      ],
    });
    // The pill still answers "is this routing?"; the failure is named below.
    expect(screen.getByText("Partly routed")).toBeTruthy();
    expect(screen.getByText(/OpenClaw failed/)).toBeTruthy();
  });

  it("flags a hand-written setup without calling the family broken", () => {
    renderHome({
      tools: [makeTool("codex", "Codex", { kind: "drifted", reason: "r" }, "OpenAI")],
    });
    expect(screen.getByText(/Codex set up elsewhere/)).toBeTruthy();
  });

  it("routes a whole family with one flip", () => {
    const onToggleGroup = vi.fn();
    renderHome({
      tools: [makeTool("opencode", "OpenCode", { kind: "detected" })],
      onToggleGroup,
    });
    fireEvent.click(screen.getByRole("switch", { name: "Route Claude through Gate" }));
    expect(onToggleGroup).toHaveBeenCalledWith("anthropic", true);
  });

  it("opens the family detail from the row body", () => {
    const onOpenGroup = vi.fn();
    renderHome({
      tools: [makeTool("claude-code", "Claude Code", { kind: "connected" })],
      onOpenGroup,
    });
    fireEvent.click(screen.getByRole("button", { name: "Claude details" }));
    expect(onOpenGroup).toHaveBeenCalledWith("anthropic");
  });
});
