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
  config_location: null,
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
    chat_domain_slugs: [],
  },
  {
    slug: "openai",
    display_name: "OpenAI",
    subtitle: "",
    enabled: false,
    available: true,
    tool_slugs: ["codex"],
    domain_slugs: ["openai"],
    chat_domain_slugs: [],
  },
];

function renderHome(props: Partial<React.ComponentProps<typeof Home>> = {}, platform: Platform = "macos") {
  (usePlatform as Mock).mockReturnValue(platform);
  // Returned so a test can reach the sr-only live region, which has no role
  // and no accessible name to query by.
  return render(
    <Home
      workspace="Constellation Labs"
      gatewayHost="gateway.constellationgate.ai"
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
      trustPending={false}
      onOpenFamily={vi.fn()}
      onOpenSettings={vi.fn()}
      envExportSeparable={true}
      envExportOn={true}
      onToggleEnvExport={vi.fn()}
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

  it("warns a Windows user about the security warning before they click", () => {
    renderHome({ caTrusted: false, domains: [makeDomain()] }, "windows");
    // Outside the disclosure: an unannounced OS security dialog is exactly what
    // makes this ceremony feel like something went wrong, and the users who
    // open "What's this?" are the ones who least need warning.
    expect(screen.getByText(/security warning: that’s expected, choose Yes/)).toBeTruthy();
  });

  it("names the login password on macOS instead of the Windows dialog", () => {
    renderHome({ caTrusted: false, domains: [makeDomain()] }, "macos");
    expect(screen.getByText(/ask for your login password/)).toBeTruthy();
  });

  it("names the dialog on screen while it is up, and says the button is not dead", () => {
    renderHome(
      { caTrusted: false, domains: [makeDomain()], trustPending: true, busy: true },
      "windows",
    );
    // The certificate the dialog is quoting back, so the user can match the two.
    // Twice: the sentence on the card and the announcement in the live region
    // below, which is the same instruction for someone who cannot see either
    // the card change or the dialog appear.
    expect(screen.getAllByText(/Gate Connect Local CA/)).toHaveLength(2);
    expect(screen.getByRole("button", { name: "Waiting…" })).toBeTruthy();
    // The pre-click phrasing is gone: the warning is no longer coming, it's here.
    expect(screen.queryByText(/that’s expected, choose Yes/)).toBeNull();
  });

  it("announces the dialog, which a screen-reader user cannot see appear", () => {
    const { container } = renderHome(
      { caTrusted: false, domains: [makeDomain()], trustPending: true, busy: true },
      "windows",
    );
    const live = container.querySelector('[aria-live="polite"]');
    // Outranks "Routing on, certificate not trusted": that state is old news
    // next to a modal the user has not been told about.
    expect(live?.textContent).toContain("Gate Connect Local CA");
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

describe("Home ledger rows", () => {
  it("heads the list with the words the panel titles itself with", () => {
    renderHome({
      tools: [makeTool("claude-code", "Claude Code", { kind: "connected" })],
      domains: [makeDomain()],
    });
    // The door retired as a control; its sentence stayed as the heading, which
    // is also what puts the list into the document outline.
    expect(screen.getByRole("heading", { name: "What routes through Gate" })).toBeTruthy();
  });

  it("opens the row's own family, not a list of all of them", () => {
    const onOpenFamily = vi.fn();
    renderHome({
      tools: [makeTool("claude-code", "Claude Code", { kind: "connected" })],
      onOpenFamily,
    });
    fireEvent.click(screen.getByRole("button", { name: "Claude details" }));
    // Four chevrons used to reach one panel that differed only by which family
    // arrived expanded. The id is the destination now, not a hint about where to
    // scroll once you get there.
    expect(onOpenFamily).toHaveBeenCalledWith("anthropic");
  });

  it("keeps the rows off the routing card", () => {
    renderHome({
      tools: [makeTool("claude-code", "Claude Code", { kind: "connected" })],
      domains: [makeDomain()],
    });
    // The card is one control and its address; a list of the things it governs
    // is a different grain, so it gets its own surface.
    const row = screen.getByRole("button", { name: "Claude details" });
    const master = screen.getByRole("switch", { name: "Route through Gate" });
    expect(master.closest(".shadow-border")!.contains(row)).toBe(false);
  });

  it("puts the dashboard link after anything the app has to say", () => {
    renderHome({ changeNotice: "on", domains: [makeDomain()] });
    // It is the one control here that leaves Gate Connect, so a warning, a
    // blocker or a failed toggle all outrank it. It used to sit above them.
    const notice = screen.getByRole("status");
    const link = screen.getByRole("button", { name: /Gate dashboard/ });
    expect(
      notice.compareDocumentPosition(link) & Node.DOCUMENT_POSITION_FOLLOWING,
    ).toBeTruthy();
  });

  it("gives every family its own row rather than a joined list", () => {
    renderHome({
      tools: [
        makeTool("claude-code", "Claude Code", { kind: "connected" }),
        makeTool("codex", "Codex", { kind: "connected" }, "OpenAI"),
      ],
      domains: [makeDomain()],
    });
    // The door printed "Claude, OpenAI" as one line of prose. A row per family
    // is what lets each carry its own pill, which is the point of the screen.
    expect(screen.getByRole("button", { name: "Claude details" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "OpenAI details" })).toBeTruthy();
    expect(screen.queryByText("Claude, OpenAI")).toBeNull();
  });

  it("still names the families when routing is off", () => {
    // Every member reports master-off with routing down, so ranking that state
    // as an exception made the door print "waiting on routing" and drop the
    // inventory - under a card already reading "Off · 1 waiting". Routing-off is
    // the one state whose only question is what comes back when you flip it.
    renderHome({
      proxyOn: false,
      tools: [makeTool("claude-code", "Claude Code", { kind: "connected" })],
      domains: [],
    });
    expect(screen.getByText("Claude")).toBeTruthy();
    expect(screen.queryByText("waiting on routing")).toBeNull();
    // The master card keeps sole ownership of its own state.
    expect(screen.getByText("Off · 1 waiting")).toBeTruthy();
  });

  it("names the failure and keeps the family it belongs to, in its own ink", () => {
    // The door had to choose, and chose the exception, so a mid-task user read
    // "Claude Code failed" without learning which of four families to open. A
    // row carries both.
    renderHome({
      tools: [makeTool("claude-code", "Claude Code", { kind: "error", message: "bad json" })],
      domains: [makeDomain()],
    });
    const note = screen.getByText("Claude Code failed");
    expect(note.className).toContain("text-gc-error-deep");
    expect(screen.getByText("Claude")).toBeTruthy();
  });

  it("floats the failure to the top and still shows the quieter exception", () => {
    renderHome({
      tools: [
        makeTool("claude-code", "Claude Code", { kind: "error", message: "bad json" }),
        makeTool("codex", "Codex", { kind: "drifted", reason: "r" }, "OpenAI"),
      ],
      domains: [makeDomain()],
    });
    // Both are reported now, because there is a row for each. What the ranking
    // buys is that the one needing a human most is the first thing on screen.
    const failure = screen.getByText("Claude Code failed");
    const drift = screen.getByText("Codex set up elsewhere");
    expect(
      failure.compareDocumentPosition(drift) & Node.DOCUMENT_POSITION_FOLLOWING,
    ).toBeTruthy();
  });

  it("leaves the certificate to the card that can fix it", () => {
    // A member is only untrusted while the CA card is up, so printing
    // "certificate not trusted" on each affected row would repeat the card's own
    // sentence beside the only button that resolves it.
    renderHome({
      caTrusted: false,
      tools: [makeTool("claude-code", "Claude Code", { kind: "connected" })],
      domains: [makeDomain()],
    });
    expect(screen.getByRole("button", { name: "Trust" })).toBeTruthy();
    expect(screen.queryByText("certificate not trusted")).toBeNull();
  });

  it("keeps a hand-written setup in the quieter ink", () => {
    renderHome({
      tools: [makeTool("codex", "Codex", { kind: "drifted", reason: "r" }, "OpenAI")],
      domains: [],
    });
    const note = screen.getByText("Codex set up elsewhere");
    expect(note.className).toContain("text-gc-ink-2");
    expect(note.className).not.toContain("error");
  });

  it("stops the header claiming green while a tool is failing", () => {
    renderHome({
      tools: [makeTool("claude-code", "Claude Code", { kind: "error", message: "bad json" })],
      domains: [makeDomain()],
    });
    // Traffic is flowing (the app row routes) and something failed, so the
    // header reports the honest half-state. It used to fly green "Routing on"
    // over a family whose own pill read grey "Not routed", which is the one
    // screen a user opens *because* a tool stopped working.
    expect(screen.queryByText("Routing on")).toBeNull();
    // Twice: the header's roll-up and the family's own pill, which agree.
    expect(screen.getAllByText("Partly routed")).toHaveLength(2);
  });

  it("keeps the header honest when a failure leaves nothing routing", () => {
    renderHome({
      tools: [makeTool("claude-code", "Claude Code", { kind: "error", message: "bad json" })],
      domains: [],
    });
    // Same words as a deliberately-off setup, but amber rather than grey,
    // because the cause is a failure.
    expect(screen.getByText("Nothing routing")).toBeTruthy();
  });

  it("leaves a hand-written setup out of the header pill", () => {
    renderHome({
      tools: [makeTool("codex", "Codex", { kind: "drifted", reason: "r" }, "OpenAI")],
      domains: [makeDomain()],
    });
    // Drift is a setup the user chose and the family switch deliberately
    // respects, not a failure, so it must not demote the header the way an
    // error does.
    expect(screen.getByText("Routing on")).toBeTruthy();
  });

  it("names the gateway host on its own line, not only in the header", () => {
    renderHome();
    // The header carries the org now, because a gateway host is byte-identical
    // for every customer and the org is what gets billed. The host still has to
    // be answerable from Home, so it gets a line.
    expect(screen.getByText("gateway.constellationgate.ai")).toBeTruthy();
    expect(screen.getByText("Constellation Labs")).toBeTruthy();
  });

  it("gives the dashboard link a line of its own", () => {
    renderHome();
    // Fifth address for this link. The previous four each failed: under the
    // fold, squeezing the credential promise in the footer, outweighing the
    // ledger heading it rode, and taking width from the one identifier on this
    // screen that cannot be shortened without lying.
    const link = screen.getByRole("button", { name: /Gate dashboard/ });
    const host = screen.getByText("gateway.constellationgate.ai");
    expect(host.contains(link)).toBe(false);
    expect(link.contains(host)).toBe(false);
  });

  it("prints the host once when there is no org to name", () => {
    // App passes an empty workspace for a key account, which has no org. The
    // header showed the host there, and the wire line shows it too, so the same
    // string appeared twice 230px apart.
    renderHome({ workspace: "" });
    expect(screen.getAllByText("gateway.constellationgate.ai").length).toBe(1);
  });

  it("leaves a way to read an identifier its slot truncates", () => {
    // The header's org line and the host line are both clamped, and the ellipsis
    // truncation paints exists in no attribute. Without a title the full value is
    // unrecoverable from the screen that names it.
    renderHome({
      workspace: "Constellation Networks Advanced Research and Platform Engineering",
    });
    expect(
      screen.getByTitle("Constellation Networks Advanced Research and Platform Engineering"),
    ).toBeTruthy();
    expect(screen.getByTitle("gateway.constellationgate.ai")).toBeTruthy();
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
    // A family switch skips a hand-written setup, so a bare denominator would
    // set a target the controls can't hit. The card used to carry a "· 1 needs
    // attention" clause for this; the row now names the member instead, which
    // is the same duty discharged by something the user can act on. Counting it
    // in the card as well made one fault arrive in three vocabularies before
    // the user learned which tool it was.
    expect(screen.getByText("Codex set up elsewhere")).toBeTruthy();
    expect(screen.getByText("On · 2 of 3 routing")).toBeTruthy();
    expect(screen.queryByText(/needs attention/)).toBeNull();
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
    expect(screen.getByText(/Install a tool to route/)).toBeTruthy();
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

  it("does not fly a green pill when routing is on but nothing is routed", () => {
    // The ledger is full and the master is on, but no row is enabled: the
    // rows all read "Not routed" and the header used to read green "Routing
    // on" above them. Green is the only signal a mid-task user takes in, so
    // this is the app telling them their traffic is covered when it is not.
    renderHome({
      tools: [makeTool("claude-code", "Claude Code", { kind: "detected" })],
      domains: [makeDomain({ enabled: false })],
    });
    expect(screen.getByText("Nothing routing")).toBeTruthy();
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

describe("Home family pill vocabulary", () => {
  it("says why a family is dark instead of reusing the switched-off grey", () => {
    // Switched on, master on, certificate untrusted. This rendered "Not routed",
    // byte-identical to a family the user turned off - and then jumped to green
    // when they pressed Trust, teaching them the pill is approximate.
    renderHome({ caTrusted: false, domains: [makeDomain({ enabled: true })] });
    expect(screen.getAllByText("Needs trust").length).toBeGreaterThan(0);
    expect(screen.queryByText("Not routed")).toBeNull();
  });

  it("leaves master-off to the card, which says it once as a count", () => {
    // `master-off` is `enabled && !proxyOn`, and proxyOn is global, so it can
    // never distinguish one family from another: on the pill it is four
    // identical capsules restating the card directly above them. DESIGN.md:
    // "Card-owned states never print on a row."
    renderHome({
      proxyOn: false,
      tools: [makeTool("claude-code", "Claude Code", { kind: "connected" })],
    });
    expect(screen.queryByText("Waiting on routing")).toBeNull();
    expect(screen.getAllByText("Not routed").length).toBeGreaterThan(0);
    // The card carries it, once, and countably.
    expect(screen.getByText(/Off .+ waiting$/)).toBeTruthy();
  });

  it("still says Not routed when the user is the reason", () => {
    renderHome({
      proxyOn: true,
      domains: [makeDomain({ enabled: false })],
    });
    expect(screen.getAllByText("Not routed").length).toBeGreaterThan(0);
    expect(screen.queryByText("Waiting on routing")).toBeNull();
    expect(screen.queryByText("Needs trust")).toBeNull();
  });
});

describe("Home blocker ordering", () => {
  /** DESIGN.md, "Blockers outrank inventory": anything that explains why traffic
   *  is not flowing, and carries the fix, sits above the list. Both banners
   *  rendered *below* all four rows, so the two states where the user's tools
   *  are broken opened with green pills on top and the contradicting sentence
   *  underneath. */
  function ledgerHeading() {
    return screen.getByRole("heading", { name: "What routes through Gate" });
  }

  it("puts the stale-address notice above the ledger", () => {
    renderHome({
      staleAgentsHint: true,
      tools: [makeTool("claude-code", "Claude Code", { kind: "connected" })],
    });
    const banner = screen.getByText(/local address changed/);
    expect(
      banner.compareDocumentPosition(ledgerHeading()) & Node.DOCUMENT_POSITION_FOLLOWING,
    ).toBeTruthy();
  });

  it("puts the routing-change notice above the ledger", () => {
    renderHome({
      changeNotice: "on",
      tools: [makeTool("claude-code", "Claude Code", { kind: "connected" })],
    });
    const banner = screen.getByText(/Anything already open/);
    expect(
      banner.compareDocumentPosition(ledgerHeading()) & Node.DOCUMENT_POSITION_FOLLOWING,
    ).toBeTruthy();
  });

  it("keeps the certificate card above the ledger too", () => {
    renderHome({ caTrusted: false, domains: [makeDomain()] });
    const card = screen.getByText(/Apps with no gateway setting need/);
    expect(
      card.compareDocumentPosition(ledgerHeading()) & Node.DOCUMENT_POSITION_FOLLOWING,
    ).toBeTruthy();
  });

  it("tabs to Trust before its explainer, which sits lower", () => {
    // The sentence wraps, so "What's this?" renders below Trust. DOM order has
    // to match what the eye meets first or focus travels bottom-then-up.
    renderHome({ caTrusted: false, domains: [makeDomain()] });
    const trust = screen.getByRole("button", { name: "Trust" });
    const explain = screen.getByRole("button", { name: /What.s this/ });
    expect(
      trust.compareDocumentPosition(explain) & Node.DOCUMENT_POSITION_FOLLOWING,
    ).toBeTruthy();
  });
});

describe("Home command-line tools switch", () => {
  const NAME = "Route command-line tools through Gate";
  const withFamily = { tools: [makeTool("claude-code", "Claude Code", { kind: "connected" })] };

  it("toggles the shell-environment channel without touching routing", () => {
    const onToggleEnvExport = vi.fn();
    const onToggleProxy = vi.fn();
    renderHome({ ...withFamily, onToggleEnvExport, onToggleProxy });
    fireEvent.click(screen.getByRole("switch", { name: NAME }));
    expect(onToggleEnvExport).toHaveBeenCalledTimes(1);
    // It spans every family, so it must never move the master as a side effect.
    expect(onToggleProxy).not.toHaveBeenCalled();
  });

  it("reflects the backend's choice rather than the master's state", () => {
    renderHome({ ...withFamily, proxyOn: true, envExportOn: false });
    expect(screen.getByRole("switch", { name: NAME }).getAttribute("aria-checked")).toBe("false");
  });

  it("is absent where the channel cannot be separated from routing", () => {
    // Linux: the environment.d drop-in *is* the system proxy, so a switch here
    // could not honour itself. Better no control than one that lies.
    renderHome({ ...withFamily, envExportSeparable: false });
    expect(screen.queryByRole("switch", { name: NAME })).toBeNull();
  });

  it("sits below the ledger, not beside the master switch", () => {
    // The arrangement this replaced put the two switches 66px apart wearing the
    // same track in the same indigo, which said a machine-wide change to git and
    // curl was routing's equal. The ledger between them is the fix.
    renderHome(withFamily);
    const master = screen.getByRole("switch", { name: "Route through Gate" });
    const shell = screen.getByRole("switch", { name: NAME });
    const heading = screen.getByRole("heading", { name: "What routes through Gate" });
    expect(master.compareDocumentPosition(heading) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    expect(heading.compareDocumentPosition(shell) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
  });

  it("stays a line rather than a card, so the master keeps the weight", () => {
    renderHome(withFamily);
    const row = screen.getByText("Command-line tools").closest("div")!.parentElement!;
    expect(row.className).not.toContain("shadow-border");
  });

  it("is absent when there is no ledger to separate it from the master", () => {
    // With nothing installed the two switches would be adjacent again, which is
    // exactly the geometry that failed. Costs nothing: the panel this used to
    // live on was reachable only through a family row.
    renderHome({ tools: [], domains: [] });
    expect(screen.queryByRole("switch", { name: NAME })).toBeNull();
  });

  it("answers for reading on over a channel that cannot be live", () => {
    renderHome({ ...withFamily, proxyOn: false, envExportOn: true });
    const toggle = screen.getByRole("switch", { name: NAME });
    // The switch reports the stored choice, which survives routing being turned
    // off. It points at the card's status line rather than repeating it: the
    // master card owns `master-off` and says it once, countably.
    expect(toggle.getAttribute("aria-checked")).toBe("true");
    const described = document.getElementById(toggle.getAttribute("aria-describedby")!);
    expect(described?.textContent).toMatch(/^Off/);
    expect(screen.queryByText("Waiting on routing")).toBeNull();
  });

  it("carries its instruction in ink that clears AA", () => {
    // ink-4 measured 3.97:1 on white, the only text in the app below 4.5:1, and
    // this is two sentences about a machine-wide change to git and curl.
    renderHome(withFamily);
    const copy = screen.getByText(/for your whole/);
    expect(copy.className).toContain("text-gc-ink-3");
    expect(copy.className).not.toContain("text-gc-ink-4");
  });
});

describe("Home family roster", () => {
  it("names the members of the family named by exclusion", () => {
    renderHome({
      tools: [
        makeTool("opencode", "OpenCode", { kind: "connected" }, "your existing providers"),
        makeTool("openclaw", "OpenClaw", { kind: "connected" }, "your existing providers"),
      ],
    });
    // "Other tools" is the label on a filter; it is the one row a first-timer
    // cannot map to anything on their own machine.
    expect(screen.getByText("Other tools")).toBeTruthy();
    expect(screen.getByText("OpenCode · OpenClaw")).toBeTruthy();
  });

  it("joins with a middot, because a member name can contain a slash", () => {
    renderHome({
      tools: [makeTool("opencode", "OpenCode", { kind: "connected" }, "your existing providers")],
      domains: [makeDomain({ slug: "anthropic" })],
    });
    // "Claude Desktop / Cowork" is one member. A slash-joined roster would read
    // as two tools where there is one, anywhere a family holds such a name.
    expect(screen.queryByText(/OpenCode \/ /)).toBeNull();
  });

  it("stays off a family whose name already says what it covers", () => {
    renderHome({
      tools: [makeTool("claude-code", "Claude Code", { kind: "connected" })],
      domains: [makeDomain()],
    });
    // Same rule the panel's blurb follows: "Claude Code · Claude Desktop /
    // Cowork" under an h3 reading "Anthropic" costs a line for nearly the same
    // fact, and these are the rows that carry an exception instead.
    expect(screen.getByText("Claude")).toBeTruthy();
    expect(screen.queryByText("Claude Code · Claude Desktop / Cowork")).toBeNull();
  });

  it("yields the line to an exception, which already names a member", () => {
    renderHome({
      tools: [
        makeTool("opencode", "OpenCode", { kind: "error", message: "bad json" }, "your existing providers"),
        makeTool("openclaw", "OpenClaw", { kind: "connected" }, "your existing providers"),
      ],
    });
    // Both would put a third line on the row that already grew.
    expect(screen.getByText("OpenCode failed")).toBeTruthy();
    expect(screen.queryByText("OpenCode · OpenClaw")).toBeNull();
  });
});
