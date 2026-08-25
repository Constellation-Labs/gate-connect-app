import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { Mock } from "vitest";
import { act, cleanup, render } from "@testing-library/react";
import type { Account, Org } from "./api";
import { useSettingsActions } from "./useSettingsActions";

vi.mock("@tauri-apps/plugin-process", () => ({ relaunch: vi.fn() }));
vi.mock("./api", () => ({
  clearAccount: vi.fn(),
  deviceName: vi.fn(),
  getAccount: vi.fn(),
  launchAtLoginStatus: vi.fn(),
  oauthBeginLogin: vi.fn(),
  oauthListOrgs: vi.fn(),
  oauthSignOut: vi.fn(),
  oauthStatus: vi.fn(),
  proxyDisable: vi.fn(),
  proxyStatus: vi.fn(),
  saveAccount: vi.fn(),
  setDeviceName: vi.fn(),
  setLaunchAtLogin: vi.fn(),
  setOrg: vi.fn(),
  switchGateway: vi.fn(),
}));
vi.mock("./analytics", () => ({ track: vi.fn(), trackError: vi.fn() }));

import { relaunch } from "@tauri-apps/plugin-process";
import {
  clearAccount,
  deviceName,
  getAccount,
  launchAtLoginStatus,
  oauthBeginLogin,
  oauthListOrgs,
  oauthSignOut,
  oauthStatus,
  proxyDisable,
  proxyStatus,
  saveAccount,
  setDeviceName,
  setLaunchAtLogin,
  setOrg,
  switchGateway,
} from "./api";

const ACCOUNT: Account = {
  gateway_base_url: "https://gw.example",
  has_api_key: true,
  auth_mode: "api_key",
  billing_mode: "byok",
  org_id: "org-1",
  org_name: "Constellation Labs",
};

const ORGS: Org[] = [
  { orgId: "org-1", name: "Constellation Labs", slug: "constellation", role: "admin" },
  { orgId: "org-2", name: "Side Project", slug: "side-project", role: "member" },
];

/** Drives the hook from a component, since it owns state. */
function harness(
  over: {
    account?: Account | null;
    launchAtLogin?: boolean;
    proxyRunning?: boolean;
  } = {},
) {
  const api: { current: ReturnType<typeof useSettingsActions> | null } = { current: null };
  const onLaunchAtLogin = vi.fn();
  const onAccount = vi.fn();
  const onDeviceName = vi.fn();
  const onSession = vi.fn();
  const onProxy = vi.fn();
  const onError = vi.fn();

  function Probe() {
    api.current = useSettingsActions({
      account: over.account === undefined ? ACCOUNT : over.account,
      proxyRunning: over.proxyRunning ?? false,
      launchAtLogin: over.launchAtLogin ?? false,
      onLaunchAtLogin,
      onAccount,
      onDeviceName,
      onSession,
      onProxy,
      onError,
    });
    return null;
  }
  render(<Probe />);
  return { api, onLaunchAtLogin, onAccount, onDeviceName, onSession, onProxy, onError };
}

beforeEach(() => {
  vi.clearAllMocks();
  (getAccount as Mock).mockResolvedValue(ACCOUNT);
  (saveAccount as Mock).mockResolvedValue(undefined);
  (setLaunchAtLogin as Mock).mockResolvedValue(undefined);
  (setOrg as Mock).mockResolvedValue(undefined);
  (oauthListOrgs as Mock).mockResolvedValue(ORGS);
  (oauthSignOut as Mock).mockResolvedValue(undefined);
  (oauthStatus as Mock).mockResolvedValue({ signed_in: false, email: null, expires_at_unix: 0 });
  (clearAccount as Mock).mockResolvedValue(undefined);
  (proxyDisable as Mock).mockResolvedValue({ running: false });
  (proxyStatus as Mock).mockResolvedValue({ running: true });
  (launchAtLoginStatus as Mock).mockResolvedValue({ enabled: true, pending_disable: false });
});
afterEach(cleanup);

describe("useSettingsActions: launch at login", () => {
  it("moves the switch before the backend answers, then reconciles", async () => {
    const { api, onLaunchAtLogin } = harness({ launchAtLogin: false });

    await act(async () => {
      await api.current!.toggleLaunchAtLogin();
    });

    expect(setLaunchAtLogin).toHaveBeenCalledWith(true);
    // Optimistic first, then the re-read.
    expect(onLaunchAtLogin.mock.calls.map(([s]) => s)).toEqual([
      { enabled: true, pendingDisable: false },
      { enabled: true, pendingDisable: false },
    ]);
  });

  it("reports a deferred opt-out from the re-read, not from the click", async () => {
    // Turning it off while routing is on leaves the OS login item registered as
    // a crash safety net. The backend decides that, so only the re-read knows.
    (launchAtLoginStatus as Mock).mockResolvedValue({ enabled: false, pending_disable: true });
    const { api, onLaunchAtLogin } = harness({ launchAtLogin: true });

    await act(async () => {
      await api.current!.toggleLaunchAtLogin();
    });

    expect(onLaunchAtLogin).toHaveBeenLastCalledWith({ enabled: false, pendingDisable: true });
  });

  it("puts the switch back when the write fails", async () => {
    (setLaunchAtLogin as Mock).mockRejectedValue(new Error("nope"));
    const { api, onLaunchAtLogin, onError } = harness({ launchAtLogin: false });

    await act(async () => {
      await api.current!.toggleLaunchAtLogin();
    });

    expect(onLaunchAtLogin).toHaveBeenLastCalledWith({ enabled: false, pendingDisable: false });
    expect(onError).toHaveBeenCalled();
    expect(launchAtLoginStatus).not.toHaveBeenCalled();
  });

  it("keeps the switch where the write left it when only the re-read fails", async () => {
    // The toggle succeeded, so a failed re-read must not revert the switch to
    // the opposite of the actual state.
    (launchAtLoginStatus as Mock).mockRejectedValue(new Error("nope"));
    const { api, onLaunchAtLogin, onError } = harness({ launchAtLogin: false });

    await act(async () => {
      await api.current!.toggleLaunchAtLogin();
    });

    expect(onLaunchAtLogin).toHaveBeenLastCalledWith({ enabled: true, pendingDisable: false });
    expect(onError).not.toHaveBeenCalled();
  });
});

describe("useSettingsActions: replacing the API key", () => {
  it("saves against the current gateway and re-reads the account", async () => {
    const { api, onAccount } = harness();

    act(() => api.current!.openReplaceKey());
    act(() => api.current!.setNewKey("  sk-gw-new  "));
    await act(async () => {
      await api.current!.replaceKey();
    });

    // Trimmed: a pasted key picks up whitespace, and the gateway rejects it.
    expect(saveAccount).toHaveBeenCalledWith("https://gw.example", "sk-gw-new");
    expect(onAccount).toHaveBeenCalledWith(ACCOUNT);
    expect(api.current!.prompt).toBeNull();
  });

  it("does nothing for an empty field", async () => {
    const { api } = harness();

    act(() => api.current!.openReplaceKey());
    act(() => api.current!.setNewKey("   "));
    await act(async () => {
      await api.current!.replaceKey();
    });

    expect(saveAccount).not.toHaveBeenCalled();
    expect(api.current!.prompt).toEqual({ kind: "replace-key" });
  });

  it("leaves the dialog open when the key is rejected", async () => {
    // So the user can correct it rather than reopening and retyping.
    (saveAccount as Mock).mockRejectedValue(new Error("401"));
    const { api, onError } = harness();

    act(() => api.current!.openReplaceKey());
    act(() => api.current!.setNewKey("sk-gw-bad"));
    await act(async () => {
      await api.current!.replaceKey();
    });

    expect(onError).toHaveBeenCalled();
    expect(api.current!.prompt).toEqual({ kind: "replace-key" });
    expect(api.current!.newKey).toBe("sk-gw-bad");
  });

  it("clears the draft key when the dialog is dismissed", async () => {
    // A credential should not survive a cancel and reappear on reopen.
    const { api } = harness();

    act(() => api.current!.openReplaceKey());
    act(() => api.current!.setNewKey("sk-gw-typed"));
    act(() => api.current!.dismissPrompt());

    expect(api.current!.newKey).toBe("");
    expect(api.current!.prompt).toBeNull();
  });
});

describe("useSettingsActions: switching organization", () => {
  it("opens the picker on the current org", async () => {
    const { api } = harness();

    await act(async () => {
      await api.current!.openSwitchOrg();
    });

    expect(api.current!.prompt).toEqual({
      kind: "switch-org",
      orgs: ORGS,
      selectedId: "org-1",
    });
  });

  it("does not open a picker over a single organization", async () => {
    // A question with one answer.
    (oauthListOrgs as Mock).mockResolvedValue([ORGS[0]]);
    const { api, onError } = harness();

    await act(async () => {
      await api.current!.openSwitchOrg();
    });

    expect(api.current!.prompt).toBeNull();
    expect(onError).not.toHaveBeenCalled();
  });

  it("sets the org, re-reads the account, and confirms", async () => {
    const { api, onAccount } = harness();

    await act(async () => {
      await api.current!.openSwitchOrg();
    });
    act(() => api.current!.selectOrg("org-2"));
    await act(async () => {
      await api.current!.confirmSwitchOrg();
    });

    expect(setOrg).toHaveBeenCalledWith("org-2", "Side Project");
    expect(onAccount).toHaveBeenCalledWith(ACCOUNT);
    expect(api.current!.prompt).toEqual({ kind: "org-switched", name: "Side Project" });
  });

  it("does not confirm a switch when the org list could not be read", async () => {
    (oauthListOrgs as Mock).mockRejectedValue(new Error("offline"));
    const { api, onError } = harness();

    await act(async () => {
      await api.current!.openSwitchOrg();
    });
    await act(async () => {
      await api.current!.confirmSwitchOrg();
    });

    expect(onError).toHaveBeenCalled();
    expect(setOrg).not.toHaveBeenCalled();
  });

  it("keeps the picker open when the switch fails", async () => {
    (setOrg as Mock).mockRejectedValue(new Error("nope"));
    const { api, onError } = harness();

    await act(async () => {
      await api.current!.openSwitchOrg();
    });
    await act(async () => {
      await api.current!.confirmSwitchOrg();
    });

    expect(onError).toHaveBeenCalled();
    expect(api.current!.prompt?.kind).toBe("switch-org");
  });
});

describe("useSettingsActions: disconnecting", () => {
  it("ends the session and keeps the account", async () => {
    // Scoped to the session: removing the account is what reset is for.
    const { api, onSession } = harness();

    act(() => api.current!.openDisconnect());
    expect(api.current!.prompt).toEqual({ kind: "disconnect" });

    await act(async () => {
      await api.current!.confirmDisconnect();
    });

    expect(oauthSignOut).toHaveBeenCalled();
    expect(clearAccount).not.toHaveBeenCalled();
    expect(onSession).toHaveBeenCalled();
    expect(api.current!.prompt).toBeNull();
  });

  it("keeps the dialog open when the sign-out fails", async () => {
    (oauthSignOut as Mock).mockRejectedValue(new Error("nope"));
    const { api, onError } = harness();

    act(() => api.current!.openDisconnect());
    await act(async () => {
      await api.current!.confirmDisconnect();
    });

    expect(onError).toHaveBeenCalled();
    expect(api.current!.prompt).toEqual({ kind: "disconnect" });
  });
});

describe("useSettingsActions: resetting", () => {
  it("refuses until the consequences are acknowledged", async () => {
    const { api } = harness();

    act(() => api.current!.openReset());
    await act(async () => {
      await api.current!.confirmReset();
    });

    expect(clearAccount).not.toHaveBeenCalled();

    act(() => api.current!.acknowledgeReset(true));
    await act(async () => {
      await api.current!.confirmReset();
    });

    expect(clearAccount).toHaveBeenCalled();
  });

  it("stops routing before removing the account", async () => {
    // The order is the point: clearing the account while the engine is up leaves
    // system HTTPS pointed at a port that is about to die.
    const { api } = harness({ proxyRunning: true });

    act(() => api.current!.openReset());
    act(() => api.current!.acknowledgeReset(true));
    await act(async () => {
      await api.current!.confirmReset();
    });

    expect((proxyDisable as Mock).mock.invocationCallOrder[0]).toBeLessThan(
      (clearAccount as Mock).mock.invocationCallOrder[0],
    );
  });

  it("aborts rather than stranding traffic on a dead proxy", async () => {
    (proxyDisable as Mock).mockRejectedValue(new Error("disable failed"));
    const { api, onError, onProxy } = harness({ proxyRunning: true });

    act(() => api.current!.openReset());
    act(() => api.current!.acknowledgeReset(true));
    await act(async () => {
      await api.current!.confirmReset();
    });

    expect(clearAccount).not.toHaveBeenCalled();
    expect(onError).toHaveBeenCalled();
    // Re-read, so the window shows what the engine actually did.
    expect(proxyStatus).toHaveBeenCalled();
    expect(onProxy).toHaveBeenLastCalledWith({ running: true });
  });

  it("does not touch the engine when it is already off", async () => {
    const { api } = harness({ proxyRunning: false });

    act(() => api.current!.openReset());
    act(() => api.current!.acknowledgeReset(true));
    await act(async () => {
      await api.current!.confirmReset();
    });

    expect(proxyDisable).not.toHaveBeenCalled();
    expect(clearAccount).toHaveBeenCalled();
  });

  it("clears the session so the window falls back to sign-in on its own", async () => {
    // The setup stage is derived from what is on disk, so this is the whole
    // handoff - there is no separate "go to first run" step to disagree with it.
    const { api, onSession } = harness();

    act(() => api.current!.openReset());
    act(() => api.current!.acknowledgeReset(true));
    await act(async () => {
      await api.current!.confirmReset();
    });

    expect(onSession).toHaveBeenCalledWith({ account: null, oauth: null });
  });

  it("leaves the user signed in when the wipe fails", async () => {
    // clear_account disconnects managed tools before wiping, so a failure means
    // we are still signed in; dropping to sign-in would show a half-reset app.
    (clearAccount as Mock).mockRejectedValue(new Error("nope"));
    const { api, onError, onSession } = harness();

    act(() => api.current!.openReset());
    act(() => api.current!.acknowledgeReset(true));
    await act(async () => {
      await api.current!.confirmReset();
    });

    expect(onError).toHaveBeenCalled();
    expect(onSession).not.toHaveBeenCalled();
    expect(api.current!.prompt?.kind).toBe("reset");
  });
});

describe("useSettingsActions: copying", () => {
  it("flags a successful copy so the row can confirm it", async () => {
    const writeText = vi.fn().mockResolvedValue(undefined);
    vi.stubGlobal("navigator", { clipboard: { writeText } });
    const { api } = harness();

    await act(async () => {
      await api.current!.copyText("gc_a1b2c3d4");
    });

    expect(writeText).toHaveBeenCalledWith("gc_a1b2c3d4");
    expect(api.current!.copied).toBe(true);
    vi.unstubAllGlobals();
  });

  it("surfaces a denied clipboard rather than silently doing nothing", async () => {
    const writeText = vi.fn().mockRejectedValue(new Error("denied"));
    vi.stubGlobal("navigator", { clipboard: { writeText } });
    const { api, onError } = harness();

    await act(async () => {
      await api.current!.copyText("gc_a1b2c3d4");
    });

    expect(onError).toHaveBeenCalled();
    expect(api.current!.copied).toBe(false);
    vi.unstubAllGlobals();
  });
});

describe("useSettingsActions: renaming the device", () => {
  it("opens prefilled with the current name", async () => {
    // The field is an edit, not a blank form.
    const { api } = harness();

    act(() => api.current!.openRenameDevice("e2e-macbook"));

    expect(api.current!.prompt).toEqual({
      kind: "rename-device",
      currentName: "e2e-macbook",
    });
    expect(api.current!.newDeviceName).toBe("e2e-macbook");
  });

  it("saves the name and reports back what the backend resolved", async () => {
    // Re-read rather than echoed: the backend decides what a name means, and a
    // cleared one goes back to following the hostname.
    (deviceName as Mock).mockResolvedValue("Studio Mac");
    const { api, onDeviceName } = harness();

    act(() => api.current!.openRenameDevice("e2e-macbook"));
    act(() => api.current!.setNewDeviceName("  Studio Mac  "));
    await act(async () => {
      await api.current!.renameDevice();
    });

    expect(setDeviceName).toHaveBeenCalledWith("Studio Mac");
    expect(onDeviceName).toHaveBeenCalledWith("Studio Mac");
    expect(api.current!.prompt).toBeNull();
  });

  it("will not save a blank name", async () => {
    // Clearing the override is a backend behaviour, not something the dialog can
    // reach: its primary is refused, so an empty field cannot be submitted here.
    const { api } = harness();

    act(() => api.current!.openRenameDevice("e2e-macbook"));
    act(() => api.current!.setNewDeviceName("   "));
    await act(async () => {
      await api.current!.renameDevice();
    });

    expect(setDeviceName).not.toHaveBeenCalled();
  });

  it("keeps the dialog open when the rename fails", async () => {
    (setDeviceName as Mock).mockRejectedValue(new Error("read-only volume"));
    const { api, onError } = harness();

    act(() => api.current!.openRenameDevice("e2e-macbook"));
    act(() => api.current!.setNewDeviceName("Studio Mac"));
    await act(async () => {
      await api.current!.renameDevice();
    });

    expect(onError).toHaveBeenCalled();
    expect(api.current!.prompt?.kind).toBe("rename-device");
    expect(api.current!.newDeviceName).toBe("Studio Mac");
  });
});

describe("useSettingsActions: changing the gateway", () => {
  it("opens on the account's current server", async () => {
    const { api } = harness();

    act(() => api.current!.openSwitchGateway());

    expect(api.current!.prompt).toEqual({
      kind: "switch-gateway",
      selectedUrl: "https://gw.example",
    });
  });

  it("refuses a switch to the server it is already on", async () => {
    // The dialog also disables its primary, but the guard is here so a stale
    // click cannot forget a key and relaunch for no reason.
    const { api } = harness();

    act(() => api.current!.openSwitchGateway());
    await act(async () => {
      await api.current!.confirmSwitchGateway();
    });

    expect(switchGateway).not.toHaveBeenCalled();
    expect(relaunch).not.toHaveBeenCalled();
  });

  it("switches, then relaunches into a clean session", async () => {
    const { api } = harness();

    act(() => api.current!.openSwitchGateway());
    act(() => api.current!.selectGateway("https://gw-staging.example"));
    await act(async () => {
      await api.current!.confirmSwitchGateway();
    });

    expect(switchGateway).toHaveBeenCalledWith("https://gw-staging.example");
    // The engine pins the gateway URL when it starts, so the app restarts rather
    // than reconciling an account, a session and a routing table live.
    expect(relaunch).toHaveBeenCalled();
  });

  it("keeps the dialog open when the switch fails", async () => {
    (switchGateway as Mock).mockRejectedValue(new Error("no such environment"));
    const { api, onError } = harness();

    act(() => api.current!.openSwitchGateway());
    act(() => api.current!.selectGateway("https://gw-staging.example"));
    await act(async () => {
      await api.current!.confirmSwitchGateway();
    });

    expect(onError).toHaveBeenCalled();
    expect(relaunch).not.toHaveBeenCalled();
    expect(api.current!.prompt?.kind).toBe("switch-gateway");
  });
});

describe("useSettingsActions: the OAuth upgrade", () => {
  it("signs in against the account that already exists", async () => {
    // Deliberately no `saveAccount`: that is `useSetup.signIn`'s first step, and
    // here it would repoint a staging install at the default gateway and drop the
    // key the user still has.
    (getAccount as Mock).mockResolvedValue({ ...ACCOUNT, auth_mode: "oauth" });
    (oauthStatus as Mock).mockResolvedValue({
      signed_in: true,
      email: "jdoe@acme.com",
      expires_at_unix: 1,
    });
    const { api, onSession } = harness();

    await act(async () => {
      await api.current!.upgradeToOAuth();
    });

    expect(oauthBeginLogin).toHaveBeenCalled();
    expect(saveAccount).not.toHaveBeenCalled();
    expect(onSession).toHaveBeenCalledWith({
      account: expect.objectContaining({ auth_mode: "oauth" }),
      oauth: expect.objectContaining({ signed_in: true }),
    });
  });

  it("lets a failed sign-in reach the offer", async () => {
    // The offer shows the error itself, so this one rethrows rather than routing
    // it to the shell's banner behind the dialog.
    (oauthBeginLogin as Mock).mockRejectedValue(new Error("browser closed"));
    const { api } = harness();

    await expect(
      act(async () => {
        await api.current!.upgradeToOAuth();
      }),
    ).rejects.toThrow("browser closed");
  });
});

describe("useSettingsActions: no account", () => {
  it("cannot replace a key with no gateway to save it against", async () => {
    const { api } = harness({ account: null });

    act(() => api.current!.openReplaceKey());
    act(() => api.current!.setNewKey("sk-gw-new"));
    await act(async () => {
      await api.current!.replaceKey();
    });

    expect(saveAccount).not.toHaveBeenCalled();
  });
});
