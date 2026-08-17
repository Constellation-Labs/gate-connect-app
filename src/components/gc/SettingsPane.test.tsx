import { afterEach, describe, expect, it } from "vitest";
import { cleanup, render, screen } from "@testing-library/react";
import { buildSettingsSections, SettingsPane } from "./SettingsPane";

const noop = () => {};

function sections(overrides: Partial<Parameters<typeof buildSettingsSections>[0]> = {}) {
  return buildSettingsSections({
    deviceName: "MacBook Pro",
    installId: "gc_a1b2c3d4",
    loginId: "jdoe@acme.com",
    plan: "Free",
    gateway: "Managed by Gate",
    apiKeyMasked: "sk-gw***********",
    launchAtLogin: true,
    notifications: true,
    version: "v0.1.4",
    onRenameDevice: noop,
    onCopyInstallId: noop,
    onUpgradePlan: noop,
    onReplaceKey: noop,
    onDisconnect: noop,
    onToggleLaunchAtLogin: noop,
    onToggleNotifications: noop,
    onReplayTutorial: noop,
    onCheckForUpdates: noop,
    onViewDiagnostics: noop,
    onReviewReset: noop,
    ...overrides,
  });
}

afterEach(cleanup);

describe("buildSettingsSections", () => {
  it("keeps Diagnostics reachable from Settings", () => {
    // The Figma does not draw this row. `screens/Diagnostics.tsx` has nowhere
    // else to live in the new IA, so losing it here loses the feature.
    const about = sections().find((s) => s.id === "about");
    expect(about?.rows.map((r) => r.id)).toContain("diagnostics");
  });

  it("marks only the two destructive actions destructive", () => {
    const destructive = sections()
      .flatMap((s) => s.rows)
      .filter((r) => r.action?.destructive)
      .map((r) => r.id);
    expect(destructive).toEqual(["session", "reset"]);
  });

  it("renders identifiers in mono and prose in sans", () => {
    // CLAUDE.md's one typography rule the new design did not overturn.
    const mono = sections()
      .flatMap((s) => s.rows)
      .filter((r) => r.mono)
      .map((r) => r.id);
    expect(mono).toEqual(["install-id", "api-key", "version"]);
  });
});

describe("SettingsPane", () => {
  it("renders every section heading and row label", () => {
    render(<SettingsPane sections={sections()} />);

    for (const heading of [
      "Device",
      "Account",
      "Connection",
      "Startup",
      "About",
      "Danger zone",
    ]) {
      expect(screen.getByRole("heading", { name: heading })).toBeTruthy();
    }
    expect(screen.getByText("MacBook Pro")).toBeTruthy();
    expect(screen.getByRole("button", { name: "View report" })).toBeTruthy();
  });

  it("drives the Startup switches from intent, not from the row label", () => {
    render(<SettingsPane sections={sections({ launchAtLogin: false })} />);

    const launch = screen.getByRole("switch", { name: "Launch at login" });
    expect(launch.getAttribute("aria-checked")).toBe("false");
    expect(
      screen.getByRole("switch", { name: "Notifications" }).getAttribute("aria-checked"),
    ).toBe("true");
  });
});
