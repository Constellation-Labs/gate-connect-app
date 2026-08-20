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
    routingHealthNotifications: true,
    shareDiagnostics: true,
    version: "v0.1.4",
    onRenameDevice: noop,
    onCopyInstallId: noop,
    onUpgradePlan: noop,
    onReplaceKey: noop,
    onDisconnect: noop,
    onToggleLaunchAtLogin: noop,
    onRetryLaunchAtLogin: noop,
    onToggleRoutingHealthNotifications: noop,
    onToggleShareDiagnostics: noop,
    onRetryPreferences: noop,
    onReplayTutorial: noop,
    onOpenDocs: noop,
    onContactSupport: noop,
    onCheckForUpdates: noop,
    onViewDiagnostics: noop,
    onReviewReset: noop,
    ...overrides,
  });
}

afterEach(cleanup);

describe("the way back to a Gate account", () => {
  it("offers a key account a Gate account, under its key", () => {
    // The popover has always carried this. The new shell had only the one-time
    // OAuthOfferDialog, so dismissing that once left no route to OAuth at all.
    const built = sections({ onSwitchToGateAccount: noop });
    const connection = built.find((sec) => sec.id === "connection")!;
    const ids = connection.rows.map((r) => r.id);

    expect(ids).toContain("sign-in-method");
    expect(ids.indexOf("sign-in-method")).toBe(ids.indexOf("api-key") + 1);

    const row = connection.rows.find((r) => r.id === "sign-in-method")!;
    expect(row.value).toBe("API key");
    expect(row.action?.label).toBe("Use a Gate account");
  });

  it("omits it when the shell does not offer one", () => {
    // An OAuth account has nowhere to switch to, and the shell says so by
    // withholding the handler - the same way it withholds Replace key.
    const built = sections({ onSwitchToGateAccount: undefined });
    const connection = built.find((sec) => sec.id === "connection")!;
    expect(connection.rows.map((r) => r.id)).not.toContain("sign-in-method");
  });

  it("speaks for itself while the browser flow is open", () => {
    const built = sections({
      onSwitchToGateAccount: noop,
      signInNote: "Finish signing in on the page that opened in your browser.",
    });
    const row = built
      .find((sec) => sec.id === "connection")!
      .rows.find((r) => r.id === "sign-in-method")!;

    expect(row.description).toBe(
      "Finish signing in on the page that opened in your browser.",
    );
  });
});

/**
 * An OAuth account keeps its old key in the keychain - the upgrade does not
 * delete it - so `has_api_key` stays true and the pane drew a masked key under
 * "API key" for a session a Cognito bearer authenticates. On the one screen
 * whose job is to say where the credential lives, that named the wrong one, and
 * offered no way to change it either (`onReplaceKey` is withheld for OAuth).
 */
describe("what a Gate account sees under Connection", () => {
  const gateAccount = { authMode: "oauth" as const, onReplaceKey: undefined };

  it("drops the API key row entirely", () => {
    const connection = sections(gateAccount).find((s) => s.id === "connection")!;
    expect(connection.rows.map((r) => r.id)).not.toContain("api-key");
  });

  it("puts the sign-in method in its place rather than going quiet", () => {
    const connection = sections(gateAccount).find((s) => s.id === "connection")!;
    const ids = connection.rows.map((r) => r.id);
    const row = connection.rows.find((r) => r.id === "sign-in-method")!;

    expect(row.value).toBe("Gate account");
    // Where the key row used to be: directly under the gateway.
    expect(ids.indexOf("sign-in-method")).toBe(ids.indexOf("gateway") + 1);
  });

  it("offers no switch to a Gate account it already is", () => {
    const row = sections(gateAccount)
      .find((s) => s.id === "connection")!
      .rows.find((r) => r.id === "sign-in-method")!;
    expect(row.action).toBeUndefined();
  });

  it("keeps Disconnect Gate, which is the row that ends the session", () => {
    const connection = sections(gateAccount).find((s) => s.id === "connection")!;
    expect(connection.rows.map((r) => r.id)).toContain("session");
  });

  it("leaves a key account's rows alone", () => {
    const connection = sections({ authMode: "api_key" }).find((s) => s.id === "connection")!;
    const ids = connection.rows.map((r) => r.id);
    expect(ids).toContain("api-key");
    expect(ids).not.toContain("sign-in-method");
  });

  /** The state before the account read lands. The key row is the older default,
   *  and flashing it away and back is worse than showing it a beat early. */
  it("keeps the key row while the auth mode is still unknown", () => {
    const connection = sections({ authMode: undefined }).find((s) => s.id === "connection")!;
    expect(connection.rows.map((r) => r.id)).toContain("api-key");
  });
});

describe("buildSettingsSections", () => {
  it("keeps Diagnostics reachable from Settings", () => {
    // The Figma does not draw this row. `screens/Diagnostics.tsx` has nowhere
    // else to live in the new IA, so losing it here loses the feature. It now
    // lives in its own section rather than under About.
    const diagnostics = sections().find((s) => s.id === "diagnostics");
    expect(diagnostics?.rows.map((r) => r.id)).toContain("diagnostics-report");
  });

  it("marks only the two destructive actions destructive", () => {
    const destructive = sections()
      .flatMap((s) => s.rows)
      .filter((r) => r.action?.destructive)
      .map((r) => r.id);
    expect(destructive).toEqual(["session", "reset"]);
  });

  it("marks removing the certificate destructive too", () => {
    // The third red action on the screen, and a deliberate one: until it is
    // trusted again, nothing routed through the local proxy is inspected.
    const rows = sections({ certificate: "Trusted", onRemoveCertificate: noop }).flatMap(
      (s) => s.rows,
    );
    expect(rows.filter((r) => r.action?.destructive).map((r) => r.id)).toEqual([
      "certificate",
      "session",
      "reset",
    ]);
  });

  it("renders identifiers in mono and prose in sans", () => {
    // CLAUDE.md's one typography rule the new design did not overturn.
    const mono = sections()
      .flatMap((s) => s.rows)
      .filter((r) => r.mono)
      .map((r) => r.id);
    // The gateway joined them: a base URL is an identifier, which is exactly
    // what CLAUDE.md reserves mono for.
    expect(mono).toEqual(["install-id", "gateway", "api-key", "version"]);
  });
});

describe("buildSettingsSections: rows with nothing behind them", () => {
  // The alternative is a control that visibly does nothing, which the user
  // cannot tell from broken. Regressing this looks like "add a noop handler".
  it("omits an action whose handler is absent", () => {
    const device = sections({ onRenameDevice: undefined }).find((s) => s.id === "device");
    expect(device?.rows.find((r) => r.id === "device")?.action).toBeUndefined();
    // The row itself survives: the device name is still worth reading.
    expect(device?.rows.map((r) => r.id)).toContain("device");
  });

  it("omits a row left with nothing to do at all", () => {
    // Active session is only ever a button, and Notifications only ever a
    // switch, so an absent handler leaves an inert label.
    const ids = sections({
      onDisconnect: undefined,
      onToggleRoutingHealthNotifications: undefined,
    })
      .flatMap((s) => s.rows)
      .map((r) => r.id);
    expect(ids).not.toContain("session");
    expect(ids).not.toContain("routing-health");
  });

  /**
   * A failed read must not draw a switch. `false` and "could not be read" look
   * identical on a toggle, and the user cannot tell one from a setting they
   * turned off themselves.
   */
  it("replaces a switch with Unavailable and Retry when its value never loaded", () => {
    const startup = sections({ launchAtLoginUnavailable: true }).find(
      (s) => s.id === "startup",
    );
    const launch = startup?.rows.find((r) => r.id === "launch");
    expect(launch?.toggle).toBeUndefined();
    expect(launch?.unavailable).toBeDefined();
  });

  it("marks both preference switches unavailable together, since they share one read", () => {
    const built = sections({ preferencesUnavailable: true });
    const rows = built.flatMap((s) => s.rows);
    expect(rows.find((r) => r.id === "routing-health")?.unavailable).toBeDefined();
    expect(rows.find((r) => r.id === "share-diagnostics")?.unavailable).toBeDefined();
  });

  /** Only the preference switches; a failed preferences read says nothing about
   * launch-at-login, which is a separate command. */
  it("does not spread one failed read onto an unrelated row", () => {
    const startup = sections({ preferencesUnavailable: true }).find((s) => s.id === "startup");
    expect(startup?.rows.find((r) => r.id === "launch")?.toggle).toBeDefined();
  });

  it("omits the whole Danger zone when reset is not wired", () => {
    // A card drawn to alarm that does nothing teaches the user to ignore it.
    expect(sections({ onReviewReset: undefined }).map((s) => s.id)).not.toContain("danger");
  });

  it("keeps every other section when the unwired ones drop out", () => {
    const ids = sections({
      onRenameDevice: undefined,
      onUpgradePlan: undefined,
      onDisconnect: undefined,
      onToggleRoutingHealthNotifications: undefined,
      onToggleShareDiagnostics: undefined,
      onCheckForUpdates: undefined,
      onOpenDocs: undefined,
      onContactSupport: undefined,
      onReviewReset: undefined,
    }).map((s) => s.id);
    expect(ids).toEqual(["device", "account", "connection", "startup", "diagnostics", "about"]);
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
      "Notifications",
      "Diagnostics",
      "About",
      "Help",
      "Danger zone",
    ]) {
      expect(screen.getByRole("heading", { name: heading })).toBeTruthy();
    }
    expect(screen.getByText("MacBook Pro")).toBeTruthy();
    expect(screen.getByRole("button", { name: "View report" })).toBeTruthy();
  });

  it("drives each switch from its own value, not from the row label", () => {
    render(
      <SettingsPane
        sections={sections({ launchAtLogin: false, routingHealthNotifications: true })}
      />,
    );

    expect(
      screen.getByRole("switch", { name: "Launch at login" }).getAttribute("aria-checked"),
    ).toBe("false");
    expect(
      screen.getByRole("switch", { name: "Routing health" }).getAttribute("aria-checked"),
    ).toBe("true");
  });

  it("renders Unavailable and a Retry in place of a switch that could not load", () => {
    render(<SettingsPane sections={sections({ launchAtLoginUnavailable: true })} />);

    expect(screen.queryByRole("switch", { name: "Launch at login" })).toBeNull();
    expect(screen.getByText("Unavailable")).toBeTruthy();
    expect(screen.getByRole("button", { name: "Retry" })).toBeTruthy();
  });
});
