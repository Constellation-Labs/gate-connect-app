import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, render, screen, fireEvent } from "@testing-library/react";
import type { Mock } from "vitest";
import type { Platform } from "../lib/platform";
import type { ProxyState } from "../lib/api";
import type { ClassifiedError } from "../lib/errors";
import { ProxyScreen } from "./ProxyScreen";

// The CA-trust panel swaps the trust-store name by platform; drive it by
// mocking usePlatform rather than the async Tauri lookup.
vi.mock("../lib/platform", () => ({ usePlatform: vi.fn() }));
import { usePlatform } from "../lib/platform";

// The screen reads launch-at-login state for its restart tip; stub the
// Tauri command so jsdom never sees an invoke. A pending promise keeps the
// tip hidden and avoids post-test state updates (act warnings).
vi.mock("../lib/api", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../lib/api")>();
  return {
    ...actual,
    launchAtLoginStatus: vi.fn(() => new Promise(() => {})),
  };
});

// running + !ca_trusted is the only state that renders the trust panel.
const proxy: ProxyState = {
  running: true,
  port: 8080,
  pac_port: null,
  ca_trusted: false,
  domains: [],
};

function renderOn(platform: Platform, relaunchHint = false, props: Partial<React.ComponentProps<typeof ProxyScreen>> = {}) {
  (usePlatform as Mock).mockReturnValue(platform);
  render(
    <ProxyScreen
      proxy={proxy}
      providers={[]}
      busy={false}
      error={null}
      onBack={vi.fn()}
      onToggleProxy={vi.fn()}
      onSetProvider={vi.fn()}
      onTrustCa={vi.fn()}
      restartHint={false}
      onDismissRestartHint={vi.fn()}
      relaunchHint={relaunchHint}
      onDismissRelaunchHint={vi.fn()}
      codexDrifted={false}
      {...props}
    />,
  );
}

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

describe("ProxyScreen CA-trust panel", () => {
  it("names the keychain on macOS", () => {
    renderOn("macos");
    expect(screen.getByText(/certificate your keychain trusts/)).toBeTruthy();
    expect(screen.queryByText(/certificate store/)).toBeNull();
  });

  it("names the certificate store on Windows", () => {
    renderOn("windows");
    expect(screen.getByText(/certificate your certificate store trusts/)).toBeTruthy();
    expect(screen.queryByText(/keychain/)).toBeNull();
  });

  it("explains the key stays local and how to undo", () => {
    renderOn("macos");
    expect(screen.getByText(/created on this machine/)).toBeTruthy();
    expect(screen.getByText(/remove it anytime in Settings/i)).toBeTruthy();
  });
});

describe("ProxyScreen Linux relaunch hint", () => {
  it("tells Linux users to reopen already-running agents when flashed", () => {
    renderOn("linux", true);
    expect(screen.getByText(/Agents already running/)).toBeTruthy();
  });

  it("is hidden on Linux until routing is turned on", () => {
    renderOn("linux", false);
    expect(screen.queryByText(/Agents already running/)).toBeNull();
  });

  it("is not shown on macOS even when flashed", () => {
    renderOn("macos", true);
    expect(screen.queryByText(/Agents already running/)).toBeNull();
  });
});

describe("ProxyScreen Codex adoption notice", () => {
  it("warns that enabling replaces an out-of-app Codex setup when drifted", () => {
    renderOn("macos", false, { codexDrifted: true });
    expect(screen.getByText(/Codex has a Gate setup written outside this app/)).toBeTruthy();
  });

  it("is hidden when Codex is not drifted", () => {
    renderOn("macos");
    expect(screen.queryByText(/Codex has a Gate setup/)).toBeNull();
  });
});

describe("ProxyScreen interactions", () => {
  it("calls onToggleProxy when the main toggle is clicked", () => {
    const onToggleProxy = vi.fn();
    renderOn("macos", false, { onToggleProxy });
    fireEvent.click(screen.getByRole("switch"));
    expect(onToggleProxy).toHaveBeenCalledTimes(1);
  });

  it("names the master toggle for assistive tech", () => {
    renderOn("macos");
    expect(screen.getByLabelText("Route through Gate")).toBeTruthy();
  });

  it("calls onTrustCa when 'Trust certificate' is clicked", () => {
    const onTrustCa = vi.fn();
    renderOn("macos", false, { onTrustCa });
    fireEvent.click(screen.getByText("Trust certificate"));
    expect(onTrustCa).toHaveBeenCalledTimes(1);
  });

  it("calls onBack when the back button is clicked", () => {
    const onBack = vi.fn();
    renderOn("macos", false, { onBack });
    fireEvent.click(screen.getByLabelText("Back"));
    expect(onBack).toHaveBeenCalledTimes(1);
  });

  it("displays a classified error with its title and hint", () => {
    const error: ClassifiedError = {
      title: "Couldn't toggle routing",
      hint: "Try again.",
      raw: "backend said no",
    };
    renderOn("macos", false, { error });
    expect(screen.getByText("Couldn't toggle routing")).toBeTruthy();
    expect(screen.getByText("Try again.")).toBeTruthy();
  });

  it("disables controls when busy", () => {
    renderOn("macos", false, { busy: true });
    const toggle = screen.getByRole("switch");
    const trustButton = screen.getByText("Trust certificate").closest("button");
    const backButton = screen.getByLabelText("Back");

    expect(toggle).toHaveProperty("disabled", true);
    expect(trustButton).toHaveProperty("disabled", true);
    expect(backButton).toHaveProperty("disabled", false); // Back button stays active
  });
});
