import { test, expect } from "./fixtures";

/**
 * The controls the window shell was drawn with and never wired: the
 * certificate, the app pane's switch, the diagnostics probes, and the one-time
 * OAuth offer. (The shell-environment channel had a card here too, until it
 * turned out no frame drew it. So did the engine's own switch, until routing
 * started following the app.)
 *
 * All of these existed as backend commands the whole time - the popover reaches
 * every one of them. What could not be tested at the hook level is exactly what
 * was broken here: whether the control on screen is connected to the action at
 * all. `onToggleProtected={noop}` type-checks perfectly.
 *
 * Same per-test opt-in as the other new-UI specs: the suite is pinned to the
 * popover, and `newUiEnabled()` reads localStorage before the build-time default.
 */
const useNewUi = { gc: "gc.newUi" };

const CLAUDE_CODE = {
  slug: "claude-code",
  // The tool's own row label. The rail draws a row per APP now, so this reaches
  // the flat-list surfaces rather than the rail - see `displayName` below for
  // the other half of that split.
  name: "CLI",
  // The product name, which is what `teardown_report` carries - that dialog has
  // no family heading, so the row label alone would name nothing there.
  displayName: "Claude Code",
  upstream_provider_name: "Anthropic",
  default_upstream_url: "https://api.anthropic.com",
  status: { kind: "detected" as const },
};

test.describe("new UI engine controls", () => {
  test.beforeEach(async ({ page }) => {
    await page.addInitScript((k) => localStorage.setItem(k.gc, "1"), useNewUi);
  });

  /**
   * Routing follows the app: it starts at launch and stops at quit, so there is
   * nothing on screen for the user to set. The rail drew this switch above the
   * families it governed, and it is the thing this branch removes.
   */
  test("the rail draws no routing switch", async ({ boot }) => {
    const app = await boot({ proxy: { running: false, ca_trusted: true } });

    await expect(
      app.page.getByRole("switch", { name: "Route traffic through Gate" }),
    ).toHaveCount(0);
    // And nothing took its place: a window whose launch enable failed reports
    // it per app, it does not offer a machine-wide control.
    await expect(app.page.getByText("Everything below stays off")).toHaveCount(0);
  });

  /**
   * AG-570 AC 8: a teardown that cannot put every tool back says which ones.
   *
   * Read back from the configs rather than assembled from what the sweep
   * believed it wrote - a sweep that returns success having written nothing is
   * the failure this report exists to catch.
   *
   * Driven from Reset rather than from routing-off, which is where it used to
   * be driven from. There is no routing-off control any more; the quit's own
   * teardown reports on its own dialog (`new-ui-quit.spec.ts`), and Reset is
   * the remaining caller of `onTeardown("teardown")`.
   *
   * A reset that *finishes* has nothing to report - `clear_account`
   * disconnects on its way through - so this is the branch where it aborts
   * with a tool still on Gate's settings. `confirmReset` reports on that path
   * deliberately: the error says the reset stopped, the report says which tool
   * stopped it.
   */
  test("a teardown lists the tools it could not put back", async ({ boot }) => {
    const app = await boot({
      proxy: { running: true, ca_trusted: true },
      // Left connected by the sweep, which is what a best-effort teardown does
      // when one tool's write fails.
      tools: [{ ...CLAUDE_CODE, status: { kind: "connected" as const } }],
      failures: { clear_account: "permission denied writing ~/.claude/settings.json" },
    });

    await app.page.getByRole("button", { name: "Settings" }).click();
    await app.page.getByRole("button", { name: "Review reset" }).click();
    await app.page.getByRole("checkbox").check();
    await app.page.getByRole("button", { name: "Reset Gate Connect" }).click();

    const dialog = app.page.getByRole("dialog");
    await expect(dialog).toBeVisible();
    await expect(dialog.getByText("Some tools were left as they were")).toBeVisible();
    await expect(dialog.getByText("Still using Gate’s values")).toBeVisible();
    await expect(dialog.getByText("Claude Code")).toBeVisible();
    // The next action per tool, which the AC asks for by name.
    await expect(dialog.getByText("Retry disconnect")).toBeVisible();
  });

  test("a chat domain starts the engine rather than routing nothing", async ({ boot }) => {
    // `proxy_set_domain` only records the flag, so with the engine off this
    // writes intent and routes nothing. Rare now that the launch enables the
    // engine, and this is the window where it is not rare: a launch whose
    // enable did not complete.
    const app = await boot({ proxy: { running: false, ca_trusted: true } });

    // A row whose surfaces are all host-intercepted: no config file to write, so
    // the engine is the only thing that could route it. The OpenAI API row
    // rather than a session app, so no consent dialog stands between the click
    // and the flag - that is tested on its own in the routing spec.
    await app.page.getByRole("switch", { name: "OpenAI API" }).click();

    await expect.poll(() => app.lastCall("proxy_set_domain")).toMatchObject({
      slug: "openai",
      enabled: true,
    });
    const cmds = (await app.calls()).map((c) => c.cmd);
    expect(cmds.indexOf("proxy_enable")).toBeGreaterThan(-1);
    expect(cmds.indexOf("proxy_enable")).toBeLessThan(cmds.indexOf("proxy_set_domain"));
  });

});

test.describe("new UI app pane", () => {
  test.beforeEach(async ({ page }) => {
    await page.addInitScript((k) => localStorage.setItem(k.gc, "1"), useNewUi);
  });

  test("the pane's own switch routes the app", async ({ boot }) => {
    const app = await boot({ proxy: { running: true, ca_trusted: true }, tools: [CLAUDE_CODE] });

    // The rail row is the app; the pane it opens is the app's.
    await app.page.getByRole("button", { name: "Claude" }).first().click();
    await app.page.getByRole("switch", { name: "Route Claude" }).click();
    // The pane's switch is the section's switch, so it asks the same question
    // the rail's does before it routes a surface the person is signed in to.
    await app.page.getByRole("button", { name: "Route Claude", exact: true }).click();

    await expect.poll(() => app.lastCall("connect_tool")).toMatchObject({
      slug: "claude-code",
    });
  });

  test("the switch reads intent, not the routing verdict", async ({ boot }) => {
    // A drifted tool is one the user asked to route: driving this switch from the
    // observed status renders it off, and clicking it then turns off the setting
    // the user was trying to turn on. That is the bug `lib/groups.ts` documents,
    // and this pane had it.
    const app = await boot({
      proxy: { running: true, ca_trusted: true },
      tools: [
        { ...CLAUDE_CODE, status: { kind: "drifted" as const, reason: "ANTHROPIC_BASE_URL" } },
      ],
    });

    await app.page.getByRole("button", { name: "Claude" }).first().click();

    // The drift alert card inside the pane carries its own switch for the same
    // app, reading off - that one is the re-adopt path. This is the header's.
    await expect(
      app.page.getByRole("switch", { name: "Route Claude" }),
    ).toHaveAttribute("aria-checked", "true");
  });
});

test.describe("new UI certificate and diagnostics", () => {
  test.beforeEach(async ({ page }) => {
    await page.addInitScript((k) => localStorage.setItem(k.gc, "1"), useNewUi);
  });

  test("removing the certificate is confirmed first", async ({ boot }) => {
    const app = await boot({ proxy: { running: true, ca_trusted: true } });

    await app.page.getByRole("button", { name: "Settings" }).click();
    await app.page.getByRole("button", { name: "Remove certificate" }).click();

    await expect(
      app.page.getByRole("heading", { name: "Remove the Gate certificate?" }),
    ).toBeVisible();

    // The two sentences the dialog exists to say, both of which have been
    // wrong in the tree. Routing does NOT stay on: `untrust_ca` sequences a
    // stop of its own, because the engine mints leaves the OS rejects the
    // moment the root is untrusted. And a tool that is already running keeps
    // the certificate bundle it loaded at startup, so it fails to connect
    // after a new one is trusted while Gate's rows still read Protected -
    // an hour of `APIConnectionError` in Hermes, measured 2026-09-21.
    const dialog = app.page.getByRole("dialog");
    await expect(dialog.getByText("Routing turns off")).toBeVisible();
    await expect(
      dialog.getByText("quit and reopen any AI tools"),
    ).toBeVisible();

    expect(await app.lastCall("proxy_untrust_ca")).toBeNull();

    await app.page.getByRole("button", { name: "Remove certificate" }).last().click();

    await expect.poll(() => app.lastCall("proxy_untrust_ca")).not.toBeNull();
  });

  test("an untrusted certificate is not offered for removal", async ({ boot }) => {
    const app = await boot({ proxy: { running: true, ca_trusted: false } });

    await app.page.getByRole("button", { name: "Settings" }).click();

    // Exact: this assertion is about the Settings row's own value, not about
    // any other surface that happens to use the same words in a sentence.
    await expect(app.page.getByText("Not trusted", { exact: true })).toBeVisible();
    await expect(app.page.getByRole("button", { name: "Remove certificate" })).toHaveCount(0);
  });

  test("the report is built from live probes", async ({ boot }) => {
    // It used to pass `backend: null`, `oauth: null`, `agents: null` and
    // `clientsStale: false` - four sections the popover fills in, and the last of
    // those is a claim rather than an unknown.
    const app = await boot({
      proxy: { running: true, ca_trusted: true },
      runningAgentNames: ["claude"],
    });

    await app.page.getByRole("button", { name: "Settings" }).click();
    await app.page.getByRole("button", { name: "View report" }).click();

    // Scoped: the Settings pane has a Diagnostics section heading of its own.
    const report = app.page.getByRole("dialog");
    await expect(report.getByRole("heading", { name: "Diagnostics" })).toBeVisible();
    for (const cmd of ["diagnostics", "routed_clients_stale", "running_agents"]) {
      await expect.poll(() => app.calls().then((c) => c.some((x) => x.cmd === cmd))).toBe(true);
    }
    // From the snapshot, not from a hard-coded unknown.
    await expect(report).toContainText("aarch64");
    await expect(report).toContainText("claude");
  });

  test("the API key row shows the stored prefix rather than a made-up one", async ({ boot }) => {
    const app = await boot({
      account: {
        gateway_base_url: "https://gw.example",
        has_api_key: true,
        auth_mode: "api_key",
      },
      accountKeyPrefix: "sk-gw-live-7f2",
      localStorage: { "gc.oauth-offer.v1.seen": "1" },
    });

    await app.page.getByRole("button", { name: "Settings" }).click();

    await expect(app.page.getByText(/^sk-gw-live-7f2\*+$/)).toBeVisible();
  });
});

test.describe("new UI device and install id", () => {
  test.beforeEach(async ({ page }) => {
    await page.addInitScript((k) => localStorage.setItem(k.gc, "1"), useNewUi);
  });

  test("both rows read from the backend rather than sitting empty", async ({ boot }) => {
    // Device was hardcoded "-", and the install id came from the analytics client:
    // absent in a build with no PostHog key, and absent again once diagnostics are
    // switched off, so the row said Unavailable on an ordinary dev build.
    const app = await boot({ installId: "install-abc-123", hostName: "e2e-macbook" });

    await app.page.getByRole("button", { name: "Settings" }).click();

    await expect(app.page.getByText("e2e-macbook")).toBeVisible();
    await expect(app.page.getByText("install-abc-123")).toBeVisible();
    await expect(app.page.getByRole("button", { name: "Copy ID" })).toBeVisible();
  });

  test("the device is renameable, and the stored name wins over the hostname", async ({
    boot,
  }) => {
    const app = await boot({ hostName: "e2e-macbook" });

    await app.page.getByRole("button", { name: "Settings" }).click();
    await app.page.getByRole("button", { name: "Rename device" }).click();

    // Prefilled with what is being replaced, and the read-only row above it says
    // the same thing.
    // By role: `ModalField` also renders a "Clear New device name" button once the
    // field has a value, so the label matches two elements.
    const field = app.page.getByRole("textbox", { name: "New device name" });
    await expect(field).toHaveValue("e2e-macbook");

    await field.fill("Studio Mac");
    await app.page.getByRole("button", { name: "Rename device" }).last().click();

    await expect.poll(() => app.lastCall("set_device_name")).toEqual({ name: "Studio Mac" });
    await expect(app.page.getByRole("dialog")).toHaveCount(0);
    await expect(app.page.getByText("Studio Mac")).toBeVisible();
  });

  test("an unreadable name says so instead of offering a blind rename", async ({ boot }) => {
    const app = await boot({ failures: { device_name: "app support dir unavailable" } });

    await app.page.getByRole("button", { name: "Settings" }).click();

    await expect(app.page.getByText("Unavailable").first()).toBeVisible();
    await expect(app.page.getByRole("button", { name: "Rename device" })).toHaveCount(0);
  });
});

test.describe("new UI OAuth offer", () => {
  test.beforeEach(async ({ page }) => {
    await page.addInitScript((k) => localStorage.setItem(k.gc, "1"), useNewUi);
  });

  /** The flag counts as answered only when it reads "1", which is how a spec
   *  asks for an install that has never been offered - `merge` folds the record
   *  into the default rather than replacing it, so the key cannot be removed. */
  const notOffered = { "gc.oauth-offer.v1.seen": "" };

  test("a key-based account is offered sign-in once", async ({ boot }) => {
    const app = await boot({
      account: {
        gateway_base_url: "https://gw.example",
        has_api_key: true,
        auth_mode: "api_key",
      },
      localStorage: notOffered,
    });

    await expect(
      app.page.getByRole("heading", { name: "Sign in instead of pasting a key" }),
    ).toBeVisible();

    await app.page.getByRole("button", { name: "Sign in with Constellation" }).click();

    // Not `save_account`: that would repoint the account at the default gateway
    // and drop the key the user still has.
    await expect.poll(() => app.lastCall("oauth_begin_login")).not.toBeNull();
    expect(await app.lastCall("save_account")).toBeNull();
  });

  test("declining is remembered", async ({ boot }) => {
    const app = await boot({
      account: {
        gateway_base_url: "https://gw.example",
        has_api_key: true,
        auth_mode: "api_key",
      },
      localStorage: notOffered,
    });

    await app.page.getByRole("button", { name: "Keep using my API key" }).click();

    await expect(app.page.getByRole("dialog")).toHaveCount(0);
    expect(
      await app.page.evaluate(() => localStorage.getItem("gc.oauth-offer.v1.seen")),
    ).toBe("1");
  });

  test("an OAuth account is never offered it", async ({ boot }) => {
    const app = await boot({
      account: {
        gateway_base_url: "https://gw.example",
        has_api_key: false,
        auth_mode: "oauth",
        org_id: "org-1",
        org_name: "Constellation Labs",
      },
      oauth: { signed_in: true, email: "jdoe@acme.com", expires_at_unix: 4_000_000_000 },
      localStorage: notOffered,
    });

    await expect(app.page.getByRole("button", { name: "Settings" })).toBeVisible();
    await expect(app.page.getByRole("dialog")).toHaveCount(0);
  });
});
