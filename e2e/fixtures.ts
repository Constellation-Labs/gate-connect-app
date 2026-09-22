/**
 * Playwright fixtures: boot the popover against a fake backend.
 *
 * A spec calls `boot()` with the deltas it cares about and gets back a handle
 * for reading the backend's state, its call log, and pushing events at the
 * frontend. Everything else - the app itself - is the real thing.
 */
import { test as base, expect, type Page } from "@playwright/test";
import { defaultState, merge, type BackendState, type DeepPartial } from "./backend";
import { installFakeTauri } from "./install";

export class App {
  constructor(readonly page: Page) {}

  /** The backend's state as it stands now, after whatever the UI just did. */
  state(): Promise<BackendState> {
    return this.page.evaluate(() => window.__GATE_E2E__.state);
  }

  /** Every command the frontend has invoked, in order. */
  calls(): Promise<{ cmd: string; args: Record<string, unknown> }[]> {
    return this.page.evaluate(() => window.__GATE_E2E__.calls);
  }

  /** Args of the last call to `cmd`, or null if it was never invoked. */
  async lastCall(cmd: string): Promise<Record<string, unknown> | null> {
    const calls = await this.calls();
    const hit = calls.filter((c) => c.cmd === cmd).pop();
    return hit ? hit.args : null;
  }

  /** Push a backend event the way the Rust side does. */
  emit(event: string, payload?: unknown): Promise<void> {
    return this.page.evaluate(
      ([e, p]) => window.__GATE_E2E__.emit(e as string, p),
      [event, payload] as const,
    );
  }

  /** Mutate the backend's state from the test, for what changes out of band -
   *  a token expiring while the popover was closed, a tool drifting. */
  async patch(patch: DeepPartial<BackendState>): Promise<void> {
    await this.page.evaluate((p) => {
      const s = window.__GATE_E2E__.state as Record<string, any>;
      for (const [k, v] of Object.entries(p as Record<string, unknown>)) {
        s[k] =
          v !== null && typeof v === "object" && !Array.isArray(v)
            ? { ...s[k], ...(v as object) }
            : v;
      }
    }, patch as Record<string, unknown>);
  }

  /** A section row on Home ("Claude", "ChatGPT / Codex", "Terminal"). */
  familyRow(name: string) {
    return this.page.getByRole("button", { name: `${name} details` });
  }

  /** Open one app's pane from the rail.
   *
   *  One button per row since the switch went: the row's own select control.
   *  Its accessible name leads with the app name and carries the status after
   *  it, so this matches on the name rather than requiring the whole string. */
  openApp(name: string) {
    return this.page
      .getByRole("listitem")
      .filter({ has: this.page.getByRole("button", { name }) })
      .getByRole("button", { name })
      .first()
      .click();
  }

  /** The TRAY's switch for one app row, which the tray still has.
   *
   *  The window's rail lost its switches on 2026-09-22 and the tray did not -
   *  it is a different surface and design scoped the change to the rail. A
   *  tray spec must therefore address its own control rather than go through
   *  {@link appSwitch}, which now opens a window pane the tray has not got. */
  trayAppSwitch(name: string) {
    return this.page.getByRole("switch", { name, exact: true });
  }

  /** {@link routeApp} for the tray, clicking the row's own switch. */
  async routeTrayApp(name: string) {
    const section = SESSION_SECTIONS[name];
    const asked =
      section !== undefined &&
      !(await this.state()).preferences.session_routing_accepted.includes(section);
    await this.trayAppSwitch(name).click();
    if (!asked) return;
    await this.page.getByRole("button", { name: `Route ${name}` }).click();
  }

  /**
   * The switch for one app section - on that app's own PANE.
   *
   * The rail had one until 2026-09-22 and this returned it, named for the app
   * alone. Design removed it: a rail row drew the app's state and a control
   * for its intent on one line, and those are different questions. Routing
   * happens on the pane now, where there is room to say what the switch will
   * do before it is flipped.
   *
   * So this opens the pane first. Every caller that only ever clicked reads
   * the same; the ones that ASSERT on it without clicking used to be able to
   * do so from any pane and now cannot, which is honest - there is one switch
   * on screen and it belongs to the app you are looking at.
   *
   * The pane's label is `Route <name>`, not `<name>`, which is also what
   * disambiguates it from the tray's rows in a shared DOM.
   */
  async appSwitch(name: string) {
    await this.openApp(name);
    return this.page.getByRole("switch", { name: `Route ${name}`, exact: true });
  }

  /**
   * Turn an app section ON, answering the consent dialog if it asks.
   *
   * A section switch routes every surface that app uses, and for Claude and
   * ChatGPT / Codex that includes a surface the person is signed in to - so the
   * switch asks once before it flips one. Most specs are about something else
   * and should not each carry that step; the ones testing consent itself click
   * the switch directly and assert on the dialog.
   *
   * Only for turning ON. Switching off needs no permission, and a helper that
   * hid a confirmation on the way out would hide a bug.
   */
  async routeApp(name: string) {
    const section = SESSION_SECTIONS[name];
    // Asked once per install, so a second ON in the same test gets no dialog and
    // waiting for one would hang. Read from the fake backend's own recording
    // rather than guessed, which is the same thing the app reads.
    const asked =
      section !== undefined &&
      !(await this.state()).preferences.session_routing_accepted.includes(section);
    await (await this.appSwitch(name)).click();
    if (!asked) return;
    // Asserted rather than probed. `isVisible()` does not auto-wait, so a probe
    // would race the dialog's first paint and silently skip it; clicking waits.
    // And if consent ever stops being asked for one of these, this is the line
    // that should fail - that is the regression worth catching, not a helper
    // quietly carrying on.
    await this.page.getByRole("button", { name: `Route ${name}`, exact: true }).click();
  }

  /**
   * Open a section's pane from the rail.
   *
   * The rail draws one row per app section, so a tool is reached through the
   * section that holds it - Codex through "ChatGPT / Codex". `.first()` because
   * the row's accessible name is the section name plus its status, and a pane
   * header can repeat the name once the pane is open.
   *
   * Four specs had grown their own copy of this line; the pane is where the
   * per-tool notices are drawn (#277 and the reopen card), so it is a fixture
   * now rather than a helper per file.
   *
   * It carries the reason those specs open a pane at all, from the copy this
   * replaced: a 250px rail row cannot fit "Not protected - Configuration update
   * failed" and truncates the reason mid-word, so the row prints the coloured
   * phrase alone and the pane carries the reason in full. A spec that wants the
   * phrase reads the row; one that wants the reason, or a notice, opens the
   * pane.
   */
  openSection(name: string) {
    return this.page.getByRole("button", { name }).first().click();
  }

  openSettings() {
    return this.page.getByRole("button", { name: "Settings" }).click();
  }
}

type Fixtures = {
  /** Install the fake backend, load the popover, wait for it to resolve a
   *  screen. `patch` is merged one level deep into the default state. */
  boot: (patch?: DeepPartial<BackendState>) => Promise<App>;
};

/**
 * The sections whose switch asks before it routes, against the default catalog,
 * and the section id each one records its answer under.
 *
 * They are the ones holding a `Credential::Additive` row - a surface the person
 * is signed in to. Listed here rather than derived because a spec that overrides
 * `proxy.domains` can change the answer, and such a spec should drive the switch
 * itself rather than through `routeApp`. The id is what lets the helper tell a
 * first ON from a later one, since the question is asked once per install.
 */
const SESSION_SECTIONS: Record<string, string> = {
  Claude: "claude",
  "ChatGPT / Codex": "chatgpt",
};

export const test = base.extend<Fixtures>({
  boot: async ({ page }, use) => {
    // A rejected invoke that no screen renders is still a bug; the app is
    // meant to catch every one of them. Console errors from React (act
    // warnings, key warnings) are not what this suite is for, so only
    // uncaught page errors fail the test.
    const crashes: Error[] = [];
    page.on("pageerror", (err) => crashes.push(err));

    await use(async (patch = {}) => {
      const state = merge(defaultState(), patch);
      await page.addInitScript(installFakeTauri, state);
      await page.goto("/");
      // No screen renders an h1 until the initial load resolves; the loading
      // lockup is a span. So the first h1 is the app deciding where to land.
      await page.locator("h1").first().waitFor();
      return new App(page);
    });

    expect(crashes, `uncaught page errors: ${crashes.map((e) => e.message).join(", ")}`).toEqual([]);
  },
});

export { expect };
