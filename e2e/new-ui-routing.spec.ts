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

/**
 * AG-570: an interrupted restore, surfaced and resumable.
 *
 * The provider snapshots have always recorded unfinished work - `restore_all`
 * keeps failures in the file and clears it only once everything is back - but
 * nothing read them for display. A half-finished restore therefore left some tools
 * routing and some not, with no statement anywhere that Gate knew about it.
 */
test.describe("new UI: an interrupted restore", () => {
  test.beforeEach(async ({ page }) => {
    await page.addInitScript((k) => localStorage.setItem(k.gc, "1"), useNewUi);
  });

  const interrupted = {
    proxy: { running: true, ca_trusted: true },
    pendingRestore: {
      providers: [{ slug: "openai", name: "OpenAI" }],
      tools: [{ slug: "opencode", name: "OpenCode" }],
    },
  };

  test("what did not finish is named", async ({ boot }) => {
    const app = await boot(interrupted);

    await expect(app.page.getByText("Routing didn’t finish coming back")).toBeVisible();
    // Providers and tools together: the user does not care which file an entry
    // came from.
    await expect(app.page.getByText(/OpenAI, OpenCode/)).toBeVisible();
  });

  test("nothing outstanding shows no notice", async ({ boot }) => {
    const app = await boot({ proxy: { running: true, ca_trusted: true } });

    await expect(app.page.getByText("Routing didn’t finish coming back")).toHaveCount(0);
  });

  test("Resume finishes the job and the notice goes", async ({ boot }) => {
    const app = await boot(interrupted);

    await app.page.getByRole("button", { name: "Resume now" }).click();

    await expect.poll(() => app.lastCall("resume_restore")).not.toBeNull();
    await expect(app.page.getByText("Routing didn’t finish coming back")).toHaveCount(0);
  });

  /**
   * The case that must not read as done: resuming fixed one entry and not the
   * other, so the notice stays and names only what is left.
   */
  test("a partial resume keeps the notice, naming only what is left", async ({ boot }) => {
    const app = await boot({ ...interrupted, pendingResumeKeeps: ["opencode"] });

    await app.page.getByRole("button", { name: "Resume now" }).click();

    // Scoped to the banner: the sidebar lists these apps by name too.
    const banner = app.page.getByRole("status");
    await expect(banner.getByText("Routing didn’t finish coming back")).toBeVisible();
    await expect(banner.getByText(/OpenCode is still waiting/)).toBeVisible();
    await expect(banner.getByText(/OpenAI/)).toHaveCount(0);
  });

  test("Finish later hides it for this session without resuming anything", async ({ boot }) => {
    const app = await boot(interrupted);

    await app.page.getByRole("button", { name: "Finish later" }).click();

    await expect(app.page.getByText("Routing didn’t finish coming back")).toHaveCount(0);
    expect(await app.lastCall("resume_restore")).toBeNull();
    // Still recorded on disk, which is what makes the notice come back later.
    expect((await app.state()).pendingRestore.providers).toHaveLength(1);
  });

  test("a live failure outranks a recorded one", async ({ boot }) => {
    const app = await boot({
      ...interrupted,
      backendErrors: [
        { context: "provider_restore", message: "failed to restore provider openai" },
      ],
    });

    // The error banner, not the recovery notice: something just went wrong.
    await expect(app.page.getByRole("button", { name: "Dismiss error" })).toBeVisible();
    await expect(app.page.getByText("Routing didn’t finish coming back")).toHaveCount(0);
  });
});
