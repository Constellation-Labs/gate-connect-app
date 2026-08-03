import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, render, screen, fireEvent } from "@testing-library/react";
import type { Mock } from "vitest";
import type { Platform } from "../lib/platform";
import type { ProviderState, Tool, ProxyDomain } from "../lib/api";
import { launchAtLoginStatus } from "../lib/api";
import { Home } from "./Home";

// The CA-trust card swaps the trust-store name by platform; drive it by
// mocking usePlatform rather than the async Tauri lookup.
vi.mock("../lib/platform", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../lib/platform")>()),
  usePlatform: vi.fn(),
}));
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
      changeNotice={null}
      onDismissChangeNotice={vi.fn()}
      onCloseAgents={vi.fn()}
      onEnableRouting={vi.fn()}
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
  it("opens compact, with the action visible and the lecture optional", () => {
    renderHome({ caTrusted: false, domains: [makeDomain()] });
    expect(screen.getByText(/Apps with no gateway setting need/)).toBeTruthy();
    // The fix is on the card; only the explanation is behind the disclosure.
    // It used to be the other way round.
    expect(screen.getByRole("button", { name: "Trust" })).toBeTruthy();
    expect(screen.queryByText(/created on this machine/)).toBeNull();
  });

  it("explains before consent, naming the keychain on macOS", () => {
    renderHome({ caTrusted: false, domains: [makeDomain()] });
    // The reassurance is on the card itself now; only the mechanism and the
    // removal condition stay behind the disclosure.
    expect(screen.getByText(/never leaves this machine/)).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "What’s this?" }));
    expect(screen.getByText(/certificate your keychain trusts/)).toBeTruthy();
    // "This machine" is the section that exists; "Certificate" was renamed
    // away when Settings collapsed to four headings, and this reference
    // survived it. Names the condition too: removal needs routing off.
    expect(screen.getByText(/Settings under This machine whenever/)).toBeTruthy();
  });

  it("names the certificate store on Windows", () => {
    renderHome({ caTrusted: false, domains: [makeDomain()] }, "windows");
    fireEvent.click(screen.getByRole("button", { name: "What’s this?" }));
    expect(screen.getByText(/certificate your certificate store trusts/)).toBeTruthy();
  });

  it("calls onTrustCa without making the user open the explanation first", () => {
    const onTrustCa = vi.fn();
    renderHome({ caTrusted: false, domains: [makeDomain()], onTrustCa });
    fireEvent.click(screen.getByRole("button", { name: "Trust" }));
    expect(onTrustCa).toHaveBeenCalledTimes(1);
  });

  it("suppresses the change notice, so the blocker is the only thing to read", () => {
    renderHome({ caTrusted: false, domains: [makeDomain()], changeNotice: "on" });
    expect(screen.getByText(/Apps with no gateway setting need/)).toBeTruthy();
    expect(screen.queryByText(/Routing is on\./)).toBeNull();
  });

  it("is absent while routing is off or the CA is trusted", () => {
    renderHome({ proxyOn: false, caTrusted: false, domains: [makeDomain()] });
    expect(screen.queryByText(/Apps with no gateway setting need/)).toBeNull();
    cleanup();
    renderHome({ caTrusted: true });
    expect(screen.queryByText(/Apps with no gateway setting need/)).toBeNull();
  });
});

describe("Home master toggle", () => {
  it("calls onToggleProxy", () => {
    const onToggleProxy = vi.fn();
    renderHome({ onToggleProxy });
    fireEvent.click(screen.getByRole("switch", { name: "Route through Gate" }));
    expect(onToggleProxy).toHaveBeenCalledTimes(1);
  });

  it("keeps the count and lets the card carry the certificate message", () => {
    renderHome({ caTrusted: false, domains: [makeDomain()] });
    // The count used to disappear here, which is the state where the user most
    // wants to know how much still works. The certificate clause left with it:
    // the trust card directly below is that sentence, louder, and saying it
    // twice within 300px was this state's worst habit.
    expect(screen.getByText("On · 0 of 1 routing")).toBeTruthy();
    expect(screen.getByText(/Apps with no gateway setting need/)).toBeTruthy();
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
    // Twice each: the visible sub-line, and the sr-only description the
    // stretch button points at (the visible copy truncates at 360px).
    expect(screen.getAllByText(/2 of 2 routing/)).toHaveLength(2);
    expect(screen.getAllByText(/0 of 1 routing/)).toHaveLength(2);
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
    expect(screen.getAllByText(/Claude Code failed/).length).toBeGreaterThan(0);
  });

  it("flags a hand-written setup without calling the family broken", () => {
    renderHome({
      tools: [makeTool("codex", "Codex", { kind: "drifted", reason: "r" }, "OpenAI")],
      domains: [],
    });
    expect(screen.getAllByText(/Codex set up elsewhere/).length).toBeGreaterThan(0);
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

describe("Home routing-change notice", () => {
  it("words the same notice by direction", () => {
    renderHome({ changeNotice: "on", domains: [makeDomain()] });
    expect(screen.getByText(/Routing is on\./)).toBeTruthy();
    cleanup();

    renderHome({ changeNotice: "off", domains: [makeDomain()] });
    expect(screen.getByText(/Routing is off\./)).toBeTruthy();
    expect(screen.queryByText(/Routing is on\./)).toBeNull();
  });

  it("offers to close the running tools in both directions, not just at startup", () => {
    const onCloseAgents = vi.fn();
    renderHome({ changeNotice: "off", onCloseAgents, domains: [makeDomain()] });
    fireEvent.click(screen.getByRole("button", { name: "Close them…" }));
    expect(onCloseAgents).toHaveBeenCalled();
  });

  it("shows one notice at a time, so a fast flip can't stack them", () => {
    // The regression this replaced: three independent hint booleans, so the
    // "on" notice stayed up over the "off" one after an on/off flip.
    renderHome({ changeNotice: "off", staleAgentsHint: true, domains: [makeDomain()] });
    expect(screen.getAllByRole("status")).toHaveLength(1);
    expect(screen.getByText(/local address changed/)).toBeTruthy();
    expect(screen.queryByText(/Routing is off\./)).toBeNull();
  });
});

describe("Home dashboard link", () => {
  it("is not in the scroll area", () => {
    // It moved to the pinned footer in App: inside the ledger it sat below
    // the fold on the most common Home, and it was costing ~34px of a screen
    // that has none to spare.
    renderHome();
    expect(screen.queryByRole("button", { name: /Gate dashboard/ })).toBeNull();
  });
});

describe("Home ledger accessibility", () => {
  it("gives the row's whole sentence to the drill-in button, pill state included", () => {
    renderHome({
      tools: [makeTool("claude-code", "Claude Code", { kind: "error", message: "bad json" })],
      domains: [makeDomain()],
    });
    const row = screen.getByRole("button", { name: "Claude details" });
    const described = document.getElementById(row.getAttribute("aria-describedby")!);
    // Without this the row announced "Claude details, button" and nothing
    // else: the count, the pill and the failure were all in pointer-events-none
    // spans under the stretch button.
    expect(described?.textContent).toContain("Partly routed");
    expect(described?.textContent).toContain("1 of 2 routing");
    expect(described?.textContent).toContain("Claude Code failed");
  });

  it("exposes the ledger as a list", () => {
    renderHome({ tools: [makeTool("claude-code", "Claude Code", { kind: "connected" })] });
    expect(screen.getByRole("list")).toBeTruthy();
    expect(screen.getAllByRole("listitem").length).toBeGreaterThan(0);
  });
});

describe("Home pending notice", () => {
  it("does not claim routing is on when nothing can route", () => {
    renderHome({ proxyOn: false, changeNotice: "pending", domains: [makeDomain()] });
    // The round-7 P0: the notice came from the toggle's direction, so
    // enabling a proxy-only family with the engine down announced
    // "Routing is on" directly above "Off · not routing".
    expect(screen.queryByText(/Routing is on\./)).toBeNull();
    expect(screen.getByText(/nothing is going through Gate yet/)).toBeTruthy();
  });

  it("offers the remedy instead of asking the user to close nothing", () => {
    const onEnableRouting = vi.fn();
    const onCloseAgents = vi.fn();
    renderHome({
      proxyOn: false,
      changeNotice: "pending",
      onEnableRouting,
      onCloseAgents,
      domains: [makeDomain()],
    });
    expect(screen.queryByRole("button", { name: "Close them…" })).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "Turn on routing" }));
    expect(onEnableRouting).toHaveBeenCalledTimes(1);
    expect(onCloseAgents).not.toHaveBeenCalled();
  });
});

describe("Home master card is a control that owns up", () => {
  it("says so when a family switch turned routing on as well", () => {
    renderHome({ changeNotice: "started", domains: [makeDomain()] });
    // The master is a control and a family switch may start it (connecting a
    // config tool has to). The rule is do it and say so, so this must not hide
    // inside the generic "Routing is on".
    expect(screen.getByText(/turned routing on too/)).toBeTruthy();
  });

  it("names members its own switches cannot reach", () => {
    renderHome({
      tools: [
        makeTool("claude-code", "Claude Code", { kind: "connected" }),
        makeTool("codex", "Codex", { kind: "drifted", reason: "r" }, "OpenAI"),
      ],
      domains: [makeDomain()],
    });
    // A family switch skips a hand-written setup, so without this the
    // denominator sets a target the controls can't hit.
    expect(screen.getByText(/needs attention/)).toBeTruthy();
  });

  it("stays quiet about attention when everything is reachable", () => {
    renderHome({
      tools: [makeTool("claude-code", "Claude Code", { kind: "connected" })],
      domains: [makeDomain()],
    });
    expect(screen.getByText("On · 2 of 2 routing")).toBeTruthy();
    expect(screen.queryByText(/attention/)).toBeNull();
  });
});

describe("Home says what is waiting", () => {
  it("names what a switch-on would revive, instead of just saying off", () => {
    renderHome({
      proxyOn: false,
      tools: [makeTool("claude-code", "Claude Code", { kind: "connected" })],
      domains: [],
    });
    // "Off · not routing" while two families read "waiting on routing" is the
    // fact that makes flipping the switch feel safe rather than speculative.
    expect(screen.getByText("Off · 1 waiting")).toBeTruthy();
  });

  it("does not offer to close apps when nothing is installed", () => {
    renderHome({ changeNotice: "on", tools: [], domains: [] });
    expect(screen.queryByText(/Routing is on\./)).toBeNull();
    expect(screen.getByText(/Nothing to route yet/)).toBeTruthy();
  });
});

describe("Home empty state tells the truth about refreshing", () => {
  it("names an action that actually re-reads the ledger", () => {
    renderHome({ tools: [], domains: [] });
    // Reopening from the tray now refreshes (App wires useWindowReopen to
    // refreshState). Before that it did not, so this copy was instructing the
    // user to perform the one action that provably changed nothing.
    expect(screen.getByText(/reopen this window from the menu bar/)).toBeTruthy();
  });
});

describe("Home quiet-state honesty", () => {
  it("does not fly a green pill over an empty ledger", () => {
    renderHome({ tools: [], domains: [] });
    // "Routing on" above "nothing installed to route" is technically true and
    // reads as a contradiction.
    expect(screen.getByText("Nothing to route")).toBeTruthy();
    expect(screen.queryByText("Routing on")).toBeNull();
  });

  it("keeps the launch-at-login tip dismissible", async () => {
    // Once, so the file-level never-resolving default (which keeps the tip
    // hidden and other tests free of act warnings) is restored afterwards.
    (launchAtLoginStatus as Mock).mockResolvedValueOnce({
      enabled: false,
      pending_disable: false,
    });
    renderHome({ tools: [makeTool("claude-code", "Claude Code", { kind: "connected" })] });
    const tip = await screen.findByRole("button", { name: /Turn on Launch at login/ });
    expect(tip).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Dismiss launch at login tip" }));
    expect(screen.queryByRole("button", { name: /Turn on Launch at login/ })).toBeNull();
  });
});
