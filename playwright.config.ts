import { defineConfig, devices } from "@playwright/test";

/**
 * Browser-level e2e for the popover UI.
 *
 * These drive the real frontend bundle against a fake Tauri backend (see
 * e2e/install.ts); the Rust side has its own suites under the workspace
 * crates and in `ci/e2e/run.sh`. One browser, because the app ships in one
 * engine per platform and none of them is a matrix we can reproduce here -
 * the value is in exercising App's orchestration, not cross-browser coverage.
 */
export default defineConfig({
  testDir: "./e2e",
  // The popover is one room: 360px wide, ~520px tall. Layout assertions are
  // only meaningful at the size the window actually is.
  use: {
    baseURL: "http://127.0.0.1:5599",
    viewport: { width: 360, height: 520 },
    trace: "on-first-retry",
  },
  projects: [{ name: "chromium", use: { ...devices["Desktop Chrome"] } }],
  fullyParallel: true,
  forbidOnly: !!process.env.CI,
  retries: process.env.CI ? 1 : 0,
  workers: process.env.CI ? 2 : undefined,
  // On CI, the HTML report is what the failure artifact contains; `github`
  // puts the failures inline on the PR diff.
  reporter: process.env.CI
    ? [["github"], ["list"], ["html", { open: "never" }]]
    : [["list"]],
  // Its own port, so a `pnpm app` already running on 5173 (strictPort) is
  // neither clobbered nor reused with a different build.
  //
  // `--host 127.0.0.1` is load-bearing, not tidiness. Vite's default host is
  // `localhost`, which it resolves through DNS: on a runner with IPv6 that
  // comes back `::1` first, so the dev server listened on `[::1]:5599` while
  // the probe below knocked on `127.0.0.1:5599` and never got an answer - the
  // server was up the whole time and the run died on the 60s webServer
  // timeout, with no test having started. Binding the family explicitly makes
  // the two agree on every runner.
  webServer: {
    command: "pnpm exec vite --port 5599 --strictPort --host 127.0.0.1",
    url: "http://127.0.0.1:5599",
    // Pin these tests to the popover, which is no longer the app's default.
    //
    // They assert on popover flows - first run, the org picker, routing counts -
    // and the new window UI cannot satisfy them: its routing actions are inert,
    // so there is nothing for "turn routing on" to observe. Rewriting them
    // against the new shell before it can route would mean asserting on a UI
    // that does not work yet, so they keep testing the surface that does.
    //
    // Retire this line together with the popover screens. `newUiEnabled()`
    // reads localStorage first and a fresh browser context has none, so the
    // build-time default is what decides here.
    env: { VITE_NEW_UI: "0" },
    reuseExistingServer: !process.env.CI,
    timeout: 60_000,
    // So the next startup failure says why instead of only that it timed out.
    stdout: "pipe",
    stderr: "pipe",
  },
});
