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

function makeTool(slug: string, name: string, status: Tool["status"]): Tool {
  return {
    slug,
    name,
    upstream_provider_name: "Anthropic",
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
      onToggleTool={vi.fn()}
      onSetDomain={vi.fn()}
      onOpenTool={vi.fn()}
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
    renderHome({ caTrusted: false });
    expect(screen.getByText(/certificate your keychain trusts/)).toBeTruthy();
    expect(screen.getByText(/created on this machine/)).toBeTruthy();
  });

  it("names the certificate store on Windows", () => {
    renderHome({ caTrusted: false }, "windows");
    expect(screen.getByText(/certificate your certificate store trusts/)).toBeTruthy();
  });

  it("calls onTrustCa from the accent button", () => {
    const onTrustCa = vi.fn();
    renderHome({ caTrusted: false, onTrustCa });
    fireEvent.click(screen.getByText("Trust certificate"));
    expect(onTrustCa).toHaveBeenCalledTimes(1);
  });

  it("is absent while routing is off or the CA is trusted", () => {
    renderHome({ proxyOn: false, caTrusted: false });
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
});

describe("Home tools ledger", () => {
  it("renders a row per installed tool with its pill, hiding not_installed", () => {
    renderHome({
      tools: [
        makeTool("claude-code", "Claude Code", { kind: "connected" }),
        makeTool("opencode", "OpenCode", { kind: "detected" }),
        makeTool("hermes", "Hermes", { kind: "not_installed" }),
      ],
    });
    expect(screen.getByText("Claude Code")).toBeTruthy();
    expect(screen.getByText("Routed")).toBeTruthy();
    expect(screen.getByText("OpenCode")).toBeTruthy();
    expect(screen.getByText("Not routed")).toBeTruthy();
    expect(screen.queryByText("Hermes")).toBeNull();
  });

  it("routes a tool with one flip from its row switch", () => {
    const onToggleTool = vi.fn();
    renderHome({
      tools: [makeTool("opencode", "OpenCode", { kind: "detected" })],
      onToggleTool,
    });
    fireEvent.click(screen.getByRole("switch", { name: "Route OpenCode through Gate" }));
    expect(onToggleTool).toHaveBeenCalledWith("opencode", true);
  });

  it("opens the tool detail from the row body", () => {
    const onOpenTool = vi.fn();
    renderHome({
      tools: [makeTool("claude-code", "Claude Code", { kind: "connected" })],
      onOpenTool,
    });
    fireEvent.click(screen.getByRole("button", { name: "Claude Code details" }));
    expect(onOpenTool).toHaveBeenCalledWith("claude-code");
  });
});

describe("Home apps ledger", () => {
  it("renders a domain row with its hosts and calls onSetDomain on flip", () => {
    const onSetDomain = vi.fn();
    renderHome({ domains: [makeDomain()], onSetDomain });
    expect(screen.getByText("Claude Desktop / Cowork")).toBeTruthy();
    expect(screen.getByText("api.anthropic.com")).toBeTruthy();
    fireEvent.click(
      screen.getByRole("switch", {
        name: "Route Claude Desktop / Cowork through Gate",
      }),
    );
    expect(onSetDomain).toHaveBeenCalledWith("anthropic", false);
  });

  it("shows Routed only when the domain is enabled, routing is on, and the CA is trusted", () => {
    renderHome({ domains: [makeDomain()] });
    expect(screen.getByText("Routed")).toBeTruthy();
    cleanup();
    renderHome({ domains: [makeDomain()], caTrusted: false });
    expect(screen.getByText("Needs trust")).toBeTruthy();
    cleanup();
    renderHome({ domains: [makeDomain()], proxyOn: false });
    expect(screen.getByText("Not routed")).toBeTruthy();
  });

  it("disables the switch on unsupported domains", () => {
    renderHome({ domains: [makeDomain({ supported: false, enabled: false })] });
    expect(screen.getByText("Coming soon")).toBeTruthy();
    expect(
      screen.getByRole("switch", {
        name: "Route Claude Desktop / Cowork through Gate",
      }),
    ).toHaveProperty("disabled", true);
  });
});
