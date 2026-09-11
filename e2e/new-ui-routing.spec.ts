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
        name: "CLI",
        upstream_provider_name: "OpenAI",
        default_upstream_url: "https://gw.example/codex",
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
    const sidebarSwitch = app.appSwitch("ChatGPT / Codex");
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
    // accessible name is the notice's own ("Let Gate Connect manage CLI"),
    // which is what distinguishes it from the sidebar row's switch.
    await expect(app.page.getByText("Reconnect to restore protection")).toBeVisible();
    const cardSwitch = app.page.getByRole("switch", { name: "Let Gate Connect manage CLI" });
    await expect(cardSwitch).toHaveAttribute("aria-checked", "false");

    await cardSwitch.click();

    await expect(
      app.page.getByRole("heading", { name: "Review CLI configuration" }),
    ).toBeVisible();
    expect(await callsFor(app.page, "connect_tool")).toEqual([]);

    await app.page.getByRole("button", { name: "Replace config and protect" }).click();

    await expect(app.page.getByRole("dialog")).toHaveCount(0);
    expect(await callsFor(app.page, "connect_tool")).toHaveLength(1);
  });

  test("declining the review leaves the config alone", async ({ boot }) => {
    const app = await boot(driftedCodex);

    await app.page.getByRole("switch", { name: "Let Gate Connect manage CLI" }).click();
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
          name: "CLI",
          upstream_provider_name: "Anthropic",
          default_upstream_url: "https://gw.example/claude-code",
          status: { kind: "detected" },
        },
      ],
    });

    await app.routeApp("Claude");

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
          name: "CLI",
          upstream_provider_name: "Anthropic",
          default_upstream_url: "https://gw.example/claude-code",
          status: { kind: "detected" },
        },
      ],
    });

    await app.routeApp("Claude");

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
          name: "CLI",
          upstream_provider_name: "Anthropic",
          default_upstream_url: "https://gw.example/claude-code",
          status: { kind: "connected" },
        },
      ],
    });

    await app.routeApp("Claude");

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
          name: "CLI",
          upstream_provider_name: "Anthropic",
          default_upstream_url: "https://gw.example/claude-code",
          status: { kind: "detected" },
        },
      ],
    });

    await app.routeApp("Claude");

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
    name: "CLI",
    upstream_provider_name: "OpenAI",
    default_upstream_url: "https://gw.example/codex",
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

    await app.page.getByRole("switch", { name: "Let Gate Connect manage CLI" }).click();

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

    await app.page.getByRole("switch", { name: "Let Gate Connect manage CLI" }).click();

    const dialog = app.page.getByRole("dialog");
    await expect(dialog).toBeVisible();
    await expect(dialog.getByText("What Gate would write instead")).toHaveCount(0);
  });

  test("a failed write says so in the pane header, not only in a banner", async ({ boot }) => {
    const app = await boot({
      proxy: { running: true, ca_trusted: true },
      tools: [{ ...codex, status: { kind: "detected" as const } }],
      failures: { connect_tool: "failed to write ~/.codex/config.toml" },
    });

    const sidebarSwitch = app.appSwitch("ChatGPT / Codex");
    await sidebarSwitch.click();

    // The rail row keeps the phrase and drops the reason, which does not fit
    // 250px. The pane header is the surface with room for the sentence, and it
    // outlives the banner - which is the half of this that still matters.
    await app.page.getByRole("button", { name: "CLI" }).first().click();
    await expect(app.page.getByText("Configuration update failed")).toBeVisible();
  });

  test("a retry that succeeds clears the failure from the pane header", async ({ boot }) => {
    const app = await boot({
      proxy: { running: true, ca_trusted: true },
      tools: [{ ...codex, status: { kind: "detected" as const } }],
      failures: { connect_tool: "failed to write ~/.codex/config.toml" },
    });

    const sidebarSwitch = app.appSwitch("ChatGPT / Codex");
    await sidebarSwitch.click();

    // Opened before the retry, not after: with the pane closed the reason is
    // nowhere on the page and the count below would pass without the retry ever
    // having cleared anything.
    await app.page.getByRole("button", { name: "CLI" }).first().click();
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
 * Detection used to run on backend events that had nothing to do with tools, so
 * installing one while this window was open showed nothing until something
 * unrelated repainted it. It was a manual control first, then a 5s poll, and now
 * an event of its own: the backend watches the tool config files and binaries
 * (`core/src/tool_watch.rs`) and emits `tools-changed`. The eyebrow's control
 * stayed gone.
 */
test.describe("new UI: refreshing the inventory", () => {
  test.beforeEach(async ({ page }) => {
    await page.addInitScript((k) => localStorage.setItem(k.gc, "1"), useNewUi);
  });

  const countOf = async (app: { calls: () => Promise<{ cmd: string }[]> }, cmd: string) =>
    (await app.calls()).filter((c) => c.cmd === cmd).length;

  test("the tool list is re-read when the backend says it changed", async ({ boot }) => {
    const app = await boot({ proxy: { running: true, ca_trusted: true } });
    await expect.poll(() => countOf(app, "list_tools")).toBeGreaterThan(0);

    const before = await countOf(app, "list_tools");
    // Nothing on a timer any more, so the count holds still until something
    // says otherwise. Asserted, because "it will re-read eventually" is exactly
    // what this stopped doing.
    await new Promise((r) => setTimeout(r, 1_000));
    expect(await countOf(app, "list_tools")).toBe(before);

    // No click anywhere: there is no control for this in the eyebrow.
    await app.emit("tools-changed");

    await expect.poll(() => countOf(app, "list_tools")).toBeGreaterThan(before);
  });

  test("an event that finds nothing new does not re-run the routing sweep", async ({
    boot,
  }) => {
    const app = await boot({ proxy: { running: true, ca_trusted: true } });
    await expect.poll(() => countOf(app, "routing_verdicts")).toBeGreaterThan(0);

    // The sweep probes the relay and the gateway, so it is the one reading a
    // filesystem event must not be able to trigger - a package manager writing
    // in a watched directory would otherwise aim a burst of probes at the
    // gateway. An unchanged machine costs the two local reads only.
    const sweeps = await countOf(app, "routing_verdicts");
    const reads = await countOf(app, "list_tools");
    await app.emit("tools-changed");
    await app.emit("tools-changed");

    await expect.poll(() => countOf(app, "list_tools")).toBeGreaterThan(reads + 1);
    expect(await countOf(app, "routing_verdicts")).toBe(sweeps);
  });

  test("a tool installed while the window was open appears when the watch fires", async ({
    boot,
  }) => {
    const app = await boot({ proxy: { running: true, ca_trusted: true }, tools: [] });

    // OpenCode rather than Codex, because a section exists as soon as anything
    // in it does: ChatGPT / Codex draws from its two chatgpt.com domains with no
    // tool installed at all, so it could never test "appears when a tool does".
    // Nothing in the default catalog belongs to the OpenCode section.
    const row = app.appSwitch("OpenCode");
    await expect(row).toHaveCount(0);
    const sweeps = await countOf(app, "routing_verdicts");

    // Installed behind the window's back, exactly as a terminal would.
    await app.patch({
      tools: [
        {
          slug: "opencode",
          name: "OpenCode",
          upstream_provider_name: "your existing providers",
          default_upstream_url: "https://gw.example/opencode",
          status: { kind: "detected" },
        },
      ],
    });
    // Nothing until the backend says so, which is the whole difference from the
    // poll this replaced.
    await expect(row).toHaveCount(0);

    await app.emit("tools-changed");

    await expect(row).toBeVisible();
    await expect(app.page.getByText("No apps detected")).toHaveCount(0);
    // The sweep rides a change: a tool that just appeared has no verdict yet, and
    // its row would sit on "Checking" until something unrelated repainted it.
    await expect.poll(() => countOf(app, "routing_verdicts")).toBeGreaterThan(sweeps);
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

  /**
   * Resume works through the entries one at a time.
   *
   * Not `resume_restore`: AG-570 asks a resume to show progress for each tool,
   * and a single batch call can only report what is left once the whole thing is
   * over. `restore_one`'s semantics are the batch's narrowed to one slug, so
   * this still repeats no completed write.
   */
  test("Resume finishes the job entry by entry and the notice goes", async ({ boot }) => {
    const app = await boot(interrupted);

    await app.page.getByRole("button", { name: "Resume now" }).click();

    await expect
      .poll(async () => (await app.state()).retryCalls)
      .toEqual(["openai", "opencode"]);
    await expect(app.page.getByText("Routing didn’t finish coming back")).toHaveCount(0);
  });

  /** The per-tool rows AG-570 asks the summary for: every tool the operation
   *  touched, with its stage, its last check and what it still needs. */
  test("the notice accounts for each tool it is waiting on", async ({ boot }) => {
    const app = await boot(interrupted);
    const banner = app.page.getByRole("status");

    await banner.getByRole("button", { name: /Show tools/ }).click();

    // The row's own name cell, not the summary sentence above it that also
    // lists the tools it is waiting on.
    await expect(
      banner.getByRole("listitem").filter({ hasText: "OpenCode" }),
    ).toBeVisible();
    // Seeded, never attempted: the interruption's own signature.
    await expect(banner.getByText("Not started").first()).toBeVisible();
    // The three readings a stage cannot carry, on the row.
    await expect(banner.getByText(/Last verified:/).first()).toBeVisible();
  });

  /** One row's Retry, which is the AC's "repeats only the failed or unverified
   *  stage for the selected tool". The other entry is left alone. */
  test("a row's Retry asks about that entry only", async ({ boot }) => {
    const app = await boot(interrupted);
    const banner = app.page.getByRole("status");
    await banner.getByRole("button", { name: /Show tools/ }).click();

    // The row for OpenCode, not the whole-notice Resume.
    await banner
      .getByRole("listitem")
      .filter({ hasText: "OpenCode" })
      .getByRole("button", { name: "Retry" })
      .click();

    await expect.poll(async () => (await app.state()).retryCalls).toEqual(["opencode"]);
  });

  /** A resume that came back offers the close-and-reopen conversation, scoped to
   *  what it actually rewrote. */
  test("a completed resume offers to close the tools it just rewrote", async ({ boot }) => {
    const app = await boot({
      ...interrupted,
      runningAgents: 1,
      staleAgents: 1,
      runningAgentNames: ["opencode"],
    });

    await app.page.getByRole("button", { name: "Resume now" }).click();

    await expect(app.page.getByRole("dialog")).toBeVisible();
    await expect.poll(() => app.lastCall("running_agents")).toMatchObject({
      only: ["openai", "opencode"],
    });
  });

  /**
   * The case that must not read as done: resuming fixed one entry and not the
   * other, so the notice stays and names only what is left.
   */
  test("a partial resume keeps the notice, naming only what is left", async ({ boot }) => {
    const app = await boot({
      ...interrupted,
      pendingResumeKeeps: ["opencode"],
      retryErrors: ["opencode"],
    });

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
    expect(await app.lastCall("retry_restore_entry")).toBeNull();
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
          name: "CLI",
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
    await expect(dialog.getByText("CLI")).toBeVisible();
    // `.first()`: each stage is drawn twice per row, as the pill and as the
    // Stage line of the diagnostics list under it.
    await expect(dialog.getByText("Configuration written").first()).toBeVisible();
    await expect(dialog.getByText("Write failed").first()).toBeVisible();
    // The interruption is the case this exists for: an entry never attempted must
    // read as not started, not as fine and not as failed.
    await expect(dialog.getByText("Not started").first()).toBeVisible();
    // The operation itself, which AG-570 asks be named along with its update
    // time and what it was trying to achieve.
    await expect(dialog.getByText(/Turning routing back on/)).toBeVisible();
    await expect(dialog.getByText(/of 3 stages completed/)).toBeVisible();
  });

  /** The rest of what the AC asks the review for: the failure's *category*, the
   *  last check that concluded, and the process state. Per tool. */
  test("it shows a category, a last check and a process state per tool", async ({
    boot,
  }) => {
    const app = await boot(withJournal);

    await app.page.getByRole("button", { name: "Review details" }).click();

    const dialog = app.page.getByRole("dialog");
    // Categories, not error strings.
    await expect(dialog.getByText(/Failures by category/)).toBeVisible();
    await expect(dialog.getByText("Configuration write").first()).toBeVisible();
    await expect(dialog.getByText("Last verified route").first()).toBeVisible();
    await expect(dialog.getByText("Last check").first()).toBeVisible();
    await expect(dialog.getByText("Not running").first()).toBeVisible();
    await expect(dialog.getByText("Next action").first()).toBeVisible();
  });

  /**
   * The panel a real master-on produced, with the engine still coming up.
   *
   * Five entries, four of them reading "Write failed / Configuration write /
   * Gate could not write this tool's config" - and nothing had been written.
   * `restore_all` runs a pass before the engine is up, and every entry that
   * needs the engine was attempted in it and its refusal recorded as a failed
   * write: the providers because `enable_inner`'s "nothing to do yet" branch
   * only returned `Applied::NotYet` when the skip list happened to be
   * non-empty, and the tools because that pass had no early-out at all.
   *
   * The row shapes here are the ones that reached the user: a provider whose
   * only route is a proxy domain, a provider that genuinely failed, and a tool
   * whose config is the engine's own address.
   */
  test("an engine that was still starting is not reported as a failed write", async ({
    boot,
  }) => {
    const app = await boot({
      proxy: { running: true, ca_trusted: true },
      pendingRestore: {
        providers: [
          { slug: "openrouter", name: "OpenRouter" },
          { slug: "anthropic", name: "Anthropic" },
        ],
        tools: [{ slug: "env-proxy", name: "Terminal tools" }],
      },
      restoreJournal: {
        updated_unix: 1_760_000_000,
        requested_routing_on: true,
        entries: [
          // Proxy-only: no `tool_ids` at all, so there is never a config file
          // for anything to have failed to write.
          {
            slug: "openrouter",
            name: "OpenRouter",
            kind: "provider" as const,
            outcome: "deferred_engine_down" as const,
            at_unix: 1_760_000_000,
          },
          // A real failure, and now with the reason the journal carries.
          {
            slug: "anthropic",
            name: "Anthropic",
            kind: "provider" as const,
            outcome: "write_failed" as const,
            at_unix: 1_760_000_001,
            error: "configuring Claude Code: permission denied",
          },
          {
            slug: "env-proxy",
            name: "Terminal tools",
            kind: "tool" as const,
            outcome: "deferred_engine_down" as const,
            at_unix: 1_760_000_002,
          },
        ],
      },
    });

    await app.page.getByRole("button", { name: "Review details" }).click();
    const dialog = app.page.getByRole("dialog");
    await expect(dialog).toBeVisible();

    // Waiting, not failed - and said apart from the failures, so the two
    // deferrals do not read as faults.
    await expect(dialog.getByText("Waiting for routing").first()).toBeVisible();
    await expect(dialog.getByText(/2 entries are waiting for routing/)).toBeVisible();
    // Exactly one entry belongs under the failure heading now.
    await expect(dialog.getByText(/Failures by category: Configuration write\./)).toBeVisible();

    // The provider that failed says so about its *routing*, not about a config
    // file - OpenRouter has none, and the sentence was shared.
    await expect(
      dialog.getByText(/could not restore this provider's routing/),
    ).toBeVisible();
    await expect(dialog.getByText(/write this tool's config/)).toHaveCount(0);

    // And it says what went wrong, which the category never could.
    await dialog.getByRole("group").filter({ hasText: "Details" }).first().click();
    await expect(
      dialog.getByText("configuring Claude Code: permission denied"),
    ).toBeVisible();

    // A provider is checked through its members, so "Never checked" would
    // invite the reader to go and check something that has no per-provider
    // check to run.
    await expect(
      dialog.getByText("Checked per tool, not per provider").first(),
    ).toBeVisible();
    await expect(dialog.getByText("Never checked")).toHaveCount(0);

    // Nothing here has a process name Gate can look for: two provider slugs
    // and the environment channel, which is not a process.
    await expect(
      dialog.getByText("Gate has no process to look for").first(),
    ).toBeVisible();
    // `exact`, because the deferral's own sentence says the proxy "was not
    // running yet" and a substring match picks that up - the assertion is about
    // the Process line, whose whole text is the reading.
    await expect(dialog.getByText("Not running", { exact: true })).toHaveCount(0);
    await expect(
      dialog.getByText("Gate has no process to look for", { exact: true }),
    ).toHaveCount(3);
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

  /**
   * An interruption before the journal was written leaves the snapshots but no
   * record of an attempt. The review still opens, and says exactly that: the
   * entries are seeded from the snapshots as never started, which is a real
   * answer rather than an empty dialog. It used to be hidden here, when the
   * dialog had nothing but journal entries to show.
   */
  test("without a journal the review says nothing was started", async ({ boot }) => {
    const app = await boot({ ...withJournal, restoreJournal: null });

    await expect(app.page.getByText("Routing didn’t finish coming back")).toBeVisible();
    await app.page.getByRole("button", { name: "Review details" }).click();

    const dialog = app.page.getByRole("dialog");
    await expect(dialog.getByText("OpenCode")).toBeVisible();
    await expect(dialog.getByText("Not started").first()).toBeVisible();
    // No journal, no update time. Unknown rather than 1970.
    await expect(dialog.getByText(/last updated/)).toHaveCount(0);
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
        name: "CLI",
        upstream_provider_name: "OpenAI",
        default_upstream_url: "https://gw.example/codex",
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

    await app.page.getByRole("switch", { name: "Let Gate Connect manage CLI" }).click();

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

    await app.page.getByRole("switch", { name: "Let Gate Connect manage CLI" }).click();

    const dialog = app.page.getByRole("dialog");
    await expect(dialog).toBeVisible();
    await expect(dialog.getByText("The file that changes:")).toHaveCount(0);
  });
});

/**
 * The rail as `Components / Sidenav` draws it (read 2026-08-23): proxy-routed
 * members are rows beside the config tools, every eyebrow carries its
 * protected-over-total counter, and each group is one client. Hook tests
 * cannot see whether a row's switch reaches the right command, which is what
 * the first of these pins.
 */
test.describe("new UI sidebar rail", () => {
  test.beforeEach(async ({ page }) => {
    await page.addInitScript((k) => localStorage.setItem(k.gc, "1"), useNewUi);
  });

  test("an app switch routes every surface that app uses", async ({ boot }) => {
    const app = await boot({ proxy: { running: true, ca_trusted: true } });

    // One switch per app, so this is a cascade rather than one write. ChatGPT /
    // Codex holds a config tool and two chatgpt.com surfaces, and the two
    // surfaces carry the user's own session - so the switch asks before it
    // routes them, which is the confirmation below.
    await app.page.getByRole("switch", { name: "ChatGPT / Codex" }).click();
    await app.page.getByRole("button", { name: "Route ChatGPT / Codex" }).click();

    // The domains route through the engine's flags, never a config write.
    await expect
      .poll(async () =>
        (await app.state()).proxy.domains
          .filter((d) => d.enabled)
          .map((d) => d.slug)
          .sort(),
      )
      .toEqual(["chatgpt", "chatgpt-apps"]);
  });

  test("an app switch asks before it routes a surface you are signed in to", async ({
    boot,
  }) => {
    const app = await boot({ proxy: { running: true, ca_trusted: true } });

    // The one place the ledger deliberately cascades over a session-credential
    // row. `provider::cascade_domains` still refuses those rows in Rust, so
    // consent is what stands in for the refusal here - and declining must leave
    // everything where it was.
    await app.page.getByRole("switch", { name: "Claude" }).click();
    await expect(
      app.page.getByRole("heading", { name: "Route Claude through Gate?" }),
    ).toBeVisible();

    await app.page.getByRole("button", { name: "Not now" }).click();
    expect(await callsFor(app.page, "proxy_set_domain")).toEqual([]);
    expect(await callsFor(app.page, "connect_tool")).toEqual([]);
  });

  test("a row opens a pane that says what its counters measured", async ({
    boot,
  }) => {
    const app = await boot({ proxy: { running: true, ca_trusted: true } });

    // A row is an app, so its pane covers several surfaces the gateway
    // attributes differently. The counters are the config tool's; saying so is
    // the difference between a measurement and a plausible number.
    await app.page
      .getByRole("listitem")
      .filter({ has: app.page.getByRole("switch", { name: "Claude", exact: true }) })
      .getByRole("button")
      .click();

    await expect(app.page.getByRole("heading", { name: "Claude" })).toBeVisible();
    await expect(app.page.getByText(/These counts cover/)).toBeVisible();
  });

  test("a row with nothing attributable says so instead of reporting a quiet day", async ({
    boot,
  }) => {
    const app = await boot({ proxy: { running: true, ca_trusted: true } });

    // OpenAI API is a host with no config tool behind it, so no reading exists
    // and none ever will. A different sentence from the one above, and
    // deliberately so: that one caveats a reading, this one reports its absence.
    await app.page
      .getByRole("listitem")
      .filter({ has: app.page.getByRole("switch", { name: "OpenAI API" }) })
      .getByRole("button")
      .click();

    await expect(app.page.getByText(/aren't attributed to a single app/)).toBeVisible();
    await expect(app.page.getByText(/These counts cover/)).toHaveCount(0);
  });

  test("a band's eyebrow counts protected rows over rows", async ({ boot }) => {
    const app = await boot({ proxy: { running: true, ca_trusted: true } });

    // The eyebrow is the band now rather than a vendor, because a row is an app
    // and labelling each row's own group would print every name twice. Read off
    // the band's own counter rather than by text: both bands draw one.
    const tools = app.page
      .getByRole("heading", { name: "Tools", exact: true })
      .locator("xpath=following-sibling::span");
    await expect(tools).toHaveText(/of \d+$/);
  });

  test("the multi-provider tools each get a switch, under one band", async ({ boot }) => {
    const app = await boot({
      proxy: { running: true, ca_trusted: true },
      // OpenClaw beside OpenCode: with one such tool on the machine, a row of
      // its own and a shared row look the same.
      tools: [
        {
          slug: "opencode",
          name: "OpenCode",
          upstream_provider_name: "your existing providers",
          default_upstream_url: "https://openrouter.ai/api/v1",
          status: { kind: "detected" },
        },
        {
          slug: "openclaw",
          name: "CLI",
          upstream_provider_name: "your existing providers",
          default_upstream_url: "https://openrouter.ai/api/v1",
          status: { kind: "detected" },
        },
        // The environment channel. It is the machine-wide row now rather than
        // OpenCode's roommate: what it routes is every program started after
        // the next login, which is a different client from the editor.
        {
          slug: "env-proxy",
          name: "Terminal tools",
          upstream_provider_name: "your existing providers",
          default_upstream_url: "https://openrouter.ai/api/v1",
          status: { kind: "detected" },
          client: "any-app",
          scope: "machine",
          credential: "brokered",
        },
      ],
    });

    // A row per tool, under one band. OpenClaw, OpenCode and the environment
    // channel are three programs and three switches - the environment channel
    // is its own row now rather than OpenCode's roommate, because what it
    // routes is every program started after the next login.
    for (const name of ["OpenClaw", "OpenCode", "Terminal"]) {
      await expect(app.page.getByRole("switch", { name, exact: true })).toBeVisible();
    }
    // The headings are the two bands, and nothing else. Every earlier grouping
    // this rail had - vendors, then clients, then a catch-all for whatever
    // those could not place - is gone.
    await expect(app.page.getByRole("heading", { name: "Tools", exact: true })).toBeVisible();
    for (const gone of ["Other tools", "Experimental", "Any app on this machine"]) {
      await expect(app.page.getByRole("heading", { name: gone })).toHaveCount(0);
    }
  });
});
