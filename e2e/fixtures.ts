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

  /** The switch that turns routing on and off. */
  get routingSwitch() {
    return this.page.getByRole("switch", { name: "Route through Gate" });
  }

  /** A family row on Home ("Claude", "OpenAI", "Other tools"). */
  familyRow(name: string) {
    return this.page.getByRole("button", { name: `${name} details` });
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
