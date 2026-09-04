import { describe, expect, it } from "vitest";
import type {
  Account,
  Diagnostics,
  LaunchAtLoginStatus,
  OAuthStatus,
  ProviderState,
  ProxyState,
  RunningAgents,
  Tool,
} from "./api";
import { buildDiagnosticsReport, agentLine, expiryPhrase, toolStatusLine } from "./diagnosticsReport";

const NOW = new Date("2026-08-10T12:00:00.000Z");

const backend: Diagnostics = {
  os_name: "Ubuntu 25.10",
  os_kernel: "6.14.0-33-generic",
  arch: "x86_64",
  data_dir: "/home/x/.local/share/Gate Connect",
  ca_cert_path: "/home/x/.local/share/Gate Connect/proxy/ca-cert.pem",
  ca_cert_present: true,
  ca_nss_trusted: true,
  routing_intent: true,
  persisted_engine_proxy_url: "http://127.0.0.1:45981",
  relay_base_url: "http://127.0.0.1:45982",
  exported_proxy_url: "http://127.0.0.1:45981",
  system_proxy: "environment.d drop-in present",
};

const account: Account = {
  gateway_base_url: "https://gateway.constellationgate.ai",
  has_api_key: false,
  auth_mode: "oauth",
  billing_mode: "byok",
  org_id: "2f9c0000-0000-0000-0000-000000000000",
  org_name: "Constellation",
};

const oauth: OAuthStatus = {
  signed_in: true,
  email: "someone@example.com",
  expires_at_unix: Math.floor(NOW.getTime() / 1000) + 47 * 60,
};

const proxy: ProxyState = {
  running: true,
  port: 45981,
  pac_port: null,
  ca_trusted: true,
  relay_base_url: "http://127.0.0.1:45981",
  env_export_opted_in: true,
  env_export_separable: false,
  domains: [
    {
      slug: "anthropic",
      display_name: "Anthropic",
      hosts: ["api.anthropic.com"],
      upstream_url: "https://api.anthropic.com",
      rewrite_prefixes: [],
      passthrough_prefixes: [],
      enabled: true,
      supported: true,
    },
    {
      slug: "openrouter",
      display_name: "OpenRouter",
      hosts: ["openrouter.ai"],
      upstream_url: "https://openrouter.ai",
      rewrite_prefixes: [],
      passthrough_prefixes: [],
      enabled: false,
      supported: false,
    },
  ],
};

const providers: ProviderState[] = [
  {
    slug: "anthropic",
    display_name: "Anthropic",
    subtitle: "Claude",
    enabled: true,
    available: true,
    tool_slugs: ["claude-code"],
    domain_slugs: ["anthropic"],
    chat_domain_slugs: [],
  },
  {
    slug: "openai",
    display_name: "OpenAI",
    subtitle: "GPT",
    enabled: false,
    available: false,
    tool_slugs: ["codex"],
    domain_slugs: ["chatgpt"],
    chat_domain_slugs: [],
  },
];

const tools: Tool[] = [
  {
    slug: "claude-code",
    name: "Claude Code",
    product_name: "Claude Code",
    upstream_provider_name: "Anthropic",
    default_upstream_url: "https://api.anthropic.com",
    config_location: null,
    status: { kind: "connected" },
  },
  {
    slug: "codex",
    name: "Codex",
    product_name: "Codex",
    upstream_provider_name: "OpenAI",
    default_upstream_url: "https://api.openai.com",
    config_location: null,
    status: { kind: "drifted", reason: "base_url points elsewhere" },
  },
];

const launchAtLogin: LaunchAtLoginStatus = { enabled: true, pending_disable: false };

const NOW_UNIX = Math.floor(NOW.getTime() / 1000);

const agents: RunningAgents = {
  scanned_names: ["claude", "codex", "opencode"],
  agents: [
    {
      slug: "claude-code",
      name: "claude",
      can_reopen: false,
      pid: 12345,
      started_at_unix: NOW_UNIX - (2 * 3600 + 46 * 60),
      predates_routing: true,
    },
    {
      slug: "codex",
      name: "codex",
      can_reopen: false,
      pid: 23456,
      started_at_unix: NOW_UNIX - 60,
      predates_routing: false,
    },
  ],
};

function report(overrides: Partial<Parameters<typeof buildDiagnosticsReport>[0]> = {}) {
  return buildDiagnosticsReport({
    now: NOW,
    version: "1.4.2",
    platform: "linux",
    analyticsId: { kind: "id", value: "0199a1b2-c3d4-7e5f-8a9b-0c1d2e3f4a5b" },
    backend,
    account,
    oauth,
    proxy,
    providers,
    tools,
    launchAtLogin,
    clientsStale: false,
    agents,
    ...overrides,
  });
}

describe("buildDiagnosticsReport", () => {
  it("carries the app, machine and routing facts a support thread needs", () => {
    const text = report();
    expect(text).toContain("Gate Connect diagnostics");
    expect(text).toContain("generated 2026-08-10T12:00:00.000Z");
    expect(text).toContain("v1.4.2");
    expect(text).toContain("linux x86_64");
    expect(text).toContain("Ubuntu 25.10");
    expect(text).toContain("6.14.0-33-generic");
    expect(text).toContain("/home/x/.local/share/Gate Connect");
    expect(text).toContain("analytics id    0199a1b2-c3d4-7e5f-8a9b-0c1d2e3f4a5b");
    expect(text).toContain("https://gateway.constellationgate.ai");
    expect(text).toContain("someone@example.com");
    expect(text).toContain("session expires in 47m");
    expect(text).toContain("Constellation");
    expect(text).toContain("routing         on");
    expect(text).toContain("engine port     45981");
    expect(text).toContain("relay           http://127.0.0.1:45982");
    expect(text).toContain("env readback    http://127.0.0.1:45981");
    expect(text).toContain("system proxy    environment.d drop-in present");
    expect(text).toContain("restore intent  on");
    expect(text).toContain("launch at login on");
  });

  it("distinguishes analytics never having started from an id it could not read", () => {
    expect(report({ analyticsId: { kind: "disabled" } })).toContain("analytics id    disabled");
    expect(report({ analyticsId: { kind: "unavailable" } })).toContain("analytics id    unknown");
  });

  it("names each running tool, how long it has been up, and whether it predates routing", () => {
    const text = report();
    expect(text).toContain("[running agents]");
    // The scanned set is what makes an empty result readable, so it is part
    // of the section whether or not anything was found.
    expect(text).toContain("scanned for     claude, codex, opencode");
    expect(text).toContain(
      "claude          pid 12345, up 2h 46m, started 2026-08-10T09:14:00Z, predates routing",
    );
    expect(text).toContain("codex           pid 23456, up 1m, started 2026-08-10T11:59:00Z");
  });

  it("distinguishes 'none of those were running' from 'we could not look'", () => {
    expect(report({ agents: { scanned_names: ["claude"], agents: [] } })).toContain(
      "running         none",
    );
    expect(report({ agents: null })).toContain("scan            unknown");
  });

  it("lists every provider, tool and proxy domain with its state", () => {
    const text = report();
    expect(text).toContain("anthropic       on");
    expect(text).toContain("openai          off (unavailable)");
    expect(text).toContain("claude-code     routed");
    // The drift reason is the most useful string in the report when a tool is
    // misbehaving, so it has to survive into the paste.
    expect(text).toContain("codex           drifted: base_url points elsewhere");
    expect(text).toContain("openrouter      off (unsupported)");
  });

  it("never prints a credential, and says so", () => {
    const text = report({
      account: { ...account, auth_mode: "api_key", has_api_key: true },
    });
    expect(text).toContain("key stored      yes");
    expect(text).not.toContain("sk-gw-");
    expect(text).toContain("No keys, tokens or passwords are included in this report.");
  });

  it("names its holes instead of leaving blanks when the backend probe fails", () => {
    const text = report({ backend: null, version: "" });
    expect(text).toContain("version         unknown");
    expect(text).toContain("os              unknown");
    // The sections that depend on the backend snapshot drop out rather than
    // printing a confident wrong answer.
    expect(text).not.toContain("restore intent");
    // Everything the popover itself knows still renders.
    expect(text).toContain("routing         on");
  });

  it("flags a trusted certificate whose file has gone missing", () => {
    const text = report({ backend: { ...backend, ca_cert_present: false } });
    expect(text).toContain("cert file       MISSING on disk");
  });

  it("flags a CA the browser's own store is missing", () => {
    // The certificate line still says trusted, because the OS store holds it.
    // Only this line explains why Chrome rejects what Firefox accepts.
    const text = report({ backend: { ...backend, ca_nss_trusted: false } });
    expect(text).toContain("certificate     trusted");
    expect(text).toContain("browser store   CA MISSING (chromium)");
  });

  it("says nothing about the browser store where the question does not apply", () => {
    const text = report({ backend: { ...backend, ca_nss_trusted: null } });
    expect(text).not.toContain("browser store");
  });

  it("survives a first-run popover with no account and no proxy", () => {
    const text = report({ account: null, proxy: null, oauth: null, launchAtLogin: null });
    expect(text).toContain("account         none configured");
    expect(text).toContain("proxy           unavailable on this platform");
    expect(text).toContain("launch at login unknown");
    expect(text).not.toContain("[proxy domains]");
  });
});

describe("expiryPhrase", () => {
  it("reads as a span in both directions, and n/a when signed out", () => {
    const unix = Math.floor(NOW.getTime() / 1000);
    expect(expiryPhrase(unix + 90 * 60, NOW)).toBe("in 1h 30m");
    expect(expiryPhrase(unix - 20 * 60, NOW)).toBe("20m ago");
    expect(expiryPhrase(0, NOW)).toBe("n/a");
  });
});

describe("agentLine", () => {
  it("switches to days for the process that has been up all weekend", () => {
    const line = agentLine(
      {
        slug: "claude-code",
        name: "Claude",
        can_reopen: false,
        pid: 9,
        started_at_unix: NOW_UNIX - (3 * 86400 + 4 * 3600),
        predates_routing: true,
      },
      NOW,
    );
    expect(line).toContain("up 3d 4h");
  });

  it("says so rather than printing an epoch when the platform withheld the start time", () => {
    const line = agentLine(
      {
        slug: "codex",
        name: "codex",
        can_reopen: false,
        pid: 9,
        started_at_unix: 0,
        predates_routing: false,
      },
      NOW,
    );
    expect(line).toContain("start time unavailable");
    expect(line).not.toContain("1970");
  });
});

describe("toolStatusLine", () => {
  it("keeps the reason attached to the states that carry one", () => {
    expect(toolStatusLine({ kind: "not_installed" })).toBe("not installed");
    expect(toolStatusLine({ kind: "detected" })).toBe("installed, not routed");
    expect(toolStatusLine({ kind: "error", message: "keychain denied" })).toBe(
      "error: keychain denied",
    );
  });
});
