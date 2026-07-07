import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, render, screen, fireEvent } from "@testing-library/react";
import type { Mock } from "vitest";
import type { Platform } from "../lib/platform";
import type { ProxyState } from "../lib/api";
import { ProxyScreen } from "./ProxyScreen";

// The CA-trust notice swaps the trust-store name by platform; drive it by
// mocking usePlatform rather than the async Tauri lookup.
vi.mock("../lib/platform", () => ({ usePlatform: vi.fn() }));
import { usePlatform } from "../lib/platform";

// running + !ca_trusted is the only state that renders the trust notice.
const proxy: ProxyState = {
  running: true,
  port: 8080,
  ca_trusted: false,
  domains: [],
  gateway_requests: 0,
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
      relaunchHint={relaunchHint}
      codexDrifted={false}
      {...props}
    />,
  );
}

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

describe("ProxyScreen CA-trust notice", () => {
  it("names the keychain on macOS", () => {
    renderOn("macos");
    expect(screen.getByText(/isn’t trusted in your keychain yet/)).toBeTruthy();
    expect(screen.queryByText(/certificate store/)).toBeNull();
  });

  it("names the certificate store on Windows", () => {
    renderOn("windows");
    expect(screen.getByText(/isn’t trusted in your certificate store yet/)).toBeTruthy();
    expect(screen.queryByText(/keychain/)).toBeNull();
  });
});

describe("ProxyScreen Linux relaunch hint", () => {
  it("tells Linux users to relaunch already-open apps when flashed", () => {
    renderOn("linux", true);
    expect(screen.getByText(/Apps already running/)).toBeTruthy();
  });

  it("is hidden on Linux until routing is turned on", () => {
    renderOn("linux", false);
    expect(screen.queryByText(/Apps already running/)).toBeNull();
  });

  it("is not shown on macOS even when flashed", () => {
    renderOn("macos", true);
    expect(screen.queryByText(/Apps already running/)).toBeNull();
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

  it("calls onTrustCa when the 'Trust' button is clicked", () => {
    const onTrustCa = vi.fn();
    renderOn("macos", false, { onTrustCa });
    fireEvent.click(screen.getByText("Trust"));
    expect(onTrustCa).toHaveBeenCalledTimes(1);
  });

  it("calls onBack when the back button is clicked", () => {
    const onBack = vi.fn();
    renderOn("macos", false, { onBack });
    fireEvent.click(screen.getByLabelText("Back"));
    expect(onBack).toHaveBeenCalledTimes(1);
  });

  it("displays an error message when an error is passed", () => {
    const error = "Something went wrong";
    renderOn("macos", false, { error });
    expect(screen.getByText(error)).toBeTruthy();
  });

  it("disables controls when busy", () => {
    renderOn("macos", false, { busy: true });
    const toggle = screen.getByRole("switch");
    const trustButton = screen.getByText("Trust").closest("button");
    const backButton = screen.getByLabelText("Back");

    expect(toggle).toHaveProperty("disabled", true);
    expect(trustButton).toHaveProperty("disabled", true);
    expect(backButton).toHaveProperty("disabled", false); // Back button stays active
  });
});
