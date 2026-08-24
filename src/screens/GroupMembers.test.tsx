import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, render, screen, fireEvent, waitFor } from "@testing-library/react";
import type { ProviderState, ProxyDomain, Tool } from "../lib/api";
import { buildGroups } from "../lib/groups";
import { GroupMembers } from "./GroupMembers";

vi.mock("../lib/analytics", () => ({ track: vi.fn(), trackError: vi.fn() }));
// GroupMembers names the secret store in two of its explainers. Pin the
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
  config_location: null,
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
    chat_domain_slugs: [],
  },
];

function renderDetail(
  tools: Tool[],
  domains: ProxyDomain[] = [domain],
  props: Partial<React.ComponentProps<typeof GroupMembers>> = {},
) {
  const [group] = buildGroups(CATALOG, tools, domains, { proxyOn: true, caTrusted: true });
  render(
    <GroupMembers
      group={group}
      busy={false}
      onToggleTool={vi.fn(() => Promise.resolve())}
      onSetDomain={vi.fn(() => Promise.resolve())}
      onTrustCa={vi.fn()}
      trustPending={false}
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

describe("GroupMembers", () => {
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
    expect(screen.getByText("the providers Gate routes")).toBeTruthy();
    expect(screen.queryByText("https://api.anthropic.com")).toBeNull();
  });

  it("does not claim it routes every provider the user configured", () => {
    // "all your providers" promised Gate stands in front of everything set up
    // in the tool. It does not: a provider the catalog does not cover is
    // skipped at connect time, not repointed, and OpenClaw and Hermes never
    // read the tool's providers at all.
    renderDetail([tool("opencode", "OpenCode", { kind: "detected" })], []);
    expect(screen.queryByText("all your providers")).toBeNull();
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

});

describe("GroupMembers inline expansion", () => {
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

describe("GroupMembers intent versus flow", () => {
  /** Routing on, certificate untrusted: the state that produced the round-6 P0. */
  function renderUntrusted(props: Partial<React.ComponentProps<typeof GroupMembers>> = {}) {
    const [group] = buildGroups(CATALOG, [], [domain], { proxyOn: true, caTrusted: false });
    render(
      <GroupMembers
        group={group}
        busy={false}
            onToggleTool={vi.fn(() => Promise.resolve())}
        onSetDomain={vi.fn(() => Promise.resolve())}
        onTrustCa={vi.fn()}
        trustPending={false}
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
    const onTrustCa = vi.fn(() => Promise.resolve());
    renderUntrusted({ onTrustCa });
    // At group level, and without expanding anything. There is one machine-wide
    // certificate, so a per-member button could only be a second or third copy
    // of this one; the family row is also where "certificate not trusted" is
    // named, so this is the level that both reports and fixes it.
    fireEvent.click(screen.getByRole("button", { name: "Trust" }));
    expect(onTrustCa).toHaveBeenCalledTimes(1);
  });

  it("names the untrusted member in the banner and offers exactly one Trust", () => {
    renderUntrusted();
    expect(screen.getByText(/Claude Desktop \/ Cowork needs the/)).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Claude Desktop / Cowork details" }));
    // Expanding the member must not add a second button for the same action.
    expect(screen.getAllByRole("button", { name: "Trust" })).toHaveLength(1);
  });

  it("warns about the system dialog here too, not only on Home", () => {
    renderUntrusted();
    // The same button raising the same OS dialog: a user who trusts from the
    // panel must not be the only one who meets it unannounced. (The platform
    // mock pins macOS, so this is the password-prompt wording.)
    expect(screen.getByText(/ask for your login password/)).toBeTruthy();
  });

  it("swaps to the present tense while the dialog is up", () => {
    renderUntrusted({ trustPending: true, busy: true });
    // Banner and live region, the same instruction seen and heard.
    expect(screen.getAllByText(/asking for your login password/)).toHaveLength(2);
    expect(screen.getByRole("button", { name: "Waiting…" })).toBeTruthy();
    // The banner is one sentence at a time: the pre-click warning and the
    // member's name give way to the instruction for the dialog on screen.
    expect(screen.queryByText(/will ask for your login password/)).toBeNull();
  });
});

describe("GroupMembers master-off remedy", () => {
  /** Switched on, engine down: the state round 6 introduced with prose only. */
  function renderMasterOff(props: Partial<React.ComponentProps<typeof GroupMembers>> = {}) {
    const [group] = buildGroups(
      CATALOG,
      [tool("claude-code", "Claude Code", { kind: "connected" })],
      [],
      { proxyOn: false, caTrusted: true },
    );
    render(
      <GroupMembers
        group={group}
        busy={false}
            onToggleTool={vi.fn(() => Promise.resolve())}
        onSetDomain={vi.fn(() => Promise.resolve())}
        onTrustCa={vi.fn()}
        trustPending={false}
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

describe("GroupMembers certificate failure", () => {
  it("shows a failed trust next to the button that failed", async () => {
    const [group] = buildGroups(CATALOG, [], [domain], { proxyOn: true, caTrusted: false });
    render(
      <GroupMembers
        group={group}
        busy={false}
            onToggleTool={vi.fn(() => Promise.resolve())}
        onSetDomain={vi.fn(() => Promise.resolve())}
        // A cancelled admin prompt: the likeliest failure in the app, and it
        // used to produce no on-screen feedback at all.
        onTrustCa={() => Promise.reject("User canceled (-128)")}
        trustPending={false}
        proxyOn={true}
        onEnableRouting={vi.fn()}
      />,
    );
    fireEvent.click(screen.getByRole("button", { name: "Trust" }));
    expect(await screen.findByText("The system prompt was cancelled")).toBeTruthy();
  });
});

describe("GroupMembers row hit target", () => {
  /** Measured in a real browser, the stretch button's 360x74.8 box only
   *  responded across 360x26: the wrapper holding the name, the pill and the
   *  switch was `relative` without `pointer-events-none`, so its own py-2.5 band
   *  sat over the button and ate the top 49px. Clicking a tool's own name did
   *  nothing while the row highlighted as one block.
   *
   *  jsdom does no hit testing, so this asserts the structure rather than the
   *  behaviour: the wrapper must be transparent to clicks and the switch must
   *  opt back in. Home's equivalent wrapper (Home.tsx) always had this, which is
   *  how the two ledgers came to disagree. */
  it("leaves the row wrapper transparent to clicks, with the switch opted back in", () => {
    renderDetail([tool("claude-code", "Claude Code", { kind: "connected" })]);
    const stretch = screen.getByRole("button", { name: "Claude Code details" });
    const row = stretch.parentElement!;

    const wrapper = Array.from(row.children).find(
      (el) => el !== stretch && el.tagName === "DIV" && el.className.includes("flex"),
    ) as HTMLElement;
    expect(wrapper).toBeTruthy();
    expect(wrapper.className).toContain("pointer-events-none");

    // The switch lives inside that wrapper and must still be clickable.
    const toggle = screen.getByRole("switch", { name: "Route Claude Code through Gate" });
    expect(wrapper.contains(toggle)).toBe(true);
    expect(toggle.parentElement!.className).toContain("pointer-events-auto");
  });

  it("announces a member flip, not just its aria-checked", async () => {
    // The panel's live region reports the family; aria-describedby is read on
    // focus arrival, not on change. So flipping a member moved its pill from
    // "Routed" to "Not routed" with nothing announced.
    const onToggleTool = vi.fn(() => Promise.resolve());
    renderDetail(
      [tool("claude-code", "Claude Code", { kind: "connected" })],
      [domain],
      { onToggleTool },
    );
    const live = document.querySelector('[aria-live="polite"]') as HTMLElement;
    expect(live).toBeTruthy();
    expect(live.textContent).toBe("");
    fireEvent.click(screen.getByRole("switch", { name: "Route Claude Code through Gate" }));
    await waitFor(() => expect(live.textContent).toContain("Claude Code"));
  });
});
