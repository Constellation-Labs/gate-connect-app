import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { Mock } from "vitest";
import { act, cleanup, render } from "@testing-library/react";
import type { Account, Org } from "./api";
import { useSettingsActions } from "./useSettingsActions";

vi.mock("./api", () => ({
  getAccount: vi.fn(),
  launchAtLoginStatus: vi.fn(),
  oauthListOrgs: vi.fn(),
  saveAccount: vi.fn(),
  setLaunchAtLogin: vi.fn(),
  setOrg: vi.fn(),
}));
vi.mock("./analytics", () => ({ track: vi.fn(), trackError: vi.fn() }));

import {
  getAccount,
  launchAtLoginStatus,
  oauthListOrgs,
  saveAccount,
  setLaunchAtLogin,
  setOrg,
} from "./api";

const ACCOUNT: Account = {
  gateway_base_url: "https://gw.example",
  has_api_key: true,
  auth_mode: "api_key",
  org_id: "org-1",
  org_name: "Constellation Labs",
};

const ORGS: Org[] = [
  { orgId: "org-1", name: "Constellation Labs", slug: "constellation", role: "admin" },
  { orgId: "org-2", name: "Side Project", slug: "side-project", role: "member" },
];

/** Drives the hook from a component, since it owns state. */
function harness(over: { account?: Account | null; launchAtLogin?: boolean } = {}) {
  const api: { current: ReturnType<typeof useSettingsActions> | null } = { current: null };
  const onLaunchAtLogin = vi.fn();
  const onAccount = vi.fn();
  const onError = vi.fn();

  function Probe() {
    api.current = useSettingsActions({
      account: over.account === undefined ? ACCOUNT : over.account,
      launchAtLogin: over.launchAtLogin ?? false,
      onLaunchAtLogin,
      onAccount,
      onError,
    });
    return null;
  }
  render(<Probe />);
  return { api, onLaunchAtLogin, onAccount, onError };
}

beforeEach(() => {
  vi.clearAllMocks();
  (getAccount as Mock).mockResolvedValue(ACCOUNT);
  (saveAccount as Mock).mockResolvedValue(undefined);
  (setLaunchAtLogin as Mock).mockResolvedValue(undefined);
  (setOrg as Mock).mockResolvedValue(undefined);
  (oauthListOrgs as Mock).mockResolvedValue(ORGS);
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
