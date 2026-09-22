import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { Mock } from "vitest";
import { act, cleanup, render } from "@testing-library/react";
import type { ProxyState, Status, Tool } from "./api";
import { useRouting, FamilyCascadeError } from "./useRouting";
import type { Group, GroupMember } from "./groups";

vi.mock("./api", () => ({
  connectTool: vi.fn(),
  disconnectTool: vi.fn(),
  listTools: vi.fn(),
  proxyDisable: vi.fn(),
  proxyEnable: vi.fn(),
  proxySetDomain: vi.fn(),
  proxySetEnvExport: vi.fn(),
  proxyStatus: vi.fn(),
  proxyTrustCa: vi.fn(),
  proxyUntrustCa: vi.fn(),
  hermesUpstreamCoverage: vi.fn(),
}));
vi.mock("./analytics", () => ({ track: vi.fn(), trackError: vi.fn() }));

import {
  connectTool,
  disconnectTool,
  listTools,
  proxyDisable,
  proxyEnable,
  proxySetDomain,
  proxySetEnvExport,
  proxyStatus,
  proxyTrustCa,
  proxyUntrustCa,
  hermesUpstreamCoverage,
} from "./api";

const tool = (slug: string, status: Status): Tool => ({
  slug,
  name: slug,
  product_name: slug,
  upstream_provider_name: "Anthropic",
  client: "claude-code",
  scope: "client",
  credential: "brokered",
  default_upstream_url: `https://gw.example/${slug}`,
  config_location: null,
  status,
});

const proxyState = (over: Partial<ProxyState> = {}): ProxyState => ({
  running: true,
  port: 8080,
  pac_port: null,
  ca_trusted: true,
  ca_nss_trust: null,
  browser_proxy_channel: false,
  relay_base_url: "http://127.0.0.1:45981",
  env_export_opted_in: false,
  env_export_separable: true,
  domains: [],
  ...over,
});

/** Drives the hook from a component, since it owns state and effects. */
function harness(tools: Tool[], proxy: ProxyState | null) {
  const api: { current: ReturnType<typeof useRouting> | null } = { current: null };
  const onSnapshot = vi.fn();
  const onError = vi.fn();

  function Probe() {
    api.current = useRouting({ tools, proxy, onSnapshot, onError });
    return null;
  }
  render(<Probe />);
  return { api, onSnapshot, onError };
}

beforeEach(() => {
  vi.clearAllMocks();
  // `clearAllMocks` clears calls but keeps implementations, and one test below
  // parks `connectTool` on a promise it never resolves. Without an explicit
  // reset that implementation leaks into every later test, which then hangs on
  // its first member and reports a call count nobody can explain.
  (connectTool as Mock).mockReset();
  (disconnectTool as Mock).mockReset();
  (listTools as Mock).mockResolvedValue([]);
  (proxyStatus as Mock).mockResolvedValue(proxyState());
  (proxyTrustCa as Mock).mockResolvedValue(proxyState());
  (proxySetDomain as Mock).mockResolvedValue(proxyState());
  (proxyEnable as Mock).mockResolvedValue(proxyState());
  (proxyDisable as Mock).mockResolvedValue(proxyState({ running: false }));
  (proxySetEnvExport as Mock).mockResolvedValue(proxyState());
  (proxyUntrustCa as Mock).mockResolvedValue(proxyState({ ca_trusted: false }));
});
afterEach(cleanup);

describe("useRouting: the drift gate", () => {
  it("will not adopt a drifted config without asking", async () => {
    // The whole point of the gate: connecting rewrites settings somebody
    // hand-wrote, so it must not happen on the first click.
    const { api } = harness([tool("codex", { kind: "drifted", reason: "API base URL" })], proxyState());

    await act(async () => {
      void api.current!.setAppRouted("codex", true);
    });

    expect(connectTool).not.toHaveBeenCalled();
    expect(api.current!.prompt).toEqual({
      kind: "drift",
      slug: "codex",
      name: "codex",
      existingConfig: "API base URL",
    });
  });

  it("connects once the review is approved", async () => {
    const { api } = harness([tool("codex", { kind: "drifted", reason: "API base URL" })], proxyState());

    await act(async () => {
      void api.current!.setAppRouted("codex", true);
    });
    await act(async () => {
      api.current!.resolvePrompt(true);
    });

    expect(connectTool).toHaveBeenCalledWith("codex", "https://gw.example/codex");
  });

  it("leaves the config alone when the review is declined", async () => {
    const { api, onError } = harness(
      [tool("codex", { kind: "drifted", reason: "API base URL" })],
      proxyState(),
    );

    await act(async () => {
      void api.current!.setAppRouted("codex", true);
    });
    await act(async () => {
      api.current!.resolvePrompt(false);
    });

    expect(connectTool).not.toHaveBeenCalled();
    // Declining is an answer, not a failure.
    expect(onError).not.toHaveBeenCalled();
  });

  it("skips the gate when the caller already reviewed", async () => {
    const { api } = harness([tool("codex", { kind: "drifted", reason: "API base URL" })], proxyState());

    await act(async () => {
      await api.current!.setAppRouted("codex", true, true);
    });

    expect(api.current!.prompt).toBeNull();
    expect(connectTool).toHaveBeenCalled();
  });

  it("does not gate turning a drifted tool off", async () => {
    // Disconnecting restores what was there, so there is nothing to review.
    const { api } = harness([tool("codex", { kind: "drifted", reason: "API base URL" })], proxyState());

    await act(async () => {
      await api.current!.setAppRouted("codex", false);
    });

    expect(api.current!.prompt).toBeNull();
    expect(disconnectTool).toHaveBeenCalledWith("codex");
  });
});

describe("useRouting: the certificate gate", () => {
  it("asks before trusting, and does not connect if refused", async () => {
    const { api } = harness([tool("claude-code", { kind: "detected" })], proxyState({ ca_trusted: false }));

    await act(async () => {
      void api.current!.setAppRouted("claude-code", true);
    });
    expect(api.current!.prompt).toEqual({ kind: "trust" });

    await act(async () => {
      api.current!.resolvePrompt(false);
    });
    expect(proxyTrustCa).not.toHaveBeenCalled();
    expect(connectTool).not.toHaveBeenCalled();
  });

  it("does not ask when the CA is already trusted", async () => {
    const { api } = harness([tool("claude-code", { kind: "detected" })], proxyState({ ca_trusted: true }));

    await act(async () => {
      await api.current!.setAppRouted("claude-code", true);
    });

    expect(api.current!.prompt).toBeNull();
    expect(proxyTrustCa).not.toHaveBeenCalled();
    expect(connectTool).toHaveBeenCalled();
  });

  it("does not ask on a platform with no proxy subsystem", async () => {
    const { api } = harness([tool("claude-code", { kind: "detected" })], null);

    await act(async () => {
      await api.current!.setAppRouted("claude-code", true);
    });

    expect(api.current!.prompt).toBeNull();
    expect(connectTool).toHaveBeenCalled();
  });
});

describe("useRouting: re-sync and failures", () => {
  it("re-reads backend truth even when the action fails", async () => {
    // Connecting can flip a provider headline and auto-start the engine, so the
    // rendered state must come from the backend, not from what we asked for.
    (connectTool as Mock).mockRejectedValue(new Error("nope"));
    const { api, onSnapshot, onError } = harness(
      [tool("claude-code", { kind: "detected" })],
      proxyState(),
    );

    await act(async () => {
      await api.current!.setAppRouted("claude-code", true);
    });

    expect(onError).toHaveBeenCalled();
    expect(onSnapshot).toHaveBeenCalled();
    expect(listTools).toHaveBeenCalled();
    expect(api.current!.busy).toBe(false);
  });

  it("ignores a second action while one is in flight", async () => {
    let release: (() => void) | undefined;
    (connectTool as Mock).mockImplementation(
      () => new Promise<void>((r) => { release = () => r(); }),
    );
    const { api } = harness([tool("claude-code", { kind: "detected" })], proxyState());

    await act(async () => {
      void api.current!.setAppRouted("claude-code", true);
    });
    await act(async () => {
      await api.current!.setAppRouted("claude-code", false);
    });

    expect(disconnectTool).not.toHaveBeenCalled();
    await act(async () => {
      release?.();
    });
  });
});

describe("useRouting: domains", () => {
  it("routes a domain without a drift gate", async () => {
    // A domain has no hand-written config to preserve, only an enabled flag.
    const { api } = harness([], proxyState());

    await act(async () => {
      await api.current!.setDomainRouted("claude-web", true);
    });

    expect(api.current!.prompt).toBeNull();
    expect(proxySetDomain).toHaveBeenCalledWith("claude-web", true);
  });

  it("still asks about the certificate when enabling one", async () => {
    const { api } = harness([], proxyState({ ca_trusted: false }));

    await act(async () => {
      void api.current!.setDomainRouted("claude-web", true);
    });

    expect(api.current!.prompt).toEqual({ kind: "trust" });
    expect(proxySetDomain).not.toHaveBeenCalled();
  });

  it("does not ask when turning one off", async () => {
    const { api } = harness([], proxyState({ ca_trusted: false }));

    await act(async () => {
      await api.current!.setDomainRouted("claude-web", false);
    });

    expect(api.current!.prompt).toBeNull();
    expect(proxySetDomain).toHaveBeenCalledWith("claude-web", false);
  });

  it("starts the engine when it is off, because a domain routes through it", async () => {
    // `proxy_set_domain` records the flag and nothing else - unlike
    // `connect_tool`, which starts the engine itself. Without this the switch
    // writes intent, routes nothing, and the window offers no way back.
    const { api } = harness([], proxyState({ running: false }));

    await act(async () => {
      await api.current!.setDomainRouted("claude-web", true);
    });

    expect(proxyEnable).toHaveBeenCalled();
    expect(proxySetDomain).toHaveBeenCalledWith("claude-web", true);
  });

  it("does not start the engine to turn a domain off", async () => {
    const { api } = harness([], proxyState({ running: false }));

    await act(async () => {
      await api.current!.setDomainRouted("claude-web", false);
    });

    expect(proxyEnable).not.toHaveBeenCalled();
  });
});

describe("useRouting: the shell environment channel", () => {
  it("sets it without touching the engine", async () => {
    // Its own action for a reason: this decides whether the proxy is written into
    // the user's environment, which reaches git and curl, not just the AI tools.
    const { api } = harness([], proxyState());

    await act(async () => {
      await api.current!.setEnvExport(true);
    });

    expect(proxySetEnvExport).toHaveBeenCalledWith(true);
    expect(proxyEnable).not.toHaveBeenCalled();
    expect(proxyDisable).not.toHaveBeenCalled();
  });
});

describe("useRouting: removing the certificate", () => {
  it("confirms first", async () => {
    const { api } = harness([], proxyState());

    let done: Promise<void> | null = null;
    await act(async () => {
      done = api.current!.untrustCa();
    });
    expect(api.current!.prompt).toEqual({ kind: "untrust" });
    expect(proxyUntrustCa).not.toHaveBeenCalled();

    await act(async () => {
      api.current!.resolvePrompt(true);
      await done;
    });
    expect(proxyUntrustCa).toHaveBeenCalled();
  });

  it("leaves the certificate alone when the confirmation is refused", async () => {
    const { api, onError } = harness([], proxyState());

    let done: Promise<void> | null = null;
    await act(async () => {
      done = api.current!.untrustCa();
    });
    await act(async () => {
      api.current!.resolvePrompt(false);
      await done;
    });

    expect(proxyUntrustCa).not.toHaveBeenCalled();
    expect(onError).not.toHaveBeenCalled();
  });
});

describe("useRouting: the certificate gate a cascade asks once", () => {
  it("answers true without asking when the CA is already trusted", async () => {
    const { api } = harness([], proxyState({ ca_trusted: true }));

    let answered: boolean | undefined;
    await act(async () => {
      answered = await api.current!.confirmCaTrusted();
    });

    expect(answered).toBe(true);
    expect(api.current!.prompt).toBeNull();
    expect(proxyTrustCa).not.toHaveBeenCalled();
  });

  it("answers false when the person declines, and writes nothing", async () => {
    // False is what abandons the cascade. Answering true would run the members
    // anyway, and each would raise its own copy of the dialog just declined -
    // which is the bug this exists to fix, seen from the other side.
    const { api, onError } = harness([], proxyState({ ca_trusted: false }));

    let answered: Promise<boolean> | undefined;
    await act(async () => {
      answered = api.current!.confirmCaTrusted();
    });
    expect(api.current!.prompt).toEqual({ kind: "trust" });
    await act(async () => {
      api.current!.resolvePrompt(false);
    });

    expect(await answered!).toBe(false);
    expect(proxyTrustCa).not.toHaveBeenCalled();
    // A declined gate is an answer, not a failure.
    expect(onError).not.toHaveBeenCalled();
  });

  it("installs once and answers true when the person accepts", async () => {
    const { api } = harness([], proxyState({ ca_trusted: false }));

    let answered: Promise<boolean> | undefined;
    await act(async () => {
      answered = api.current!.confirmCaTrusted();
    });
    await act(async () => {
      api.current!.resolvePrompt(true);
    });

    expect(await answered!).toBe(true);
    expect(proxyTrustCa).toHaveBeenCalledTimes(1);
  });
});

const group = (members: GroupMember[]): Group => ({
  id: "claude-code",
  name: "Claude Code",
  band: "anthropic",
  switchLabel: "Route Claude Code through Gate",
  members,
  routed: members.filter((m) => m.routed).length,
  desired: members.filter((m) => m.desired).length,
  cascadeDesired: members.filter((m) => m.desired && m.cascade).length,
  // The app switch's own predicate, spelled out rather than imported so a
  // change to it fails these tests loudly instead of quietly rewriting what
  // they assert. `governingMembers`' fallback included: a section with no
  // brokered member is described by the members it has.
  switchOn: governing(members).some(isIntended),
  switchDesired: governing(members).filter(isIntended).length,
});

const configMember = (over: Partial<GroupMember> = {}): GroupMember => ({
  key: "claude-code",
  kind: "config",
  name: "Claude Code",
  routed: false,
  desired: false,
  attention: null,
  client: "claude-code",
  scope: "client",
  credential: "brokered",
  cascade: true,
  tool: tool("claude-code", { kind: "detected" }),
  ...over,
});

describe("useRouting: the family cascade", () => {
  it("trusts the certificate once, before the first member", async () => {
    // A config member's connect auto-enables the engine, which trusts the CA.
    // Sprung from member three, that system dialog reads as malware.
    const { api } = harness([], proxyState({ ca_trusted: false }));
    const g = group([configMember(), configMember({ key: "codex", name: "Codex" })]);

    await act(async () => {
      void api.current!.setFamilyRouted(g, true);
    });
    expect(api.current!.prompt).toEqual({ kind: "trust" });
    expect(connectTool).not.toHaveBeenCalled();

    await act(async () => {
      api.current!.resolvePrompt(true);
    });

    expect((proxyTrustCa as Mock).mock.calls).toHaveLength(1);
    expect(connectTool).toHaveBeenCalledTimes(2);
  });

  it("aborts the whole family when the certificate is refused", async () => {
    const { api, onError } = harness([], proxyState({ ca_trusted: false }));
    const g = group([configMember()]);

    await act(async () => {
      void api.current!.setFamilyRouted(g, true);
    });
    await act(async () => {
      api.current!.resolvePrompt(false);
    });

    expect(connectTool).not.toHaveBeenCalled();
    // The user chose this on our own screen; it is an answer, not a failure.
    expect(onError).not.toHaveBeenCalled();
  });

  it("keeps going after one member fails, and names the ones that did", async () => {
    // One failure used to abort the loop and report "couldn't connect this
    // tool", naming nobody and hiding the partial success.
    (connectTool as Mock).mockImplementation(async (slug: string) => {
      if (slug === "codex") throw new Error("nope");
    });
    const { api, onError } = harness([], proxyState());
    const g = group([
      configMember(),
      configMember({ key: "codex", name: "Codex", tool: tool("codex", { kind: "detected" }) }),
      configMember({ key: "opencode", name: "OpenCode", tool: tool("opencode", { kind: "detected" }) }),
    ]);

    await act(async () => {
      await api.current!.setFamilyRouted(g, true);
    });

    expect(connectTool).toHaveBeenCalledTimes(3);
    const reported = onError.mock.calls[0][0] as FamilyCascadeError;
    expect(reported).toBeInstanceOf(FamilyCascadeError);
    expect(reported.names).toEqual(["Codex"]);
    expect(reported.attempted).toBe(3);
  });

  it("reports a change when some members succeeded", async () => {
    // The follow-up sequence keys off this: configs were written, so a running
    // app is now stale even though one member failed.
    (connectTool as Mock).mockImplementationOnce(async () => {
      throw new Error("nope");
    });
    const { api } = harness([], proxyState());
    const g = group([
      configMember(),
      configMember({ key: "codex", name: "Codex", tool: tool("codex", { kind: "detected" }) }),
    ]);

    let changed: boolean | undefined;
    await act(async () => {
      changed = await api.current!.setFamilyRouted(g, true);
    });

    expect(changed).toBe(true);
  });

  it("does nothing, and says so, when every member is already there", async () => {
    const { api } = harness([], proxyState());
    const g = group([configMember({ desired: true })]);

    let changed: boolean | undefined;
    await act(async () => {
      changed = await api.current!.setFamilyRouted(g, true);
    });

    expect(changed).toBe(false);
    expect(connectTool).not.toHaveBeenCalled();
    // Not even a certificate prompt: there is nothing to route.
    expect(api.current!.prompt).toBeNull();
  });

  it("routes domain members through the proxy, not through connect", async () => {
    const { api } = harness([], proxyState());
    const g = group([
      {
        key: "anthropic-api",
        kind: "proxy",
        name: "Anthropic API",
        routed: false,
        desired: false,
        attention: null,
        client: "claude-desktop",
        scope: "host",
        credential: "brokered",
        cascade: true,
        domain: {
          slug: "anthropic-api",
          display_name: "Anthropic API",
          hosts: ["api.anthropic.com"],
          upstream_url: "https://gw.example/anthropic",
          rewrite_prefixes: [],
          passthrough_prefixes: [],
          client: "claude-desktop",
          credential: "brokered",
          scope: "host",
          enabled: false,
          supported: true,
        },
      },
    ]);

    await act(async () => {
      await api.current!.setFamilyRouted(g, true);
    });

    expect(proxySetDomain).toHaveBeenCalledWith("anthropic-api", true);
    expect(connectTool).not.toHaveBeenCalled();
  });
});

describe("useRouting: remembering a failed write", () => {
  /**
   * A failed write leaves nothing on disk, so no sweep can find it. Without this
   * the row went on asserting whatever it said before the click, and the only
   * report was a transient banner.
   */
  it("marks the slug whose write failed", async () => {
    const { api } = harness([tool("codex", { kind: "detected" })], proxyState());
    (connectTool as Mock).mockRejectedValue("config file locked");

    await act(async () => {
      await api.current!.setAppRouted("codex", true);
    });

    expect(api.current!.writeFailures.has("codex")).toBe(true);
  });

  it("clears the mark once a write for that slug succeeds", async () => {
    const { api } = harness([tool("codex", { kind: "detected" })], proxyState());
    (connectTool as Mock).mockRejectedValue("config file locked");
    await act(async () => {
      await api.current!.setAppRouted("codex", true);
    });
    expect(api.current!.writeFailures.has("codex")).toBe(true);

    (connectTool as Mock).mockResolvedValue({ kind: "connected" });
    await act(async () => {
      await api.current!.setAppRouted("codex", true);
    });

    expect(api.current!.writeFailures.has("codex")).toBe(false);
  });

  /** Declining a gate is an answer, not a failure. Marking the row would tell
   * the user their own choice went wrong. */
  it("does not mark a slug when the user declined the review", async () => {
    const { api } = harness(
      [tool("codex", { kind: "drifted", reason: "API base URL: https://api.openai.com/v1" })],
      proxyState(),
    );

    let pending: Promise<boolean>;
    await act(async () => {
      pending = api.current!.setAppRouted("codex", true);
    });
    await act(async () => {
      api.current!.resolvePrompt(false);
      await pending!;
    });

    expect(api.current!.writeFailures.has("codex")).toBe(false);
    expect(connectTool).not.toHaveBeenCalled();
  });

  it("keeps one slug's failure off another slug's row", async () => {
    const { api } = harness(
      [tool("codex", { kind: "detected" }), tool("claude-code", { kind: "detected" })],
      proxyState(),
    );
    (connectTool as Mock).mockRejectedValueOnce("config file locked");
    await act(async () => {
      await api.current!.setAppRouted("codex", true);
    });
    (connectTool as Mock).mockResolvedValue({ kind: "connected" });
    await act(async () => {
      await api.current!.setAppRouted("claude-code", true);
    });

    expect(api.current!.writeFailures.has("codex")).toBe(true);
    expect(api.current!.writeFailures.has("claude-code")).toBe(false);
  });
});

describe("useRouting: Hermes and the provider it talks to", () => {
  const covered = { switched_off: [], unknown: [] };
  const off = {
    switched_off: [["openrouter.ai", "openrouter"]] as [string, string][],
    unknown: [],
  };

  beforeEach(() => {
    (hermesUpstreamCoverage as Mock).mockResolvedValue(covered);
  });

  it("asks before routing Hermes at a provider Gate is not inspecting", async () => {
    // `hermes` is a one-member section, so its switch routes the tool and
    // intercepts nothing. Claude and ChatGPT cannot reach this state: their
    // sections bundle the provider rows with the tool.
    (hermesUpstreamCoverage as Mock).mockResolvedValue(off);
    const { api } = harness([tool("hermes", { kind: "detected" })], proxyState());

    await act(async () => {
      void api.current!.setAppRouted("hermes", true);
    });

    expect(api.current!.prompt).toEqual({
      kind: "hermes-provider",
      // Carries the row's display name, which is what the dialog shows. With
      // no catalog row to name it falls back to the host - see the lookup in
      // `setAppRouted`; this harness's `proxyState()` has no domains, so this
      // is that fallback.
      domains: [
        { name: "openrouter.ai", host: "openrouter.ai", slug: "openrouter" },
      ],
    });
    expect(connectTool).not.toHaveBeenCalled();
    expect(proxySetDomain).not.toHaveBeenCalled();
  });

  it("turns the provider on once the person says yes", async () => {
    (hermesUpstreamCoverage as Mock).mockResolvedValue(off);
    const { api } = harness([tool("hermes", { kind: "detected" })], proxyState());

    await act(async () => {
      void api.current!.setAppRouted("hermes", true);
    });
    await act(async () => {
      api.current!.resolvePrompt(true);
    });

    expect(connectTool).toHaveBeenCalledWith("hermes", "https://gw.example/hermes");
    expect(proxySetDomain).toHaveBeenCalledWith("openrouter", true);
  });

  it("writes nothing at all when the provider is declined", async () => {
    // Cancel means cancel, like the drift and certificate gates. An earlier
    // version connected Hermes anyway and skipped only the domain, leaving it
    // routed with its provider uninspected - which is the state this whole
    // dialog exists to prevent, reporting Protected while every request
    // tunnels past unseen. The gate runs before `connectTool`, so declining
    // leaves the switch off and the config untouched.
    (hermesUpstreamCoverage as Mock).mockResolvedValue(off);
    const { api } = harness([tool("hermes", { kind: "detected" })], proxyState());

    await act(async () => {
      void api.current!.setAppRouted("hermes", true);
    });
    await act(async () => {
      api.current!.resolvePrompt(false);
    });

    expect(connectTool).not.toHaveBeenCalled();
    expect(proxySetDomain).not.toHaveBeenCalled();
    // And it is not reported as a failure: declining is an answer, so the row
    // must not come back marked as a failed write.
    expect(api.current!.writeFailures.has("hermes")).toBe(false);
  });

  it("stays quiet when the provider is already inspected", async () => {
    // A dialog that fires on every Hermes toggle for someone whose provider is
    // already on is noise, and it would train people to dismiss the one case
    // that matters.
    const { api } = harness([tool("hermes", { kind: "detected" })], proxyState());

    await act(async () => {
      await api.current!.setAppRouted("hermes", true);
    });

    expect(api.current!.prompt).toBeNull();
    expect(connectTool).toHaveBeenCalledWith("hermes", "https://gw.example/hermes");
  });

  it("stays quiet for an upstream no domain claims", async () => {
    // Bedrock, or a self-hosted endpoint. No switch fixes it, so a dialog
    // offering one would be a lie. `Coverage` keeps these in `unknown`.
    (hermesUpstreamCoverage as Mock).mockResolvedValue({
      switched_off: [],
      unknown: ["bedrock-runtime.us-east-1.amazonaws.com"],
    });
    const { api } = harness([tool("hermes", { kind: "detected" })], proxyState());

    await act(async () => {
      await api.current!.setAppRouted("hermes", true);
    });

    expect(api.current!.prompt).toBeNull();
    expect(connectTool).toHaveBeenCalled();
  });

  it("connects anyway when the coverage read fails", async () => {
    // The reading is advice, not a gate. A backend that cannot answer must not
    // cost the person the toggle they asked for.
    (hermesUpstreamCoverage as Mock).mockRejectedValue(new Error("no config"));
    const { api } = harness([tool("hermes", { kind: "detected" })], proxyState());

    await act(async () => {
      await api.current!.setAppRouted("hermes", true);
    });

    expect(api.current!.prompt).toBeNull();
    expect(connectTool).toHaveBeenCalled();
  });

  it("asks nothing when Hermes is being turned off", async () => {
    (hermesUpstreamCoverage as Mock).mockResolvedValue(off);
    const { api } = harness([tool("hermes", { kind: "connected" })], proxyState());

    await act(async () => {
      await api.current!.setAppRouted("hermes", false);
    });

    expect(hermesUpstreamCoverage).not.toHaveBeenCalled();
    expect(api.current!.prompt).toBeNull();
  });
});

describe("useRouting: OpenCode and the environment channel", () => {
  it("asks before turning the machine-wide variables on with it", async () => {
    // OpenCode's own rewrite covers the providers it found at connect; the
    // variables cover whatever else it sends - and those reach git, curl and npm
    // too. The click does two things, so it has to say so before it does either.
    const { api } = harness(
      [tool("opencode", { kind: "detected" })],
      proxyState({ env_export_opted_in: false }),
    );

    await act(async () => {
      void api.current!.setAppRouted("opencode", true);
    });

    expect(api.current!.prompt).toEqual({ kind: "opencode-env" });
    expect(connectTool).not.toHaveBeenCalled();
    expect(proxySetEnvExport).not.toHaveBeenCalled();
  });

  it("turns both on once the coupling is accepted", async () => {
    const { api } = harness(
      [tool("opencode", { kind: "detected" })],
      proxyState({ env_export_opted_in: false }),
    );

    await act(async () => {
      void api.current!.setAppRouted("opencode", true);
    });
    await act(async () => {
      api.current!.resolvePrompt(true);
    });

    expect(connectTool).toHaveBeenCalledWith("opencode", "https://gw.example/opencode");
    expect(proxySetEnvExport).toHaveBeenCalledWith(true);
  });

  it("writes nothing at all when the coupling is declined", async () => {
    // Declining must not leave OpenCode connected against a channel the user
    // just refused: the config write would claim a route it does not have.
    const { api, onError } = harness(
      [tool("opencode", { kind: "detected" })],
      proxyState({ env_export_opted_in: false }),
    );

    await act(async () => {
      void api.current!.setAppRouted("opencode", true);
    });
    await act(async () => {
      api.current!.resolvePrompt(false);
    });

    expect(connectTool).not.toHaveBeenCalled();
    expect(proxySetEnvExport).not.toHaveBeenCalled();
    expect(onError).not.toHaveBeenCalled();
  });

  it("stays quiet when the channel is already on", async () => {
    // The default for most users. A dialog announcing a side effect that has
    // already happened is noise, and it would fire on every OpenCode toggle.
    const { api } = harness(
      [tool("opencode", { kind: "detected" })],
      proxyState({ env_export_opted_in: true }),
    );

    await act(async () => {
      void api.current!.setAppRouted("opencode", true);
    });

    expect(api.current!.prompt).toBeNull();
    expect(connectTool).toHaveBeenCalledWith("opencode", "https://gw.example/opencode");
    expect(proxySetEnvExport).not.toHaveBeenCalled();
  });

  it("leaves every other tool's switch a one-step action", async () => {
    const { api } = harness(
      [tool("claude-code", { kind: "detected" })],
      proxyState({ env_export_opted_in: false }),
    );

    await act(async () => {
      void api.current!.setAppRouted("claude-code", true);
    });

    expect(api.current!.prompt).toBeNull();
    expect(proxySetEnvExport).not.toHaveBeenCalled();
  });

  it("does not ask on the way off", async () => {
    // Turning OpenCode off does not turn the channel off: other tools ride it,
    // and the Terminal tools row is its own switch.
    const { api } = harness(
      [tool("opencode", { kind: "connected" })],
      proxyState({ env_export_opted_in: false }),
    );

    await act(async () => {
      void api.current!.setAppRouted("opencode", false);
    });

    expect(api.current!.prompt).toBeNull();
    expect(disconnectTool).toHaveBeenCalledWith("opencode");
    expect(proxySetEnvExport).not.toHaveBeenCalled();
  });
});

/** `governingMembers`' rule, restated for the fixtures above. */
const governing = (members: GroupMember[]): GroupMember[] => {
  const brokered = members.filter((m) => m.cascade);
  return brokered.length > 0 ? brokered : members;
};
/** `intended`'s rule: asked for, or drifted while asked for. */
const isIntended = (m: GroupMember): boolean => m.desired || m.attention === "drifted";
