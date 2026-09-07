import { describe, it, expect, vi, beforeEach } from "vitest";
import {
  errorContext,
  initErrorContext,
  resetErrorContext,
  setRoutingContext,
} from "./errorContext";
import { installId, osName } from "./api";
import type { Status, Tool, Verdict } from "./api";

// The context exists to make a failure readable, and its constraint is that it
// stays anonymous: the two fields AG-603 lists that would identify someone -
// the installation NAME and the selected organization id - must never appear
// here, or `analytics.ts`'s anonymous-only posture and the disclosure's "no
// name, email, or account identifier" both become false.

vi.mock("./api", () => ({
  installId: vi.fn(async () => "gc_a1b2c3d4"),
  osName: vi.fn(async () => "Ubuntu 25.10"),
}));

function tool(slug: string, status: Status): Tool {
  return {
    slug,
    name: "CLI",
    product_name: slug,
    upstream_provider_name: "anthropic",
    default_upstream_url: "https://api.anthropic.com",
    config_location: null,
    status,
  };
}

function verdict(slug: string, over: Partial<Verdict> = {}): Verdict {
  return {
    slug,
    state: "on",
    reason: null,
    next_action: null,
    route_in_use: null,
    requested_route: null,
    ...over,
  };
}

beforeEach(() => {
  resetErrorContext();
  vi.clearAllMocks();
  vi.mocked(installId).mockResolvedValue("gc_a1b2c3d4");
  vi.mocked(osName).mockResolvedValue("Ubuntu 25.10");
});

describe("initErrorContext", () => {
  it("resolves the two fields that never change", async () => {
    await initErrorContext();
    expect(errorContext()).toMatchObject({
      install_id: "gc_a1b2c3d4",
      os_version: "Ubuntu 25.10",
    });
  });

  it("drops a field it cannot read rather than inventing one", async () => {
    // A hole in an error event beats a launch that blocked on a failed IPC.
    vi.mocked(osName).mockRejectedValue(new Error("command missing"));
    await initErrorContext();
    const ctx = errorContext();
    expect(ctx.install_id).toBe("gc_a1b2c3d4");
    expect("os_version" in ctx).toBe(false);
  });

  it("does not reject when both reads fail", async () => {
    vi.mocked(installId).mockRejectedValue(new Error("ipc down"));
    vi.mocked(osName).mockRejectedValue(new Error("ipc down"));
    await expect(initErrorContext()).resolves.toBeUndefined();
    expect(errorContext()).toEqual({});
  });
});

describe("setRoutingContext", () => {
  it("summarises the tools that are actually installed", () => {
    setRoutingContext({
      tools: [
        tool("codex", { kind: "connected" }),
        tool("claude_code", { kind: "detected" }),
        tool("opencode", { kind: "not_installed" }),
      ],
      verdicts: new Map(),
      routingOn: true,
      feedState: "live",
    });
    // Sorted, so two reports from one machine compare as strings; and no
    // `opencode`, which is not installed.
    expect(errorContext().tools_detected).toBe("claude_code,codex");
  });

  it("carries the reason alongside the state, where there is one", () => {
    setRoutingContext({
      tools: [],
      verdicts: new Map([
        ["codex", verdict("codex", { state: "needs_attention", reason: "reopen_required" })],
        ["claude_code", verdict("claude_code")],
      ]),
      routingOn: true,
      feedState: "live",
    });
    expect(errorContext().verdict_states).toBe(
      "claude_code:on,codex:needs_attention:reopen_required",
    );
  });

  it("reports the event-delivery state", () => {
    setRoutingContext({
      tools: [],
      verdicts: new Map(),
      routingOn: false,
      feedState: "reconnecting",
    });
    expect(errorContext().feed_state).toBe("reconnecting");
    expect(errorContext().routing_on).toBe(false);
  });

  it("omits routing rather than claiming Off when it was never read", () => {
    // The distinction `lib/groups.ts` and the unavailable Settings rows both
    // make: "off" and "could not be read" are different findings.
    setRoutingContext({
      tools: [],
      verdicts: new Map(),
      routingOn: null,
      feedState: "offline",
    });
    expect("routing_on" in errorContext()).toBe(false);
  });

  it("keeps the fields resolved at startup", async () => {
    await initErrorContext();
    setRoutingContext({
      tools: [],
      verdicts: new Map(),
      routingOn: true,
      feedState: "live",
    });
    expect(errorContext().install_id).toBe("gc_a1b2c3d4");
  });

  it("never carries a device name or an organization id", async () => {
    // The load-bearing one. AG-603's field list names both; sending either on
    // every failure would identify a stream the app promises is anonymous.
    await initErrorContext();
    setRoutingContext({
      tools: [tool("codex", { kind: "connected" })],
      verdicts: new Map([["codex", verdict("codex")]]),
      routingOn: true,
      feedState: "live",
    });
    const keys = Object.keys(errorContext());
    expect(keys).not.toContain("device_name");
    expect(keys).not.toContain("org_id");
    expect(keys.sort()).toEqual([
      "feed_state",
      "install_id",
      "os_version",
      "routing_on",
      "tools_detected",
      "verdict_states",
    ]);
  });
});
