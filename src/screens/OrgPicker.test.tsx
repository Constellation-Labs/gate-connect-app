import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, render, screen, fireEvent, waitFor } from "@testing-library/react";
import type { Mock } from "vitest";
import type { Org } from "../lib/api";
import { GATE_DASHBOARD_URL } from "../lib/config";

// The picker fetches the org list on mount and persists a choice; mock the two
// commands (and keep everything else real) so a test can hold the fetch
// pending, shape the list, or fail either call on cue.
vi.mock("../lib/api", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../lib/api")>();
  return {
    ...actual,
    oauthListOrgs: vi.fn(async (): Promise<Org[]> => []),
    setOrg: vi.fn(async () => {}),
  };
});
vi.mock("../lib/analytics", () => ({ track: vi.fn(), trackError: vi.fn() }));
vi.mock("../lib/openExternal", () => ({ openExternal: vi.fn(async () => {}) }));

import { oauthListOrgs, setOrg } from "../lib/api";
import { openExternal } from "../lib/openExternal";
import { OrgPicker } from "./OrgPicker";

function makeOrg(overrides: Partial<Org> = {}): Org {
  return {
    orgId: "org-1",
    name: "Constellation Labs",
    slug: "constellation-labs",
    role: "admin",
    ...overrides,
  };
}

const TWO_ORGS: Org[] = [
  makeOrg(),
  makeOrg({ orgId: "org-2", name: "Side Project", slug: "side-project", role: "member" }),
];

function renderPicker(props: Partial<React.ComponentProps<typeof OrgPicker>> = {}) {
  const onDone = vi.fn();
  const onReauth = vi.fn();
  render(<OrgPicker onDone={onDone} onReauth={onReauth} {...props} />);
  return { onDone, onReauth };
}

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
  // `clearAllMocks` clears calls but keeps implementations; restore the happy
  // path so one test's rejection doesn't fake a failure in the next.
  (oauthListOrgs as Mock).mockImplementation(async () => []);
  (setOrg as Mock).mockImplementation(async () => {});
});

describe("OrgPicker loading and list", () => {
  it("shows the loading state while the org list is in flight", () => {
    (oauthListOrgs as Mock).mockImplementation(() => new Promise(() => {}));
    renderPicker();
    expect(screen.getByRole("status").textContent).toContain("Loading organizations");
    expect(screen.getByRole("heading", { level: 1, name: "Choose an organization" })).toBeTruthy();
  });

  it("renders each org with its identity line", async () => {
    (oauthListOrgs as Mock).mockImplementation(async () => TWO_ORGS);
    renderPicker();
    await screen.findByText("Constellation Labs");
    expect(screen.getByText("Side Project")).toBeTruthy();
    // slug · role in mono is the identity line.
    expect(screen.getByText("constellation-labs · admin")).toBeTruthy();
    expect(screen.getByText("side-project · member")).toBeTruthy();
  });

  it("persists the picked org and advances", async () => {
    (oauthListOrgs as Mock).mockImplementation(async () => TWO_ORGS);
    const { onDone } = renderPicker();
    fireEvent.click(await screen.findByText("Side Project"));
    await waitFor(() => expect(onDone).toHaveBeenCalledTimes(1));
    // The UUID and the display name, not the slug.
    expect(setOrg).toHaveBeenCalledWith("org-2", "Side Project");
  });

  it("disables every row while a choice is in flight", async () => {
    (oauthListOrgs as Mock).mockImplementation(async () => TWO_ORGS);
    (setOrg as Mock).mockImplementation(() => new Promise(() => {}));
    renderPicker();
    const row = (await screen.findByText("Side Project")).closest("button")!;
    fireEvent.click(row);
    await waitFor(() => expect(row.disabled).toBe(true));
    expect((screen.getByText("Constellation Labs").closest("button") as HTMLButtonElement).disabled).toBe(true);
  });
});

describe("OrgPicker single-org auto-select", () => {
  it("selects the lone org without rendering a picker", async () => {
    (oauthListOrgs as Mock).mockImplementation(async () => [makeOrg()]);
    const { onDone } = renderPicker();
    await waitFor(() => expect(onDone).toHaveBeenCalledTimes(1));
    expect(setOrg).toHaveBeenCalledWith("org-1", "Constellation Labs");
    // The one-tap-shorter promise: the list never flashed.
    expect(screen.queryByText("Constellation Labs")).toBeNull();
  });

  it("surfaces the failure when auto-selecting the lone org fails", async () => {
    (oauthListOrgs as Mock).mockImplementation(async () => [makeOrg()]);
    (setOrg as Mock).mockImplementation(async () => {
      throw "gateway said no";
    });
    const { onDone, onReauth } = renderPicker();
    await screen.findByRole("alert");
    expect(onDone).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole("button", { name: "Sign in again" }));
    expect(onReauth).toHaveBeenCalledTimes(1);
  });
});

describe("OrgPicker error and empty states", () => {
  it("offers a re-sign-in when the list can't be fetched", async () => {
    (oauthListOrgs as Mock).mockImplementation(async () => {
      throw "401 unauthorized";
    });
    const { onReauth } = renderPicker();
    const alert = await screen.findByRole("alert");
    expect(alert.textContent).toContain("Gateway rejected");
    fireEvent.click(screen.getByRole("button", { name: "Sign in again" }));
    expect(onReauth).toHaveBeenCalledTimes(1);
  });

  it("gives a solo developer both ways forward when there is no org", async () => {
    (oauthListOrgs as Mock).mockImplementation(async () => []);
    const onUseApiKey = vi.fn();
    renderPicker({ onUseApiKey });
    await screen.findByText(/isn’t in an organization yet/);
    fireEvent.click(screen.getByRole("button", { name: "Create an organization" }));
    expect(openExternal).toHaveBeenCalledWith(GATE_DASHBOARD_URL);
    fireEvent.click(screen.getByRole("button", { name: "Use a Gate API key instead" }));
    expect(onUseApiKey).toHaveBeenCalledTimes(1);
  });

  it("hides the key fallback when no handler is wired", async () => {
    (oauthListOrgs as Mock).mockImplementation(async () => []);
    renderPicker();
    await screen.findByText(/isn’t in an organization yet/);
    expect(screen.queryByRole("button", { name: "Use a Gate API key instead" })).toBeNull();
  });
});

describe("OrgPicker exits", () => {
  it("offers the wrong-account exit in the post-login flow (no back button)", async () => {
    (oauthListOrgs as Mock).mockImplementation(async () => TWO_ORGS);
    const { onReauth } = renderPicker();
    fireEvent.click(await screen.findByRole("button", { name: "Wrong account? Sign out" }));
    expect(onReauth).toHaveBeenCalledTimes(1);
  });

  it("uses the panel header, not the sign-out exit, in the Settings switch flow", async () => {
    (oauthListOrgs as Mock).mockImplementation(async () => TWO_ORGS);
    const onBack = vi.fn();
    renderPicker({ onBack });
    await screen.findByText("Constellation Labs");
    expect(screen.getByRole("heading", { level: 1, name: "Choose organization" })).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Wrong account? Sign out" })).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "Back" }));
    expect(onBack).toHaveBeenCalledTimes(1);
  });
});
