import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, render, screen, fireEvent, waitFor } from "@testing-library/react";
import type { ProviderState, ProxyDomain, Tool } from "../lib/api";
import { buildGroups } from "../lib/groups";
import { GroupDetail } from "./GroupDetail";

vi.mock("../lib/analytics", () => ({ track: vi.fn(), trackError: vi.fn() }));
// GroupDetail names the secret store in two of its explainers. Pin the
// platform so that copy is deterministic, and so the real hook's async
// resolve does not settle outside act().
vi.mock("../lib/platform", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../lib/platform")>()),
  usePlatform: () => "macos",
}));

function tool(slug: string, name: string, status: Tool["status"]): Tool {
  return {
    slug,
    name,
    upstream_provider_name: "Anthropic",
    default_upstream_url: "https://api.anthropic.com",
    requires_upstream_credential: false,
    status,
  };
}

const domain: ProxyDomain = {
  slug: "anthropic",
  display_name: "Claude Desktop / Cowork",
  hosts: ["api.anthropic.com"],
  upstream_url: "https://api.anthropic.com",
  rewrite_prefixes: [],
  passthrough_prefixes: [],
  enabled: true,
  supported: true,
};

const CATALOG: ProviderState[] = [
  {
    slug: "anthropic",
    display_name: "Claude",
    subtitle: "",
    enabled: false,
    available: true,
    tool_slugs: ["claude-code", "openclaw", "codex"],
    domain_slugs: ["anthropic"],
  },
];

function renderDetail(
  tools: Tool[],
  domains: ProxyDomain[] = [domain],
  props: Partial<React.ComponentProps<typeof GroupDetail>> = {},
) {
  const [group] = buildGroups(CATALOG, tools, domains, { proxyOn: true, caTrusted: true });
  render(
    <GroupDetail
      group={group}
      busy={false}
      onBack={vi.fn()}
      onToggleGroup={vi.fn()}
      onToggleTool={vi.fn(() => Promise.resolve())}
      onSetDomain={vi.fn(() => Promise.resolve())}
      onTrustCa={vi.fn()}
      proxyOn={true}
      onEnableRouting={vi.fn()}
      {...props}
    />,
  );
  return group;
}

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

describe("GroupDetail", () => {
  it("shows the mechanism per member, which Home deliberately omits", () => {
    renderDetail([tool("claude-code", "Claude Code", { kind: "connected" })]);
    expect(screen.getByText("config file")).toBeTruthy();
    expect(screen.getByText("proxy")).toBeTruthy();
    // Both members now print a bare host: the config member used to print the
    // full URL and truncate to `https://api.ant…` in the same slot.
    expect(screen.getAllByText("api.anthropic.com")).toHaveLength(2);
    expect(screen.queryByText("https://api.anthropic.com")).toBeNull();
  });

  it("names no upstream host for a harness, whose default_upstream_url is a placeholder", () => {
    // OpenCode is claimed by no provider and its default_upstream_url is the
    // constant api.anthropic.com, which is wrong the moment the user has
    // OpenAI configured in it.
    renderDetail([tool("opencode", "OpenCode", { kind: "detected" })], []);
    expect(screen.getByText("all your providers")).toBeTruthy();
    expect(screen.queryByText("https://api.anthropic.com")).toBeNull();
  });

  it("toggles one member without touching the rest", async () => {
    const onToggleTool = vi.fn(() => Promise.resolve());
    const onSetDomain = vi.fn(() => Promise.resolve());
    renderDetail([tool("claude-code", "Claude Code", { kind: "detected" })], [domain], {
      onToggleTool,
      onSetDomain,
    });
    fireEvent.click(screen.getByRole("switch", { name: "Route Claude Code through Gate" }));
    await waitFor(() => expect(onToggleTool).toHaveBeenCalledWith("claude-code", true));
    expect(onSetDomain).not.toHaveBeenCalled();

    fireEvent.click(
      screen.getByRole("switch", { name: "Route Claude Desktop / Cowork through Gate" }),
    );
    await waitFor(() => expect(onSetDomain).toHaveBeenCalledWith("anthropic", false));
  });

  it("routes the whole family from the group switch", () => {
    const onToggleGroup = vi.fn();
    renderDetail([tool("claude-code", "Claude Code", { kind: "detected" })], [], { onToggleGroup });
    fireEvent.click(screen.getByRole("switch", { name: "Route Claude through Gate" }));
    expect(onToggleGroup).toHaveBeenCalledWith("anthropic", true);
  });
});

describe("GroupDetail inline expansion", () => {
  it("explains a member in place rather than on a third screen", () => {
    renderDetail([tool("claude-code", "Claude Code", { kind: "connected" })], []);
    const row = screen.getByRole("button", { name: "Claude Code details" });
    expect(row.getAttribute("aria-expanded")).toBe("false");
    fireEvent.click(row);
    expect(row.getAttribute("aria-expanded")).toBe("true");
    expect(screen.getByText(/never lands in the config file/)).toBeTruthy();
  });

  it("shows a failure's whole message inline", () => {
    renderDetail(
      [
        tool("openclaw", "OpenClaw", {
          kind: "error",
          message: "failed to parse ~/.openclaw/openclaw.json: unexpected character",
        }),
      ],
      [],
    );
    fireEvent.click(screen.getByRole("button", { name: "OpenClaw details" }));
    expect(screen.getByText(/unexpected character/)).toBeTruthy();
  });

  it("surfaces a failed toggle next to the member that failed", async () => {
    const onToggleTool = vi.fn(() => Promise.reject("No upstream credential saved."));
    renderDetail([tool("claude-code", "Claude Code", { kind: "detected" })], [], { onToggleTool });
    fireEvent.click(screen.getByRole("switch", { name: "Route Claude Code through Gate" }));
    expect(await screen.findByText("Couldn’t connect this tool")).toBeTruthy();
  });

  it("asks before replacing a hand-written setup, and only writes on confirm", async () => {
    const onToggleTool = vi.fn(() => Promise.resolve());
    renderDetail([tool("codex", "Codex", { kind: "drifted", reason: "r" })], [], { onToggleTool });
    fireEvent.click(screen.getByRole("switch", { name: "Route Codex through Gate" }));
    // First flip only arms the confirm; nothing is written yet.
    expect(onToggleTool).not.toHaveBeenCalled();
    expect(screen.getByText(/Replace Codex/)).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Replace setup" }));
    await waitFor(() => expect(onToggleTool).toHaveBeenCalledWith("codex", true));
  });

  it("says the group switch will leave a hand-written setup alone", () => {
    renderDetail([tool("codex", "Codex", { kind: "drifted", reason: "r" })], []);
    expect(screen.getByText(/group switch leaves/)).toBeTruthy();
  });

  it("announces a failed member at the top, not only inside its own row", () => {
    renderDetail([tool("openclaw", "OpenClaw", { kind: "error", message: "bad json" })], []);
    // The most severe member state was the only one with no banner: master-off
    // and drifted each got one, so a failure was the single thing the screen
    // left the user to find by expanding rows.
    expect(screen.getByText(/OpenClaw isn’t reporting its routing state/)).toBeTruthy();
  });

  it("points a failed member at its own payload, not at restarting the app", async () => {
    renderDetail(
      [
        tool("openclaw", "OpenClaw", {
          kind: "error",
          message: "failed to parse ~/.openclaw/openclaw.json line 42",
        }),
      ],
      [],
    );
    fireEvent.click(screen.getByRole("button", { name: "OpenClaw details" }));
    // The advice used to be "try again after restarting Gate Connect" printed
    // directly above a payload naming a syntax error in the user's own file.
    expect(screen.queryByText(/restarting Gate Connect/)).toBeNull();
    expect(screen.getByText(/details below name the cause/)).toBeTruthy();
  });

  it("lets the payload be copied out of a 360px window", async () => {
    const writeText = vi.fn(() => Promise.resolve());
    Object.assign(navigator, { clipboard: { writeText } });
    renderDetail(
      [tool("openclaw", "OpenClaw", { kind: "error", message: "line 42, column 3" })],
      [],
    );
    fireEvent.click(screen.getByRole("button", { name: "OpenClaw details" }));
    fireEvent.click(screen.getByRole("button", { name: /Copy details/ }));
    await waitFor(() => expect(writeText).toHaveBeenCalledWith("line 42, column 3"));
  });
});

describe("GroupDetail intent versus flow", () => {
  /** Routing on, certificate untrusted: the state that produced the round-6 P0. */
  function renderUntrusted(props: Partial<React.ComponentProps<typeof GroupDetail>> = {}) {
    const [group] = buildGroups(CATALOG, [], [domain], { proxyOn: true, caTrusted: false });
    render(
      <GroupDetail
        group={group}
        busy={false}
        onBack={vi.fn()}
        onToggleGroup={vi.fn()}
        onToggleTool={vi.fn(() => Promise.resolve())}
        onSetDomain={vi.fn(() => Promise.resolve())}
        onTrustCa={vi.fn()}
      proxyOn={true}
      onEnableRouting={vi.fn()}
        {...props}
      />,
    );
  }

  it("shows a needs-trust member as switched on, because it is", () => {
    renderUntrusted();
    const sw = screen.getByRole("switch", { name: "Route Claude Desktop / Cowork through Gate" });
    expect(sw.getAttribute("aria-checked")).toBe("true");
    // Twice: the aria-hidden pill, and the sr-only node the switch points at
    // so a screen reader doesn't hear "on" for something that isn't flowing.
    expect(screen.getAllByText("Needs trust")).toHaveLength(2);
    expect(document.getElementById(sw.getAttribute("aria-describedby")!)?.textContent).toBe(
      "Needs trust",
    );
  });

  it("turns a needs-trust member OFF when clicked, not off again", async () => {
    const onSetDomain = vi.fn(() => Promise.resolve());
    renderUntrusted({ onSetDomain });
    fireEvent.click(
      screen.getByRole("switch", { name: "Route Claude Desktop / Cowork through Gate" }),
    );
    // The old code also sent `false` here, but from a switch that read off, so
    // the user believed they were switching it on.
    await waitFor(() => expect(onSetDomain).toHaveBeenCalledWith("anthropic", false));
  });

  it("offers the certificate remedy where the problem is named", () => {
    const onTrustCa = vi.fn();
    renderUntrusted({ onTrustCa });
    fireEvent.click(screen.getByRole("button", { name: "Claude Desktop / Cowork details" }));
    fireEvent.click(screen.getByRole("button", { name: "Trust certificate" }));
    expect(onTrustCa).toHaveBeenCalledTimes(1);
  });
});

describe("GroupDetail master-off remedy", () => {
  /** Switched on, engine down: the state round 6 introduced with prose only. */
  function renderMasterOff(props: Partial<React.ComponentProps<typeof GroupDetail>> = {}) {
    const [group] = buildGroups(
      CATALOG,
      [tool("claude-code", "Claude Code", { kind: "connected" })],
      [],
      { proxyOn: false, caTrusted: true },
    );
    render(
      <GroupDetail
        group={group}
        busy={false}
        onBack={vi.fn()}
        onToggleGroup={vi.fn()}
        onToggleTool={vi.fn(() => Promise.resolve())}
        onSetDomain={vi.fn(() => Promise.resolve())}
        onTrustCa={vi.fn()}
        proxyOn={false}
        onEnableRouting={vi.fn()}
        {...props}
      />,
    );
    return group;
  }

  it("names the state on the member", () => {
    renderMasterOff();
    expect(screen.getAllByText("Waiting on routing")).toHaveLength(2);
  });

  it("offers the way out from the expanded member, not just prose", () => {
    const onEnableRouting = vi.fn();
    renderMasterOff({ onEnableRouting });
    fireEvent.click(screen.getByRole("button", { name: "Claude Code details" }));
    const buttons = screen.getAllByRole("button", { name: "Turn on routing" });
    fireEvent.click(buttons[buttons.length - 1]);
    expect(onEnableRouting).toHaveBeenCalled();
  });

  it("does not tell the user to close an app that was never routing", async () => {
    const onToggleTool = vi.fn(() => Promise.resolve());
    renderMasterOff({ onToggleTool });
    fireEvent.click(screen.getByRole("switch", { name: "Route Claude Code through Gate" }));
    await waitFor(() => expect(onToggleTool).toHaveBeenCalled());
    // Closing and reopening changes nothing while the engine is down.
    expect(screen.queryByText(/to apply the change/)).toBeNull();
  });
});

describe("GroupDetail certificate failure", () => {
  it("shows a failed trust next to the button that failed", async () => {
    const [group] = buildGroups(CATALOG, [], [domain], { proxyOn: true, caTrusted: false });
    render(
      <GroupDetail
        group={group}
        busy={false}
        onBack={vi.fn()}
        onToggleGroup={vi.fn()}
        onToggleTool={vi.fn(() => Promise.resolve())}
        onSetDomain={vi.fn(() => Promise.resolve())}
        // A cancelled admin prompt: the likeliest failure in the app, and it
        // used to produce no on-screen feedback at all.
        onTrustCa={() => Promise.reject("User canceled (-128)")}
        proxyOn={true}
        onEnableRouting={vi.fn()}
      />,
    );
    fireEvent.click(screen.getByRole("button", { name: "Claude Desktop / Cowork details" }));
    fireEvent.click(screen.getByRole("button", { name: "Trust certificate" }));
    expect(await screen.findByText("The system prompt was cancelled")).toBeTruthy();
  });
});
