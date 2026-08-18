import { test, expect } from "./fixtures";

/**
 * The new window UI's routing actions, against the same fake backend the
 * popover suite uses.
 *
 * The rest of this suite is pinned to the popover (`VITE_NEW_UI=0` in
 * playwright.config.ts) because those tests assert on popover flows. This spec
 * opts back in per-test: `newUiEnabled()` reads localStorage before the
 * build-time default, so an init script is enough and nothing global changes.
 *
 * What this covers that `lib/useRouting.test.tsx` cannot: that the gate is
 * actually wired to the switch, that the dialog the design specifies is the one
 * that opens, and that approving it reaches the backend.
 */
const useNewUi = { gc: "gc.newUi" };

test.describe("new UI routing", () => {
  test.beforeEach(async ({ page }) => {
    await page.addInitScript((k) => localStorage.setItem(k.gc, "1"), useNewUi);
  });

  const driftedCodex = {
    proxy: { running: true, ca_trusted: true },
    tools: [
      {
        slug: "codex",
        name: "Codex",
        upstream_provider_name: "OpenAI",
        default_upstream_url: "https://gw.example/codex",
        requires_upstream_credential: false,
        status: { kind: "drifted" as const, reason: "API base URL: https://api.openai.com/v1" },
      },
    ],
  };

  test("a drifted app's sidebar switch reads on, and turning it off just disconnects", async ({
    boot,
  }) => {
    // Observed and intent are different things. Drift means the config changed
    // behind Gate, not that the user turned the app off, so the switch stays on
    // and the only thing the sidebar can do is turn it off - no review needed,
    // because disconnecting restores what was there.
    const app = await boot(driftedCodex);

    await expect(app.page.getByText("Config drifted")).toBeVisible();
    const sidebarSwitch = app.page.getByRole("switch", { name: "Codex" }).first();
    await expect(sidebarSwitch).toHaveAttribute("aria-checked", "true");

    await sidebarSwitch.click();

    await expect(app.page.getByRole("dialog")).toHaveCount(0);
    expect(await callsFor(app.page, "disconnect_tool")).toHaveLength(1);
    expect(await callsFor(app.page, "connect_tool")).toEqual([]);
  });

  test("re-adopting from the alert card goes through the review", async ({ boot }) => {
    const app = await boot(driftedCodex);

    // The card's switch reads off: the app is not protected. This is the path
    // that re-adopts, and the only one that reaches the review gate.
    await expect(app.page.getByText(/isn't protected/)).toBeVisible();
    const cardSwitch = app.page.getByRole("switch", { name: "Codex" }).last();
    await expect(cardSwitch).toHaveAttribute("aria-checked", "false");

    await cardSwitch.click();

    await expect(
      app.page.getByRole("heading", { name: "Review Codex configuration" }),
    ).toBeVisible();
    expect(await callsFor(app.page, "connect_tool")).toEqual([]);

    await app.page.getByRole("button", { name: "Replace config and protect" }).click();

    await expect(app.page.getByRole("dialog")).toHaveCount(0);
    expect(await callsFor(app.page, "connect_tool")).toHaveLength(1);
  });

  test("declining the review leaves the config alone", async ({ boot }) => {
    const app = await boot(driftedCodex);

    await app.page.getByRole("switch", { name: "Codex" }).last().click();
    await app.page.getByRole("button", { name: "Keep existing config" }).click();

    await expect(app.page.getByRole("dialog")).toHaveCount(0);
    expect(await callsFor(app.page, "connect_tool")).toEqual([]);
  });

  test("an untrusted certificate is asked about before anything is written", async ({
    boot,
  }) => {
    const app = await boot({
      proxy: { running: true, ca_trusted: false },
      tools: [
        {
          slug: "claude-code",
          name: "Claude Code",
          upstream_provider_name: "Anthropic",
          default_upstream_url: "https://gw.example/claude-code",
          requires_upstream_credential: false,
          status: { kind: "detected" },
        },
      ],
    });

    await app.page.getByRole("switch", { name: "Claude Code" }).click();

    await expect(
      app.page.getByRole("heading", { name: /Trust the Gate certificate/ }),
    ).toBeVisible();
    expect(await callsFor(app.page, "proxy_trust_ca")).toEqual([]);
    expect(await callsFor(app.page, "connect_tool")).toEqual([]);

    // Not now abandons the toggle rather than failing it: no write, no error.
    await app.page.getByRole("button", { name: "Not now" }).click();
    await expect(app.page.getByRole("dialog")).toHaveCount(0);
    await expect(app.page.getByRole("alert")).toHaveCount(0);
    expect(await callsFor(app.page, "connect_tool")).toEqual([]);
  });

  test("turning an app off needs no gate at all", async ({ boot }) => {
    const app = await boot({
      proxy: { running: true, ca_trusted: true },
      tools: [
        {
          slug: "claude-code",
          name: "Claude Code",
          upstream_provider_name: "Anthropic",
          default_upstream_url: "https://gw.example/claude-code",
          requires_upstream_credential: false,
          status: { kind: "connected" },
        },
      ],
    });

    await app.page.getByRole("switch", { name: "Claude Code" }).click();

    await expect(app.page.getByRole("dialog")).toHaveCount(0);
    expect(await callsFor(app.page, "disconnect_tool")).toHaveLength(1);
  });

  test("a failed write says why instead of failing silently", async ({ boot }) => {
    const app = await boot({
      proxy: { running: true, ca_trusted: true },
      failures: { connect_tool: "gateway rejected the key" },
      tools: [
        {
          slug: "claude-code",
          name: "Claude Code",
          upstream_provider_name: "Anthropic",
          default_upstream_url: "https://gw.example/claude-code",
          requires_upstream_credential: false,
          status: { kind: "detected" },
        },
      ],
    });

    await app.page.getByRole("switch", { name: "Claude Code" }).click();

    await expect(app.page.getByRole("alert")).toBeVisible();
  });
});

/** The fake backend records every command it is asked for. */
async function callsFor(page: import("@playwright/test").Page, cmd: string) {
  return page.evaluate(
    (c) =>
      (
        window as unknown as {
          __GATE_E2E__: { calls: { cmd: string }[] };
        }
      ).__GATE_E2E__.calls.filter((x) => x.cmd === c),
    cmd,
  );
}

/**
 * AG-568's failure branch, and the Gate route the review dialog now shows.
 *
 * What these cover that the unit tests cannot: that the failed write reaches the
 * row the user is looking at, rather than only the banner, and that the dialog
 * shows what it would write before asking for approval.
 */
test.describe("new UI drift repair", () => {
  test.beforeEach(async ({ page }) => {
    await page.addInitScript((k) => localStorage.setItem(k.gc, "1"), useNewUi);
  });

  const codex = {
    slug: "codex",
    name: "Codex",
    upstream_provider_name: "OpenAI",
    default_upstream_url: "https://gw.example/codex",
    requires_upstream_credential: false,
  };
  const drifted = {
    proxy: { running: true, ca_trusted: true },
    tools: [
      {
        ...codex,
        status: {
          kind: "drifted" as const,
          reason: "API base URL: https://api.openai.com/v1",
        },
      },
    ],
  };

  /**
   * The alert card's switch is the one path that re-adopts a drifted config and
   * so the only one that reaches the review (the sidebar switch reads *on* for a
   * drifted tool and turns it off). This is where showing the Gate route matters:
   * the user is being asked to approve an overwrite.
   */
  test("the review dialog shows what Gate would write, not just what it found", async ({
    boot,
  }) => {
    const app = await boot(drifted);

    await app.page.getByRole("switch", { name: "Codex" }).last().click();

    const dialog = app.page.getByRole("dialog");
    await expect(dialog).toBeVisible();
    // What Gate found...
    await expect(dialog.getByText("API base URL: https://api.openai.com/v1")).toBeVisible();
    // ...and what it would write in its place.
    await expect(dialog.getByText("What Gate would write instead")).toBeVisible();
    await expect(dialog.getByText("http://127.0.0.1:45981")).toBeVisible();
  });

  test("the Gate-route row is omitted when no relay port has been bound", async ({ boot }) => {
    // Not "unknown" dressed as an address: with no port there is nothing true to
    // show, so the row goes rather than guessing.
    const app = await boot({ ...drifted, proxy: { running: true, ca_trusted: true, relay_base_url: null } });

    await app.page.getByRole("switch", { name: "Codex" }).last().click();

    const dialog = app.page.getByRole("dialog");
    await expect(dialog).toBeVisible();
    await expect(dialog.getByText("What Gate would write instead")).toHaveCount(0);
  });

  test("a failed write says so on the row, not only in a banner", async ({ boot }) => {
    const app = await boot({
      proxy: { running: true, ca_trusted: true },
      tools: [{ ...codex, status: { kind: "detected" as const } }],
      failures: { connect_tool: "failed to write ~/.codex/config.toml" },
    });

    const sidebarSwitch = app.page.getByRole("switch", { name: "Codex" }).first();
    await sidebarSwitch.click();

    await expect(app.page.getByText("Configuration update failed")).toBeVisible();
  });

  test("a retry that succeeds clears the failure from the row", async ({ boot }) => {
    const app = await boot({
      proxy: { running: true, ca_trusted: true },
      tools: [{ ...codex, status: { kind: "detected" as const } }],
      failures: { connect_tool: "failed to write ~/.codex/config.toml" },
    });

    const sidebarSwitch = app.page.getByRole("switch", { name: "Codex" }).first();
    await sidebarSwitch.click();
    await expect(app.page.getByText("Configuration update failed")).toBeVisible();

    // Clear the injected failure, then click again - the switch is the retry.
    // `app.patch` merges objects one level deep, so it cannot *remove* a key;
    // this reaches for the harness state directly to empty the map.
    await app.page.evaluate(() => {
      window.__GATE_E2E__.state.failures = {};
    });
    await sidebarSwitch.click();

    await expect(app.page.getByText("Configuration update failed")).toHaveCount(0);
  });
});

/**
 * AG-558's one buildable line: "Gate Connect checks for each supported tool
 * during setup, MANUAL REFRESH, and application changes that can affect
 * detection."
 *
 * Detection only ran on backend events, so installing a tool while this window
 * was open showed nothing until something unrelated repainted it.
 */
test.describe("new UI: refreshing the inventory", () => {
  test.beforeEach(async ({ page }) => {
    await page.addInitScript((k) => localStorage.setItem(k.gc, "1"), useNewUi);
  });

  test("the refresh control re-reads tools and re-runs the routing sweep", async ({ boot }) => {
    const app = await boot({ proxy: { running: true, ca_trusted: true } });

    const before = (await app.calls()).filter((c) => c.cmd === "list_tools").length;
    await app.page.getByRole("button", { name: "Refresh apps" }).click();

    await expect
      .poll(async () => (await app.calls()).filter((c) => c.cmd === "list_tools").length)
      .toBeGreaterThan(before);
    // The sweep rides along: a tool that just appeared has no verdict yet, and
    // leaving the old ones on screen would describe a different set of tools.
    await expect
      .poll(async () => (await app.calls()).filter((c) => c.cmd === "routing_verdicts").length)
      .toBeGreaterThan(1);
  });

  test("a tool installed while the window was open appears on refresh", async ({ boot }) => {
    const app = await boot({ proxy: { running: true, ca_trusted: true }, tools: [] });

    await expect(app.page.getByRole("switch", { name: "Codex" })).toHaveCount(0);

    await app.patch({
      tools: [
        {
          slug: "codex",
          name: "Codex",
          upstream_provider_name: "OpenAI",
          default_upstream_url: "https://gw.example/codex",
          requires_upstream_credential: false,
          status: { kind: "detected" },
        },
      ],
    });
    // Starting from an empty list, so the control on screen is the inventory
    // card's Refresh - the eyebrow one is hidden while that card shows.
    await app.page.getByRole("button", { name: "Refresh", exact: true }).click();

    await expect(app.page.getByRole("switch", { name: "Codex" }).first()).toBeVisible();
    await expect(app.page.getByText("No apps detected")).toHaveCount(0);
  });
});

/**
 * AG-560's first two criteria: a completed scan that found nothing and a scan
 * that could not complete are different results, and must not look alike.
 *
 * The bug this closes: `listTools().catch(() => [])` turned a failed read into an
 * empty array, so a device Gate could not scan rendered as a device with no AI
 * apps on it - with a "0/0" count that reads like a clean answer.
 */
test.describe("new UI: an empty inventory", () => {
  test.beforeEach(async ({ page }) => {
    await page.addInitScript((k) => localStorage.setItem(k.gc, "1"), useNewUi);
  });

  test("a completed scan with nothing on the device says so, with a time", async ({ boot }) => {
    const app = await boot({ proxy: { running: true, ca_trusted: true }, tools: [] });

    await expect(app.page.getByText("No apps detected")).toBeVisible();
    await expect(app.page.getByText(/^Checked /)).toBeVisible();
    // The card's own control. The eyebrow one is hidden while this card shows,
    // so there is exactly one Refresh on screen.
    await expect(app.page.getByRole("button", { name: "Refresh", exact: true })).toBeVisible();
    await expect(app.page.getByRole("button", { name: "Refresh apps" })).toHaveCount(0);
  });

  test("a failed scan says it could not look, not that there is nothing", async ({ boot }) => {
    const app = await boot({
      proxy: { running: true, ca_trusted: true },
      tools: [],
      failures: { list_tools: "permission denied reading the app list" },
    });

    await expect(app.page.getByText("Couldn’t check for apps")).toBeVisible();
    // The distinction that matters: it must not claim the device is clean.
    await expect(app.page.getByText("No apps detected")).toHaveCount(0);
    await expect(app.page.getByRole("button", { name: "Try again" })).toBeVisible();
  });

  test("neither state appears once apps are found", async ({ boot }) => {
    const app = await boot({ proxy: { running: true, ca_trusted: true } });

    await expect(app.page.getByText("No apps detected")).toHaveCount(0);
    await expect(app.page.getByText("Couldn’t check for apps")).toHaveCount(0);
  });

  test("a failed scan recovers when the retry succeeds", async ({ boot }) => {
    const app = await boot({
      proxy: { running: true, ca_trusted: true },
      tools: [],
      failures: { list_tools: "permission denied reading the app list" },
    });
    await expect(app.page.getByText("Couldn’t check for apps")).toBeVisible();

    await app.page.evaluate(() => {
      window.__GATE_E2E__.state.failures = {};
    });
    await app.page.getByRole("button", { name: "Try again" }).click();

    // Now a real answer: the device genuinely has no tools in this fixture.
    await expect(app.page.getByText("No apps detected")).toBeVisible();
    await expect(app.page.getByText("Couldn’t check for apps")).toHaveCount(0);
  });
});

/**
 * The window shell had no backend-error drain at all, so a failure that happened
 * before this webview existed - the startup auto-enable runs before either shell
 * mounts - went to telemetry and nowhere else.
 *
 * `report_backend_error("provider_restore", ...)` fires on both restore passes in
 * `proxy_enable`, which is AG-570's central scenario: routing did not fully come
 * back, and the window said nothing.
 */
test.describe("new UI: buffered backend failures", () => {
  test.beforeEach(async ({ page }) => {
    await page.addInitScript((k) => localStorage.setItem(k.gc, "1"), useNewUi);
  });

  test("a failed restore that predates the window is shown, not just logged", async ({
    boot,
  }) => {
    const app = await boot({
      proxy: { running: true, ca_trusted: true },
      backendErrors: [
        { context: "provider_restore", message: "failed to restore provider openai" },
      ],
    });

    await expect.poll(() => app.lastCall("drain_backend_errors")).not.toBeNull();
    // The banner, not the console: this is the one error class the user cannot
    // discover any other way.
    await expect(app.page.getByRole("button", { name: "Dismiss" })).toBeVisible();
  });

  test("it drains again on the nudge, so a later failure is not stranded", async ({ boot }) => {
    const app = await boot({ proxy: { running: true, ca_trusted: true } });
    const before = (await app.calls()).filter((c) => c.cmd === "drain_backend_errors").length;

    await app.patch({
      backendErrors: [
        { context: "provider_restore", message: "failed to restore provider openai" },
      ],
    });
    await app.emit("backend-error-pending");

    await expect
      .poll(async () => (await app.calls()).filter((c) => c.cmd === "drain_backend_errors").length)
      .toBeGreaterThan(before);
    await expect(app.page.getByRole("button", { name: "Dismiss" })).toBeVisible();
  });

  test("a failure that does not mean routing is down stays out of the user's way", async ({
    boot,
  }) => {
    // Drained and sent to analytics, but not interrupting: only the routing-down
    // contexts earn a banner.
    const app = await boot({
      proxy: { running: true, ca_trusted: true },
      backendErrors: [{ context: "account_reconcile", message: "keychain busy" }],
    });

    await expect.poll(() => app.lastCall("drain_backend_errors")).not.toBeNull();
    await expect(app.page.getByRole("button", { name: "Dismiss" })).toHaveCount(0);
  });
});
