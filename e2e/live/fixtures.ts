/**
 * Fixtures for the live suite: the real frontend, driven against the real Rust
 * commands in `src-tauri/examples/ui-harness.rs`.
 *
 * `e2e/fixtures.ts` boots the popover against a fake backend and hands back a
 * handle for reading that fake's state. There is no such handle here on
 * purpose: the backend's state is on disk and in a real process, so a test
 * reads it the way anything else would - by asking the backend, through
 * `harness.invoke`, or by looking at the files under `home()`.
 *
 * The harness is ONE process for the whole run, holding process-global managers
 * and a single throwaway home. That is why the live project is serial
 * (`playwright.config.ts`), and why specs here read as a sequence rather than as
 * independent cases. It mirrors what `ci/e2e/run.sh` does, and what a person
 * does: state accumulates.
 */
import { test as base, expect, type Page } from "@playwright/test";
import * as fs from "node:fs";
import { installLiveTauri } from "./install";

const HARNESS = process.env.GATE_UI_HARNESS_URL ?? "http://127.0.0.1:5610";

/** The HTTPS mock gateway the relay forwards to. Its cert is the throwaway CA's. */
export const MOCK_GATEWAY = `https://127.0.0.1:${process.env.GATE_UI_HARNESS_MOCK_PORT ?? 8453}`;

/** One request the mock gateway received: what `ci/e2e/mock-gateway.mjs` logs. */
export interface Captured {
  method: string;
  path: string;
  headers: Record<string, string>;
}

export class Harness {
  /** `$HOME` for everything the backend writes: tool configs, preferences, logs. */
  home!: string;
  /** Where `GATE_CONNECT_TEST_SECRETS` puts what would otherwise be in the keychain. */
  secrets!: string;
  /** The mock gateway's capture log. */
  capturePath!: string;

  /**
   * Learn where the harness actually put things, from the harness.
   *
   * Not recomputed here, because the two must agree exactly and the paths are
   * not obvious: `ci/e2e/ui-harness.sh` puts them under the system temp dir
   * rather than in the repo, since the Linux helper daemon binds
   * `$HOME/run/gate-connect/proxyd.sock` and a Unix socket path is capped at
   * ~108 bytes - which a checkout a few directories deep already exceeds.
   */
  async load(): Promise<void> {
    const res = await fetch(`${HARNESS}/health`);
    const body = (await res.json()) as { home: string; secrets: string; capture: string };
    this.home = body.home;
    this.secrets = body.secrets;
    this.capturePath = body.capture;
  }

  /** Call a real backend command directly, outside the UI.
   *
   *  For arranging state a test is not itself asserting on, and for reading
   *  back what a click did. A read here is worth more than one through the
   *  page: it comes from the backend rather than from the UI that might be
   *  wrong about it. */
  async invoke<T = unknown>(cmd: string, payload: Record<string, unknown> = {}): Promise<T> {
    const res = await fetch(`${HARNESS}/invoke`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ cmd, payload }),
    });
    const body = (await res.json()) as { ok?: T; err?: unknown };
    if ("err" in body && body.err !== undefined) {
      throw new Error(`${cmd} failed: ${JSON.stringify(body.err)}`);
    }
    return body.ok as T;
  }

  /** How many events the backend has emitted so far. */
  async eventCount(): Promise<number> {
    const res = await fetch(`${HARNESS}/events?since=0`);
    return ((await res.json()) as { next: number }).next;
  }

  /**
   * Every request the mock gateway has received so far.
   *
   * The same capture log `ci/e2e/run.sh` asserts on, read the same way. This is
   * what "the message went through Gate" means here: not that a command was
   * called, but that a request arrived at the gateway carrying the credential
   * the relay injected.
   */
  captured(): Captured[] {
    if (!fs.existsSync(this.capturePath)) return [];
    return fs
      .readFileSync(this.capturePath, "utf8")
      .split("\n")
      .filter((l) => l.trim().length > 0)
      .map((l) => JSON.parse(l) as Captured);
  }
}

export class LiveApp {
  constructor(
    readonly page: Page,
    readonly harness: Harness,
  ) {}

  /** Every command the frontend has invoked, in order. */
  calls(): Promise<{ cmd: string; args: Record<string, unknown> }[]> {
    return this.page.evaluate(() => window.__GATE_LIVE__.calls);
  }

  /** Args of the last call to `cmd`, or null if it was never invoked. */
  async lastCall(cmd: string): Promise<Record<string, unknown> | null> {
    const hit = (await this.calls()).filter((c) => c.cmd === cmd).pop();
    return hit ? hit.args : null;
  }

  /** The rail switch for one app section ("Claude", "ChatGPT / Codex"). */
  appSwitch(name: string) {
    return this.page.getByRole("switch", { name, exact: true });
  }

  /**
   * Turn an app section on, answering whichever gates it raises.
   *
   * The popover fixture's `routeApp` reads the fake backend's recorded answers
   * to know whether consent will be asked. There is no such oracle here - the
   * answer is a preference on disk that a previous test may have written - so
   * this waits briefly for each gate and carries on if it does not come.
   *
   * The certificate gate is answered rather than asserted away, because whether
   * it appears is a property of the machine, not of the app: `ui-harness.sh`
   * tries to pre-trust the CA with `sudo -n`, which a CI runner allows and a
   * desktop with a sudo password does not. Where it worked there is no dialog;
   * where it did not, this is the real gate, and clicking through it reaches
   * `proxy_trust_ca` and whatever escalation the session can offer. Either way
   * the spec asserts on `ca_trusted` afterwards, which is the fact underneath.
   */
  async routeApp(name: string) {
    await this.appSwitch(name).click();
    const consent = this.page.getByRole("button", { name: `Route ${name}`, exact: true });
    await consent.click({ timeout: 2_000 }).catch(() => {});
    const trust = this.page.getByRole("button", { name: "Trust certificate", exact: true });
    await trust.click({ timeout: 2_000 }).catch(() => {});
  }

  /** Open a section's pane from the rail. */
  openSection(name: string) {
    return this.page.getByRole("button", { name }).first().click();
  }
}

type Fixtures = {
  /** Load the window UI against the live backend and wait for a screen. */
  boot: () => Promise<LiveApp>;
  /** The backend itself, for arranging and for reading back. */
  harness: Harness;
};

export const test = base.extend<Fixtures>({
  harness: async ({}, use) => {
    const harness = new Harness();
    await harness.load();
    await use(harness);
  },
  boot: async ({ page, harness }, use) => {
    const crashes: Error[] = [];
    page.on("pageerror", (err) => crashes.push(err));

    await use(async () => {
      // The window UI, not the popover. The shared Vite server pins
      // `VITE_NEW_UI=0` for the popover suite, and `newUiEnabled()` reads
      // localStorage before that build-time default - so this is the same
      // per-test opt-in the `new-ui-*` specs use, and it costs no second server.
      await page.addInitScript(() => {
        localStorage.setItem("gc.newUi", "1");
        // The one-time "sign in instead of pasting a key" offer, marked answered.
        //
        // It is an artefact of this harness rather than a flow under test:
        // `FirstRun` stamps this key itself when a key connects, and these
        // specs seed the account through the backend instead - so the offer
        // fires on every boot and its overlay swallows the first click of every
        // test. A spec that wants to drive the offer should clear this key.
        localStorage.setItem("gc.oauth-offer.v1.seen", "1");
      });

      // Only events from this page's own boot onward; see `LiveOptions.since`.
      const since = await harness.eventCount();
      await page.addInitScript(installLiveTauri, {
        harness: HARNESS,
        windowLabel: "main",
        since,
      });
      await page.goto("/");
      // No screen renders an h1 until the initial load resolves.
      await page.locator("h1").first().waitFor();
      return new LiveApp(page, harness);
    });

    expect(crashes, `uncaught page errors: ${crashes.map((e) => e.message).join(", ")}`).toEqual([]);
  },
});

export { expect };
