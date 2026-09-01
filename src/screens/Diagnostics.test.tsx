import { afterEach, describe, expect, it, vi } from "vitest";
import type { Mock } from "vitest";
import { cleanup, render, screen, fireEvent, waitFor, act } from "@testing-library/react";
import type { Account, Diagnostics as BackendDiagnostics, ProxyState } from "../lib/api";
import { Diagnostics } from "./Diagnostics";

vi.mock("../lib/platform", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../lib/platform")>()),
  usePlatform: () => "linux",
}));
vi.mock("../lib/api", () => ({
  diagnostics: vi.fn(),
  launchAtLoginStatus: vi.fn(),
  routedClientsStale: vi.fn(),
  runningAgents: vi.fn(),
}));
vi.mock("../lib/analytics", () => ({
  track: vi.fn(),
  trackError: vi.fn(),
  analyticsId: () => ({ kind: "id", value: "0199a1b2-c3d4-7e5f-8a9b-0c1d2e3f4a5b" }),
}));
import {
  diagnostics as fetchDiagnostics,
  launchAtLoginStatus,
  routedClientsStale,
  runningAgents,
} from "../lib/api";

const backend: BackendDiagnostics = {
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
  has_api_key: true,
  auth_mode: "api_key",
  billing_mode: "byok",
  org_id: null,
  org_name: null,
};

const proxy: ProxyState = {
  running: true,
  port: 45981,
  pac_port: null,
  ca_trusted: true,
  relay_base_url: "http://127.0.0.1:45981",
  env_export_opted_in: true,
  env_export_separable: false,
  domains: [],
};

function renderPanel(props: Partial<React.ComponentProps<typeof Diagnostics>> = {}) {
  render(
    <Diagnostics
      onBack={vi.fn()}
      version="1.4.2"
      account={account}
      oauth={null}
      proxy={proxy}
      providers={[]}
      tools={[]}
      {...props}
    />,
  );
}

function resolveProbes(snapshot: BackendDiagnostics | null = backend) {
  (fetchDiagnostics as Mock).mockResolvedValue(snapshot);
  (launchAtLoginStatus as Mock).mockResolvedValue({ enabled: true, pending_disable: false });
  (routedClientsStale as Mock).mockResolvedValue(false);
  (runningAgents as Mock).mockResolvedValue({
    scanned_names: ["claude", "codex", "opencode"],
    agents: [
      {
        name: "claude",
        pid: 12345,
        started_at_unix: Math.floor(Date.now() / 1000) - 3600,
        predates_routing: true,
      },
    ],
  });
}

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

describe("Diagnostics", () => {
  it("shows the report once the probes land", async () => {
    resolveProbes();
    renderPanel();
    const pre = screen.getByLabelText("Diagnostics report");
    // Before the reads land it must not claim zeroes it hasn't measured.
    expect(pre.textContent).toBe("Collecting…");
    await waitFor(() => expect(pre.textContent).toContain("Ubuntu 25.10"));
    expect(pre.textContent).toContain("claude          pid 12345, up 1h 0m");
  });

  it("copies exactly what is on screen", async () => {
    const writeText = vi.fn(() => Promise.resolve());
    Object.assign(navigator, { clipboard: { writeText } });
    resolveProbes();
    renderPanel();
    const pre = screen.getByLabelText("Diagnostics report");
    await waitFor(() => expect(pre.textContent).toContain("Ubuntu 25.10"));
    const shown = pre.textContent;
    fireEvent.click(screen.getByRole("button", { name: "Copy report" }));
    await waitFor(() => expect(writeText).toHaveBeenCalledWith(shown));
    // The button is the only confirmation this panel gives, and it has text to
    // swap - so the screen reader gets the same signal the icon does.
    await screen.findByRole("button", { name: "Copied" });
  });

  it("still reports when the backend snapshot fails", async () => {
    resolveProbes(null);
    (fetchDiagnostics as Mock).mockRejectedValue(new Error("no"));
    renderPanel();
    await waitFor(() =>
      expect(screen.getByLabelText("Diagnostics report").textContent).toContain("os              unknown"),
    );
    // What the popover itself knows is unaffected by a failed probe.
    expect(screen.getByLabelText("Diagnostics report").textContent).toContain("routing         on");
  });

  it("renders the rest of the report when the process scan never answers", async () => {
    // The one probe that walks the whole process table, hung. Everything else
    // has landed, and a support thread needs those findings more than it needs
    // to know which tools were running.
    resolveProbes();
    (runningAgents as Mock).mockReturnValue(new Promise(() => {}));
    vi.useFakeTimers();
    try {
      renderPanel();
      // Past the scan's ceiling: the three resolved probes settle on the
      // microtask queue this drains, then the timer fires.
      await act(async () => {
        await vi.advanceTimersByTimeAsync(3000);
      });
      const pre = screen.getByLabelText("Diagnostics report");
      expect(pre.textContent).toContain("Ubuntu 25.10");
      expect(pre.textContent).toContain("scan            unknown");
    } finally {
      vi.useRealTimers();
    }
  });

  it("goes back to where it was opened from", async () => {
    resolveProbes();
    const onBack = vi.fn();
    renderPanel({ onBack });
    fireEvent.click(screen.getByRole("button", { name: "Back" }));
    await waitFor(() => expect(onBack).toHaveBeenCalled());
  });
});
