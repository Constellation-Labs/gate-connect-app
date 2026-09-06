import { test, expect } from "./fixtures";

/**
 * The controls the window shell was drawn with and never wired: the engine's own
 * switch - now in the navigation rail, above the families it governs - the
 * shell-environment channel, the certificate, the app pane's switch,
 * the diagnostics probes, and the one-time OAuth offer.
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
  // The surface, not the product: the eyebrow over the row says "Anthropic".
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

  test("the master switch starts the engine", async ({ boot }) => {
    const app = await boot({ proxy: { running: false, ca_trusted: true } });

    const master = app.page.getByRole("switch", { name: "Route traffic through Gate" });
    await expect(master).toHaveAttribute("aria-checked", "false");

    await master.click();

    await expect.poll(() => app.lastCall("proxy_enable")).not.toBeNull();
    await expect(master).toHaveAttribute("aria-checked", "true");
  });

  test("and stops it, without asking about the certificate", async ({ boot }) => {
    // Disabling is promptless: the certificate stays trusted so re-enabling does
    // not raise the OS dialog again.
    const app = await boot({ proxy: { running: true, ca_trusted: false } });

    await app.page.getByRole("switch", { name: "Route traffic through Gate" }).click();

    await expect.poll(() => app.lastCall("proxy_disable")).not.toBeNull();
    expect(await app.lastCall("proxy_trust_ca")).toBeNull();
  });

  /**
   * AG-570 AC 8: a teardown that cannot put every tool back says which ones.
   *
   * Read back from the configs rather than assembled from what the sweep
   * believed it wrote - a sweep that returns success having written nothing is
   * the failure this report exists to catch.
   */
  test("routing off lists the tools it could not put back", async ({ boot }) => {
    const app = await boot({
      proxy: { running: true, ca_trusted: true },
      // Left connected by the sweep, which is what a best-effort teardown does
      // when one tool's write fails.
      tools: [{ ...CLAUDE_CODE, status: { kind: "connected" as const } }],
    });

    await app.page.getByRole("switch", { name: "Route traffic through Gate" }).click();

    const dialog = app.page.getByRole("dialog");
    await expect(dialog).toBeVisible();
    await expect(dialog.getByText("Some tools were left as they were")).toBeVisible();
    await expect(dialog.getByText("Still using Gate’s values")).toBeVisible();
    await expect(dialog.getByText("Claude Code")).toBeVisible();
    // The next action per tool, which the AC asks for by name.
    await expect(dialog.getByText("Retry disconnect")).toBeVisible();
  });

  test("a clean routing-off reports nothing, because there is nothing to report", async ({
    boot,
  }) => {
    const app = await boot({
      proxy: { running: true, ca_trusted: true },
      tools: [CLAUDE_CODE],
    });

    await app.page.getByRole("switch", { name: "Route traffic through Gate" }).click();

    await expect.poll(() => app.lastCall("proxy_disable")).not.toBeNull();
    await expect(app.page.getByRole("dialog")).toHaveCount(0);
  });

  /** A tool that is clean on disk but still running is its own bucket: it is
   *  not on its own settings yet, however the file reads. */
  test("routing off separates a tool waiting to be reopened", async ({ boot }) => {
    const app = await boot({
      proxy: { running: true, ca_trusted: true },
      tools: [CLAUDE_CODE],
      staleAgents: 1,
      runningAgents: 1,
      runningAgentNames: ["claude"],
    });

    await app.page.getByRole("switch", { name: "Route traffic through Gate" }).click();

    // The close-and-reopen offer comes first (the master toggle has always
    // raised it); dismissing it reveals the report behind.
    await app.page.getByRole("button", { name: /reopen later/i }).click();
    const dialog = app.page.getByRole("dialog");
    await expect(dialog.getByText("Waiting to be reopened")).toBeVisible();
    await expect(dialog.getByText("Reopen tool")).toBeVisible();
  });

  test("a chat domain starts the engine rather than routing nothing", async ({ boot }) => {
    // `proxy_set_domain` only records the flag, so with the engine off this used
    // to write intent and route nothing, with no control anywhere to start it.
    const app = await boot({ proxy: { running: false, ca_trusted: true } });

    // A chat surface, which is where this matters most: it has no config file to
    // write, so the engine is the only thing that could route it.
    await app.page.getByRole("switch", { name: "OpenAI App" }).click();

    await expect.poll(() => app.lastCall("proxy_set_domain")).toMatchObject({
      slug: "chatgpt",
      enabled: true,
    });
    const cmds = (await app.calls()).map((c) => c.cmd);
    expect(cmds.indexOf("proxy_enable")).toBeGreaterThan(-1);
    expect(cmds.indexOf("proxy_enable")).toBeLessThan(cmds.indexOf("proxy_set_domain"));
  });

  test("the shell-environment channel is its own choice", async ({ boot }) => {
    // It never starts or stops the engine: it decides whether the proxy is also
    // written into the user's environment, which reaches git and curl.
    const app = await boot({
      proxy: { running: true, ca_trusted: true, env_export_separable: true },
    });

    await app.page
      .getByRole("switch", { name: "Also set shell environment variables" })
      .click();

    await expect.poll(() => app.lastCall("proxy_set_env_export")).toEqual({ enabled: true });
    expect(await app.lastCall("proxy_disable")).toBeNull();
  });

  test("Linux is not offered a choice it cannot make", async ({ boot }) => {
    // There the variables *are* the system proxy, so the switch must not render.
    const app = await boot({
      proxy: { running: true, ca_trusted: true, env_export_separable: false },
    });

    await expect(
      app.page.getByRole("switch", { name: "Also set shell environment variables" }),
    ).toHaveCount(0);
  });
});

test.describe("new UI app pane", () => {
  test.beforeEach(async ({ page }) => {
    await page.addInitScript((k) => localStorage.setItem(k.gc, "1"), useNewUi);
  });

  test("the pane's own switch routes the app", async ({ boot }) => {
    const app = await boot({ proxy: { running: true, ca_trusted: true }, tools: [CLAUDE_CODE] });

    await app.page.getByRole("button", { name: "CLI" }).first().click();
    await app.page.getByRole("switch", { name: "Route CLI" }).click();

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

    await app.page.getByRole("button", { name: "CLI" }).first().click();

    // The drift alert card inside the pane carries its own switch for the same
    // app, reading off - that one is the re-adopt path. This is the header's.
    await expect(
      app.page.getByRole("switch", { name: "Route CLI" }),
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
    expect(await app.lastCall("proxy_untrust_ca")).toBeNull();

    await app.page.getByRole("button", { name: "Remove certificate" }).last().click();

    await expect.poll(() => app.lastCall("proxy_untrust_ca")).not.toBeNull();
  });

  test("an untrusted certificate is not offered for removal", async ({ boot }) => {
    const app = await boot({ proxy: { running: true, ca_trusted: false } });

    await app.page.getByRole("button", { name: "Settings" }).click();

    // Exact: the rail's master card says "the certificate is not trusted" in a
    // sentence on every screen now, and this assertion is about the Settings
    // row's own value.
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
