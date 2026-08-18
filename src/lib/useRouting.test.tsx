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
  proxySetDomain: vi.fn(),
  proxyStatus: vi.fn(),
  proxyTrustCa: vi.fn(),
}));
vi.mock("./analytics", () => ({ track: vi.fn(), trackError: vi.fn() }));

import {
  connectTool,
  disconnectTool,
  listTools,
  proxySetDomain,
  proxyStatus,
  proxyTrustCa,
} from "./api";

const tool = (slug: string, status: Status): Tool => ({
  slug,
  name: slug,
  upstream_provider_name: "Anthropic",
  default_upstream_url: `https://gw.example/${slug}`,
  requires_upstream_credential: false,
  status,
});

const proxyState = (over: Partial<ProxyState> = {}): ProxyState => ({
  running: true,
  port: 8080,
  pac_port: null,
  ca_trusted: true,
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
});

const group = (members: GroupMember[]): Group => ({
  id: "anthropic",
  name: "Anthropic",
  switchLabel: "Route Anthropic through Gate",
  members,
  routed: members.filter((m) => m.routed).length,
  desired: members.filter((m) => m.desired).length,
  cascadeDesired: members.filter((m) => m.desired && !m.chat).length,
});

const configMember = (over: Partial<GroupMember> = {}): GroupMember => ({
  key: "claude-code",
  kind: "config",
  name: "Claude Code",
  routed: false,
  desired: false,
  attention: null,
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
        domain: {
          slug: "anthropic-api",
          display_name: "Anthropic API",
          hosts: ["api.anthropic.com"],
          upstream_url: "https://gw.example/anthropic",
          rewrite_prefixes: [],
          passthrough_prefixes: [],
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
