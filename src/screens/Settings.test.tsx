import { afterEach, describe, expect, it, vi } from "vitest";
import { act, cleanup, render, screen, fireEvent, waitFor } from "@testing-library/react";
import type { Mock } from "vitest";
import type { Platform } from "../lib/platform";
import type { Account, LaunchAtLoginStatus } from "../lib/api";
import { Settings } from "./Settings";

// The certificate section swaps the trust-store name by platform; drive it by
// mocking usePlatform rather than the async Tauri lookup.
vi.mock("../lib/platform", () => ({ usePlatform: vi.fn() }));
vi.mock("../lib/api", () => ({
  launchAtLoginStatus: vi.fn(),
  setLaunchAtLogin: vi.fn(),
  getAccountKeyPrefix: vi.fn().mockResolvedValue(null),
  backfillAccountKeyPrefix: vi.fn(),
}));
vi.mock("../lib/analytics", () => ({ track: vi.fn(), trackError: vi.fn() }));
import { usePlatform } from "../lib/platform";
import { launchAtLoginStatus, setLaunchAtLogin } from "../lib/api";

const account: Account = {
  gateway_base_url: "https://gate.example.com",
  has_api_key: true,
  auth_mode: "api_key",
  org_id: null,
  org_name: null,
};

// Typed helper so the mock can't silently drift from the real
// LaunchAtLoginStatus shape again (the component reads status.enabled /
// status.pending_disable, and an untyped factory would let a stale boolean
// mock run every test with `undefined` state).
function lalStatus(enabled: boolean, pending_disable = false): LaunchAtLoginStatus {
  return { enabled, pending_disable };
}

async function renderOn(platform: Platform, props: Partial<React.ComponentProps<typeof Settings>> = {}) {
  (usePlatform as Mock).mockReturnValue(platform);
  render(
    <Settings
      account={account}
      oauth={null}
      onBack={vi.fn()}
      onReplaceKey={vi.fn()}
      onUpgradeToOAuth={vi.fn()}
      onForget={vi.fn()}
      onSignOut={vi.fn()}
      onSwitchOrg={vi.fn()}
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
    (launchAtLoginStatus as Mock).mockResolvedValue(lalStatus(false));
    await renderOn("macos", { caTrusted: true });
    expect(screen.getByText(/still trusted in your keychain/i)).toBeTruthy();
    expect(screen.getByRole("button", { name: "Remove" })).toBeTruthy();
  });

  it("names the certificate store on Windows", async () => {
    (launchAtLoginStatus as Mock).mockResolvedValue(lalStatus(false));
    await renderOn("windows", { caTrusted: true });
    expect(screen.getByText(/still trusted in your certificate store/i)).toBeTruthy();
  });

  it("stays available while routing is on, because Home promises it does", async () => {
    // Home tells the user "You can remove it anytime in Settings under
    // Certificate" in the untrusted-CA state, i.e. with routing on. Hiding the
    // section behind !routingOn broke that promise the moment they acted on
    // it, for a root CA on their machine.
    (launchAtLoginStatus as Mock).mockResolvedValue(lalStatus(false));
    await renderOn("macos", { caTrusted: true, routingOn: true });
    expect(screen.getByText(/still trusted in your keychain/i)).toBeTruthy();
    expect(screen.getByRole("button", { name: "Remove" })).toBeTruthy();
  });

  it("warns what removal costs only while routing is on", async () => {
    (launchAtLoginStatus as Mock).mockResolvedValue(lalStatus(false));
    await renderOn("macos", { caTrusted: true, routingOn: true });
    expect(screen.getByText(/stop routing/i)).toBeTruthy();
    cleanup();

    (launchAtLoginStatus as Mock).mockResolvedValue(lalStatus(false));
    await renderOn("macos", { caTrusted: true });
    expect(screen.queryByText(/stop routing/i)).toBeNull();
  });

  it("is hidden when the CA is not trusted", async () => {
    (launchAtLoginStatus as Mock).mockResolvedValue(lalStatus(false));
    await renderOn("macos");
    expect(screen.queryByText(/still trusted/i)).toBeNull();
  });

  it("confirms before removing the certificate, and only then removes it", async () => {
    (launchAtLoginStatus as Mock).mockResolvedValue(lalStatus(false));
    const onUntrustCa = vi.fn();
    await renderOn("macos", { caTrusted: true, onUntrustCa });
    fireEvent.click(screen.getByRole("button", { name: "Remove" }));
    // It deletes a private key by its own copy, so the first click only arms.
    expect(onUntrustCa).not.toHaveBeenCalled();
    expect(screen.getByText(/deletes it and its private key/)).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Remove certificate" }));
    expect(onUntrustCa).toHaveBeenCalledTimes(1);
  });
});

describe("Settings launch at login", () => {
  const lalSwitch = () => screen.getByRole("switch");
  const pendingNote = () => screen.queryByText(/login items as a safety net/i);

  it("reflects the loaded status on the toggle and hides the note when nothing is pending", async () => {
    (launchAtLoginStatus as Mock).mockResolvedValue(lalStatus(true));
    await renderOn("macos");
    expect(lalSwitch().getAttribute("aria-checked")).toBe("true");
    expect(pendingNote()).toBeNull();
  });

  it("shows the safety-net note while a deferred opt-out is pending", async () => {
    (launchAtLoginStatus as Mock).mockResolvedValue(lalStatus(false, true));
    await renderOn("macos");
    // Pending reports the user's choice (off), plus the note explaining why
    // the OS login-items list still shows the app.
    expect(lalSwitch().getAttribute("aria-checked")).toBe("false");
    expect(pendingNote()).toBeTruthy();
  });

  it("adopts the backend's defer decision from the post-toggle re-read", async () => {
    // Routing on: toggling off is deferred, so the re-read reports
    // off-but-pending and the note appears.
    (launchAtLoginStatus as Mock)
      .mockResolvedValueOnce(lalStatus(true))
      .mockResolvedValueOnce(lalStatus(false, true));
    (setLaunchAtLogin as Mock).mockResolvedValue(undefined);
    await renderOn("macos");

    fireEvent.click(lalSwitch());
    await waitFor(() => expect(pendingNote()).toBeTruthy());
    expect(setLaunchAtLogin).toHaveBeenCalledWith(false);
    expect(lalSwitch().getAttribute("aria-checked")).toBe("false");
  });

  it("keeps a successful toggle applied when the post-toggle re-read fails", async () => {
    (launchAtLoginStatus as Mock)
      .mockResolvedValueOnce(lalStatus(false))
      .mockRejectedValueOnce(new Error("status read failed"));
    (setLaunchAtLogin as Mock).mockResolvedValue(undefined);
    await renderOn("macos");

    fireEvent.click(lalSwitch());
    await waitFor(() => expect(setLaunchAtLogin).toHaveBeenCalledWith(true));
    await act(async () => {});
    // The re-read is best-effort; its failure must not revert the switch.
    expect(lalSwitch().getAttribute("aria-checked")).toBe("true");
  });

  it("reverts the toggle when the backend call itself fails", async () => {
    (launchAtLoginStatus as Mock).mockResolvedValue(lalStatus(false));
    (setLaunchAtLogin as Mock).mockRejectedValue(new Error("no login items API"));
    await renderOn("macos");

    fireEvent.click(lalSwitch());
    await waitFor(() => expect(setLaunchAtLogin).toHaveBeenCalledWith(true));
    await act(async () => {});
    expect(lalSwitch().getAttribute("aria-checked")).toBe("false");
    expect(screen.getByText(/no login items API/i)).toBeTruthy();
  });
});
