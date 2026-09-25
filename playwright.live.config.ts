import { randomBytes } from "node:crypto";
import { defineConfig, devices } from "@playwright/test";

/**
 * The harness port and its token, settled here and inherited by both children:
 * `ci/e2e/ui-harness.sh` reads them from the environment, and the specs read
 * the same variables. One definition, so a port change cannot desynchronise the
 * server from the client.
 *
 * The token is what makes the port safe to open. `/invoke` reaches every command
 * the app has, and a page in the developer's browser can post to loopback, so a
 * fresh secret per run is the difference between a test fixture and an open
 * remote control on the machine.
 */
const PORT = process.env.GATE_UI_HARNESS_PORT ?? "5610";
process.env.GATE_UI_HARNESS_PORT = PORT;
process.env.GATE_UI_HARNESS_TOKEN ??= randomBytes(32).toString("hex");

/**
 * The live suite: the real frontend driven against the REAL Rust command table.
 *
 * Its own config rather than a project inside `playwright.config.ts`, and that
 * is not tidiness. Playwright resolves `webServer` per config file, never per
 * project, so a `live` project there started the Rust harness on every
 * `pnpm test:e2e` - including `--project=chromium` and the CI `web` job, which
 * installs no Rust toolchain and no webkit deps and would have sat through the
 * harness's build timeout before failing. Splitting the config is what makes
 * "run the popover suite" and "run the live suite" genuinely separate.
 *
 * See `e2e/live/README.md` for what this suite covers and what it cannot.
 */
export default defineConfig({
  testDir: "./e2e/live",
  use: {
    baseURL: "http://127.0.0.1:5600",
    // The window's own size. The popover suite's 360x520 would hide the rail
    // these specs click.
    viewport: { width: 1280, height: 800 },
    trace: "on-first-retry",
  },
  projects: [{ name: "live", use: { ...devices["Desktop Chrome"] } }],
  // One stateful backend process for the whole run, so the specs are a sequence
  // and not a set. A retry would re-run a step against state the first attempt
  // already changed, which is why retries stay off even on CI: a flake here has
  // to be read rather than papered over.
  fullyParallel: false,
  workers: 1,
  retries: 0,
  forbidOnly: !!process.env.CI,
  reporter: process.env.CI
    ? [["github"], ["list"], ["html", { open: "never" }]]
    : [["list"]],
  webServer: [
    {
      // Its own port and its own server: 5599 belongs to the popover config,
      // which pins `VITE_NEW_UI=0`. `--host 127.0.0.1` for the IPv6 reason
      // spelled out in `playwright.config.ts`.
      command: "pnpm exec vite --port 5600 --strictPort --host 127.0.0.1",
      url: "http://127.0.0.1:5600",
      reuseExistingServer: !process.env.CI,
      timeout: 60_000,
      stdout: "pipe",
      stderr: "pipe",
    },
    {
      // The live backend, on a wiped home.
      //
      // Never reused, even locally: `ci/e2e/ui-harness.sh` wipes its work
      // directory at startup, and every spec here is written against that
      // clean state. Adopting a harness left over from the last run would mean
      // the first spec asserting "no account on disk" against an account the
      // previous run created.
      //
      // The timeout is a cold `cargo build` of the example, which on a runner
      // with no warm target dir is minutes rather than seconds.
      command: "bash ci/e2e/ui-harness.sh",
      url: `http://127.0.0.1:${PORT}/health`,
      reuseExistingServer: false,
      timeout: 600_000,
      stdout: "pipe",
      stderr: "pipe",
    },
  ],
});
