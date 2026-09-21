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
  projects: [
    {
      name: "chromium",
      testIgnore: "live/**",
      use: { ...devices["Desktop Chrome"] },
    },
    // The live suite: the same frontend, driven against the REAL Rust command
    // table hosted by `src-tauri/examples/ui-harness.rs` (see e2e/live/README.md).
    //
    // Its own project because almost nothing it needs is shared. It runs at the
    // window's own size rather than the popover's, it is serial against one
    // stateful backend process, and a retry would re-run a step against state
    // the first attempt already changed - so retries are off here even on CI,
    // where a flake has to be read rather than papered over.
    {
      name: "live",
      testMatch: "live/**/*.spec.ts",
      fullyParallel: false,
      workers: 1,
      retries: 0,
      use: {
        ...devices["Desktop Chrome"],
        viewport: { width: 1280, height: 800 },
      },
    },
  ],
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
  webServer: [
    {
      command: "pnpm exec vite --port 5599 --strictPort --host 127.0.0.1",
      url: "http://127.0.0.1:5599",
      // Pin these tests to the popover, which is no longer the app's default.
      //
      // They assert on popover flows - first run, the org picker, routing counts -
      // in the popover's own copy and layout, so they keep testing the surface
      // they were written against. This line used to say the new shell's
      // routing was inert and could not satisfy them; it is wired
      // (`src/lib/useRouting.ts`), and the `live` project drives it.
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
    // The live backend. Built and started by the script, on a wiped home; the
    // `live` project is the only one that talks to it, and the popover project
    // is unaffected by it being up.
    //
    // The generous timeout is a cold `cargo build` of the example, which on a
    // CI runner with no warm target dir is minutes rather than seconds.
    {
      command: "bash ci/e2e/ui-harness.sh",
      url: "http://127.0.0.1:5610/health",
      reuseExistingServer: !process.env.CI,
      timeout: 600_000,
      stdout: "pipe",
      stderr: "pipe",
    },
  ],
});
