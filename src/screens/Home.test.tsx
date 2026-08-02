import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, render, screen, fireEvent } from "@testing-library/react";
import type { Mock } from "vitest";
import type { Platform } from "../lib/platform";
import type { ProviderState, Tool, ProxyDomain } from "../lib/api";
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

/** Mirrors the real catalog: Claude Code and Codex are claimed; OpenCode and
 * OpenClaw deliberately are not, so they land in "Agent harnesses". */
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

function renderHome(props: Partial<React.ComponentProps<typeof Home>> = {}, platform: Platform = "macos") {
  (usePlatform as Mock).mockReturnValue(platform);
  render(
    <Home
      workspace="gateway.constellationgate.ai"
      proxyOn={true}
      caTrusted={true}
      showProxy={true}
      providers={CATALOG}
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
    // Claude: Claude Code + Cowork both routing. OpenAI: Codex only, off.
    expect(screen.getByText(/2 of 2 routing/)).toBeTruthy();
    expect(screen.getByText(/0 of 1 routing/)).toBeTruthy();
  });

  it("gives the multi-provider tools an honest home, not a wrong family", () => {
    renderHome({
      // The real backend calls their upstream "your existing providers".
      tools: [
        makeTool("opencode", "OpenCode", { kind: "detected" }, "your existing providers"),
        makeTool("openclaw", "OpenClaw", { kind: "detected" }, "your existing providers"),
      ],
      domains: [],
    });
    expect(screen.getByText("Agent harnesses")).toBeTruthy();
    expect(screen.queryByText(/existing providers/)).toBeNull();
  });

  it("reports a partly-routed family without rounding up", () => {
    renderHome({
      tools: [makeTool("claude-code", "Claude Code", { kind: "connected" })],
      // Its sibling app row is switched off, so the family is half on.
      domains: [makeDomain({ enabled: false })],
    });
    expect(screen.getByText("Partly routed")).toBeTruthy();
    // Exact: the Routing card says "On · 1 of 2 routing", the row just the count.
    expect(screen.getByText("1 of 2 routing")).toBeTruthy();
  });

  it("names an exception in the sub-line instead of hijacking the pill", () => {
    renderHome({
      tools: [makeTool("claude-code", "Claude Code", { kind: "error", message: "bad json" })],
      domains: [makeDomain()],
    });
    // The pill still answers "is this routing?"; the failure is named below.
    expect(screen.getByText("Partly routed")).toBeTruthy();
    expect(screen.getByText(/Claude Code failed/)).toBeTruthy();
  });

  it("flags a hand-written setup without calling the family broken", () => {
    renderHome({
      tools: [makeTool("codex", "Codex", { kind: "drifted", reason: "r" }, "OpenAI")],
      domains: [],
    });
    expect(screen.getByText(/Codex set up elsewhere/)).toBeTruthy();
  });

  it("routes a whole family with one flip", () => {
    const onToggleGroup = vi.fn();
    renderHome({
      tools: [makeTool("claude-code", "Claude Code", { kind: "detected" })],
      domains: [],
      onToggleGroup,
    });
    fireEvent.click(screen.getByRole("switch", { name: "Route Claude through Gate" }));
    expect(onToggleGroup).toHaveBeenCalledWith("anthropic", true);
  });

  it("opens the family detail from the row body", () => {
    const onOpenGroup = vi.fn();
    renderHome({
      tools: [makeTool("claude-code", "Claude Code", { kind: "connected" })],
      domains: [],
      onOpenGroup,
    });
    fireEvent.click(screen.getByRole("button", { name: "Claude details" }));
    expect(onOpenGroup).toHaveBeenCalledWith("anthropic");
  });
});
