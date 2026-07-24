import { afterEach, describe, expect, it, vi } from "vitest";
import { act, cleanup, render, screen, fireEvent } from "@testing-library/react";
import type { Mock } from "vitest";
import type { Platform } from "../lib/platform";
import type { Account } from "../lib/api";
import { Settings } from "./Settings";

// The certificate section swaps the trust-store name by platform; drive it by
// mocking usePlatform rather than the async Tauri lookup.
vi.mock("../lib/platform", () => ({ usePlatform: vi.fn() }));
vi.mock("../lib/api", () => ({
  launchAtLoginStatus: vi.fn().mockResolvedValue(false),
  setLaunchAtLogin: vi.fn(),
  getAccountKeyPrefix: vi.fn().mockResolvedValue(null),
  backfillAccountKeyPrefix: vi.fn(),
}));
vi.mock("../lib/analytics", () => ({ track: vi.fn(), trackError: vi.fn() }));
import { usePlatform } from "../lib/platform";

const account: Account = {
  gateway_base_url: "https://gate.example.com",
  has_api_key: true,
};

async function renderOn(platform: Platform, props: Partial<React.ComponentProps<typeof Settings>> = {}) {
  (usePlatform as Mock).mockReturnValue(platform);
  render(
    <Settings
      account={account}
      onBack={vi.fn()}
      onReplaceKey={vi.fn()}
      onDisconnect={vi.fn()}
      onSwitchGateway={vi.fn()}
      onReplayTour={vi.fn()}
      routingOn={false}
      caTrusted={false}
      proxyBusy={false}
      onUntrustCa={vi.fn()}
      {...props}
    />,
  );
  // Flush the launch-at-login/key-prefix mount effects so their state
  // updates land inside act.
  await act(async () => {});
}

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

describe("Settings certificate section", () => {
  it("offers Remove when routing is off but the CA is still trusted", async () => {
    await renderOn("macos", { caTrusted: true });
    expect(screen.getByText(/still trusted in your keychain/i)).toBeTruthy();
    expect(screen.getByRole("button", { name: "Remove" })).toBeTruthy();
  });

  it("names the certificate store on Windows", async () => {
    await renderOn("windows", { caTrusted: true });
    expect(screen.getByText(/still trusted in your certificate store/i)).toBeTruthy();
  });

  it("is hidden while routing is running", async () => {
    await renderOn("macos", { caTrusted: true, routingOn: true });
    expect(screen.queryByText(/still trusted/i)).toBeNull();
  });

  it("is hidden when the CA is not trusted", async () => {
    await renderOn("macos");
    expect(screen.queryByText(/still trusted/i)).toBeNull();
  });

  it("calls onUntrustCa when Remove is clicked", async () => {
    const onUntrustCa = vi.fn();
    await renderOn("macos", { caTrusted: true, onUntrustCa });
    fireEvent.click(screen.getByRole("button", { name: "Remove" }));
    expect(onUntrustCa).toHaveBeenCalledTimes(1);
  });
});
