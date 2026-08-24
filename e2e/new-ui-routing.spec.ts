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
    // that re-adopts, and the only one that reaches the review gate. Its
    // accessible name is the notice's own ("Let Gate Connect manage Codex"),
    // which is what distinguishes it from the sidebar row's switch.
    await expect(app.page.getByText("Reconnect to restore protection")).toBeVisible();
    const cardSwitch = app.page.getByRole("switch", { name: "Let Gate Connect manage Codex" });
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

    await app.page.getByRole("switch", { name: "Let Gate Connect manage Codex" }).click();
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

  test("the certificate gate names the system dialog Windows is about to raise", async ({
    boot,
  }) => {
    // AG-534. `proxy_trust_ca` shells out to `certutil -user -addstore Root`,
    // which raises a red "Security Warning" quoting the CA's name - unexplained,
    // that reads as malware rather than as the step the user just asked for.
    const app = await boot({
      platform: "windows",
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
      app.page.getByText("Windows will show a security warning: that’s expected, choose Yes."),
    ).toBeVisible();
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
 * was open showed nothing until something unrelated repainted it. It was a manual
 * control first; now the window polls, and the control is gone from the eyebrow.
 */
test.describe("new UI: refreshing the inventory", () => {
  test.beforeEach(async ({ page }) => {
    await page.addInitScript((k) => localStorage.setItem(k.gc, "1"), useNewUi);
  });

  const countOf = async (app: { calls: () => Promise<{ cmd: string }[]> }, cmd: string) =>
    (await app.calls()).filter((c) => c.cmd === cmd).length;

  test("the tool list is re-read with nothing asking it to be", async ({ boot }) => {
    const app = await boot({ proxy: { running: true, ca_trusted: true } });

    const before = await countOf(app, "list_tools");
    // No click anywhere: there is no control for this any more. The timeouts in
    // this describe block are all several polling periods, not a guess.
    await expect
      .poll(() => countOf(app, "list_tools"), { timeout: 20_000 })
      .toBeGreaterThan(before);
  });

  test("a poll that finds nothing new does not re-run the routing sweep", async ({ boot }) => {
    const app = await boot({ proxy: { running: true, ca_trusted: true } });
    await expect.poll(() => countOf(app, "routing_verdicts")).toBeGreaterThan(0);

    // The sweep probes the relay and the gateway, so it is the one reading that
    // must not ride a timer. An unchanged machine costs the two local reads only.
    const sweeps = await countOf(app, "routing_verdicts");
    const polls = await countOf(app, "list_tools");
    await expect
      .poll(() => countOf(app, "list_tools"), { timeout: 30_000 })
      .toBeGreaterThan(polls + 1);
    expect(await countOf(app, "routing_verdicts")).toBe(sweeps);
  });

  test("a tool installed while the window was open appears on its own", async ({ boot }) => {
    const app = await boot({ proxy: { running: true, ca_trusted: true }, tools: [] });

    // `exact`: the rail's ChatGPT (Codex subscription) row is drawn from the
    // catalog whether or not any tool is installed, and its switch's name
    // would otherwise substring-match "Codex".
    await expect(app.page.getByRole("switch", { name: "Codex", exact: true })).toHaveCount(0);
    const sweeps = await countOf(app, "routing_verdicts");

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

    await expect(
      app.page.getByRole("switch", { name: "Codex", exact: true }).first(),
    ).toBeVisible({
      timeout: 20_000,
    });
    await expect(app.page.getByText("No apps detected")).toHaveCount(0);
    // The sweep rides a change: a tool that just appeared has no verdict yet, and
    // its row would sit on "Checking" until something unrelated repainted it.
    await expect
      .poll(() => countOf(app, "routing_verdicts"), { timeout: 20_000 })
      .toBeGreaterThan(sweeps);
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
    // The card keeps a control of its own - a scan may have failed and be worth
    // retrying against rather than waiting out. It is the only one on screen: the
    // eyebrow's was removed when detection started polling.
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
    // `exact`, for the same reason the Refresh assertion above uses it: the
    // Overview's activity notices offer their own retry, named "Try again:
    // <section>" so a screen reader can tell three identically-worded buttons
    // apart. Substring matching would find those too.
    await expect(app.page.getByRole("button", { name: "Try again", exact: true })).toBeVisible();
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
    await app.page.getByRole("button", { name: "Try again", exact: true }).click();

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

/**
 * AG-570's "Review details": what the restore did, entry by entry, read-only.
 *
 * The criterion is explicit that reviewing "does not change state", so the only
 * action closes it - and the journal holds slugs, display names, outcomes and
 * timestamps, with no credentials or request content, which is what makes showing
 * it in full safe.
 */
test.describe("new UI: reviewing an interrupted restore", () => {
  test.beforeEach(async ({ page }) => {
    await page.addInitScript((k) => localStorage.setItem(k.gc, "1"), useNewUi);
  });

  const withJournal = {
    proxy: { running: true, ca_trusted: true },
    pendingRestore: {
      providers: [] as { slug: string; name: string }[],
      tools: [{ slug: "opencode", name: "OpenCode" }],
    },
    restoreJournal: {
      updated_unix: 1_760_000_000,
      requested_routing_on: true,
      entries: [
        {
          slug: "codex",
          name: "Codex",
          kind: "tool" as const,
          outcome: "restored" as const,
          at_unix: 1_760_000_000,
        },
        {
          slug: "opencode",
          name: "OpenCode",
          kind: "tool" as const,
          outcome: "write_failed" as const,
          at_unix: 1_760_000_001,
        },
        {
          slug: "hermes",
          name: "Hermes",
          kind: "tool" as const,
          outcome: "pending" as const,
          at_unix: 1_760_000_002,
        },
      ],
    },
  };

  test("it accounts for every entry, including the ones never reached", async ({ boot }) => {
    const app = await boot(withJournal);

    await app.page.getByRole("button", { name: "Review details" }).click();

    const dialog = app.page.getByRole("dialog");
    await expect(dialog).toBeVisible();
    await expect(dialog.getByText("Codex")).toBeVisible();
    await expect(dialog.getByText("Done")).toBeVisible();
    await expect(dialog.getByText("Failed")).toBeVisible();
    // The interruption is the case this exists for: an entry never attempted must
    // read as not reached, not as fine and not as failed.
    await expect(dialog.getByText("Not reached")).toBeVisible();
  });

  test("reviewing changes nothing", async ({ boot }) => {
    const app = await boot(withJournal);

    await app.page.getByRole("button", { name: "Review details" }).click();
    await app.page.getByRole("button", { name: "Close" }).click();

    expect(await app.lastCall("resume_restore")).toBeNull();
    expect(await app.lastCall("connect_tool")).toBeNull();
    // Still outstanding, so the notice is still there.
    await expect(app.page.getByText("Routing didn’t finish coming back")).toBeVisible();
  });

  test("no journal, no Review details button", async ({ boot }) => {
    // An interruption before the journal was written leaves the snapshots but no
    // explanation. A button onto an empty dialog is worse than no button.
    const app = await boot({ ...withJournal, restoreJournal: null });

    await expect(app.page.getByText("Routing didn’t finish coming back")).toBeVisible();
    await expect(app.page.getByRole("button", { name: "Review details" })).toHaveCount(0);
  });
});

/**
 * AG-564's one unambiguous line: "The warning names the tool and configuration
 * location without displaying credentials or secret values."
 *
 * The location is the file Gate is about to rewrite. Showing it is also the
 * transparency the product trades on - the user can go and read it, which beats
 * any sentence about what Gate does and does not touch.
 */
test.describe("new UI: the review names the file it will change", () => {
  test.beforeEach(async ({ page }) => {
    await page.addInitScript((k) => localStorage.setItem(k.gc, "1"), useNewUi);
  });

  const driftedWithPath = {
    proxy: { running: true, ca_trusted: true },
    tools: [
      {
        slug: "codex",
        name: "Codex",
        upstream_provider_name: "OpenAI",
        default_upstream_url: "https://gw.example/codex",
        requires_upstream_credential: false,
        config_location: "/Users/someone/.codex/config.toml",
        status: {
          kind: "drifted" as const,
          reason: "API base URL: https://api.openai.com/v1",
        },
      },
    ],
  };

  test("the review names the config file", async ({ boot }) => {
    const app = await boot(driftedWithPath);

    await app.page.getByRole("switch", { name: "Codex" }).last().click();

    const dialog = app.page.getByRole("dialog");
    await expect(dialog).toBeVisible();
    await expect(dialog.getByText("/Users/someone/.codex/config.toml")).toBeVisible();
  });

  test("no file is named when the tool owns none", async ({ boot }) => {
    // The environment channel writes machine-wide settings, not a file of its
    // own, so the line goes rather than naming something invented.
    const app = await boot({
      ...driftedWithPath,
      tools: [{ ...driftedWithPath.tools[0], config_location: null }],
    });

    await app.page.getByRole("switch", { name: "Codex" }).last().click();

    const dialog = app.page.getByRole("dialog");
    await expect(dialog).toBeVisible();
    await expect(dialog.getByText("The file that changes:")).toHaveCount(0);
  });
});

/**
 * The rail as `Components / Sidenav` draws it (read 2026-08-23): proxy-routed
 * members are rows beside the config tools, every eyebrow carries its
 * protected-over-total counter, and the multi-provider tools share one
 * "Other tools" group. Hook tests cannot see whether a row's switch reaches
 * the right command, which is what the first of these pins.
 */
test.describe("new UI sidebar rail", () => {
  test.beforeEach(async ({ page }) => {
    await page.addInitScript((k) => localStorage.setItem(k.gc, "1"), useNewUi);
  });

  test("a proxy domain is a rail row whose switch routes it", async ({ boot }) => {
    const app = await boot({ proxy: { running: true, ca_trusted: true } });

    await app.page
      .getByRole("switch", { name: "ChatGPT (Codex subscription)" })
      .first()
      .click();

    // A domain routes through the engine's flag, never a config write.
    await expect.poll(() => app.lastCall("proxy_set_domain")).toMatchObject({
      slug: "chatgpt",
      enabled: true,
    });
    expect(await callsFor(app.page, "connect_tool")).toEqual([]);
  });

  test("a domain row opens a pane that says its activity can't be attributed", async ({
    boot,
  }) => {
    const app = await boot({ proxy: { running: true, ca_trusted: true } });

    await app.page.getByRole("button", { name: "ChatGPT (Codex subscription)" }).click();

    await expect(
      app.page.getByRole("heading", { name: "ChatGPT (Codex subscription)" }),
    ).toBeVisible();
    // The gateway attributes requests to config tools only, so the pane says
    // why its sections are empty rather than reporting a quiet day.
    await expect(app.page.getByText(/aren't attributed to a single app/)).toBeVisible();

    // The pane's own switch routes the domain, same as the rail row's.
    await app.page
      .getByRole("switch", { name: "Route ChatGPT (Codex subscription)" })
      .click();
    await expect.poll(() => app.lastCall("proxy_set_domain")).toMatchObject({
      slug: "chatgpt",
      enabled: true,
    });
    expect(await callsFor(app.page, "connect_tool")).toEqual([]);
  });

  test("a group's eyebrow counts protected rows over rows", async ({ boot }) => {
    const app = await boot({ proxy: { running: true, ca_trusted: true } });

    // OPEN AI holds Codex (detected, off), OpenAI apps and ChatGPT (both off).
    // `exact`, because the routing banner's "0 of 3 Apps" contains the phrase.
    await expect(app.page.getByText("0 of 3", { exact: true })).toBeVisible();

    // Routing the OpenAI apps domain with the engine up and the certificate
    // trusted makes it the group's one protected row.
    await app.page.getByRole("switch", { name: "OpenAI apps" }).click();
    await expect(app.page.getByText("1 of 3", { exact: true })).toBeVisible();
  });

  test("the multi-provider tools share one Other tools eyebrow", async ({ boot }) => {
    const app = await boot({ proxy: { running: true, ca_trusted: true } });

    await expect(app.page.getByRole("heading", { name: "Other tools" })).toBeVisible();
    // Not one eyebrow per tool - the 2026-08-21 read drew that, and the
    // Sidenav page reversed it.
    await expect(app.page.getByRole("heading", { name: "OpenCode" })).toHaveCount(0);
  });
});
