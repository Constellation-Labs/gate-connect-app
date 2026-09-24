import { test, expect } from "./fixtures";
import { OPENCLAW } from "./backend";

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
    const sidebarSwitch = await app.appSwitch("ChatGPT / Codex");
    await expect(sidebarSwitch).toHaveAttribute("aria-checked", "true");

    await sidebarSwitch.click();

    await expect(app.page.getByRole("dialog")).toHaveCount(0);
    expect(await callsFor(app.page, "disconnect_tool")).toHaveLength(1);
    expect(await callsFor(app.page, "connect_tool")).toEqual([]);
  });

  test("re-adopting from the alert card goes through the review", async ({ boot }) => {
    const app = await boot(driftedCodex);
    await app.openSection("ChatGPT / Codex");

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
    await app.openSection("ChatGPT / Codex");

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

    // The switch directly, not `routeApp`, to keep this explicit about the
    // direction: this one is turning the app OFF. That nothing is asked here
    // is the assertion below.
    await (await app.appSwitch("Claude")).click();

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
    await app.openSection("ChatGPT / Codex");

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
    await app.openSection("ChatGPT / Codex");

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

    // Through the helper: this section holds a session surface, so its switch
    // asks before it routes. The answer is recorded, which is why the retry
    // below is a plain click.
    await app.routeApp("ChatGPT / Codex");

    // The rail row keeps the phrase and drops the reason, which does not fit
    // 250px. The pane header is the surface with room for the sentence, and it
    // outlives the banner - which is the half of this that still matters.
    //
    // Opened by the section's name: the row is the app, and Codex is inside it.
    await app.page.getByRole("button", { name: "ChatGPT / Codex" }).first().click();
    await expect(app.page.getByText("Configuration update failed")).toBeVisible();
  });

  test("a retry that succeeds clears the failure from the pane header", async ({ boot }) => {
    const app = await boot({
      proxy: { running: true, ca_trusted: true },
      tools: [{ ...codex, status: { kind: "detected" as const } }],
      failures: { connect_tool: "failed to write ~/.codex/config.toml" },
    });

    // Through the helper: this section holds a session surface, so its switch
    // asks before it routes. The answer is recorded, which is why the retry
    // below is a plain click.
    await app.routeApp("ChatGPT / Codex");

    // Opened before the retry, not after: with the pane closed the reason is
    // nowhere on the page and the count below would pass without the retry ever
    // having cleared anything.
    await app.page.getByRole("button", { name: "ChatGPT / Codex" }).first().click();
    await expect(app.page.getByText("Configuration update failed")).toBeVisible();

    // Clear the injected failure, then click again - the switch is the retry.
    // `app.patch` merges objects one level deep, so it cannot *remove* a key;
    // this reaches for the harness state directly to empty the map.
    await app.page.evaluate(() => {
      window.__GATE_E2E__.state.failures = {};
    });
    await (await app.appSwitch("ChatGPT / Codex")).click();

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

    // OpenClaw, because a section exists as soon as anything in it does: Claude
    // and ChatGPT / Codex draw from catalog domains with no tool installed at
    // all, so neither could test "appears when a tool does". OpenClaw, Hermes
    // and OpenCode (while its Zen / Go host is off) are the ones whose only
    // member is the tool. Until then the name is on a display-only row under
    // "Not installed", which is not a button, so the count below still holds.
    // The ROW, not its switch, and not through `appSwitch`: that helper opens
    // the app's pane before returning the control, so it cannot answer "is
    // this row absent" - there would be nothing to open. The rail's row is a
    // button whose accessible name leads with the app name.
    const row = app.page.getByRole("button", { name: "OpenClaw" });
    await expect(row).toHaveCount(0);
    const sweeps = await countOf(app, "routing_verdicts");

    // Installed behind the window's back, exactly as a terminal would.
    await app.patch({
      tools: [
        { ...OPENCLAW },
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

/**
 * AG-570's "Review details": what the restore did, entry by entry, read-only.
 *
 * The criterion is explicit that reviewing "does not change state", so the only
 * action closes it - and the journal holds slugs, display names, outcomes and
 * timestamps, with no credentials or request content, which is what makes showing
 * it in full safe.
 */

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
    await app.openSection("ChatGPT / Codex");

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
    await app.openSection("ChatGPT / Codex");

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

  test("apps detection did not find are listed under Not installed, and counted nowhere", async ({
    boot,
  }) => {
    const missing = (slug: string, name: string) => ({
      ...OPENCLAW,
      slug,
      name,
      status: { kind: "not_installed" as const },
    });
    const app = await boot({
      proxy: { running: true, ca_trusted: true },
      tools: [{ ...OPENCLAW }, missing("hermes", "Hermes"), missing("opencode", "OpenCode")],
    });

    const heading = app.page.getByRole("heading", { name: "Not installed", exact: true });
    await expect(heading).toBeVisible();
    // No "0 of 2" beside it: that would score apps nobody has.
    await expect(heading.locator("xpath=following-sibling::span")).toHaveCount(0);
    // Listed, and display-only: the names are on screen and neither is a
    // button that would open a pane with nothing behind it.
    const list = heading.locator("xpath=../following-sibling::ul");
    await expect(list.getByText("Hermes", { exact: true })).toBeVisible();
    await expect(list.getByText("OpenCode", { exact: true })).toBeVisible();
    await expect(list.getByText("Not installed")).toHaveCount(2);
    await expect(app.page.getByRole("button", { name: /^Hermes/ })).toHaveCount(0);
    await expect(app.page.getByRole("button", { name: /^OpenCode/ })).toHaveCount(0);
    // The installed one keeps its ordinary row.
    await expect(app.page.getByRole("button", { name: /^OpenClaw/ })).toHaveCount(1);

    // The topbar's denominator is every app on the rail, and these are not.
    // Claude, ChatGPT / Codex and OpenClaw: the three rows drawn above.
    await expect(app.page.getByText("0 of 3 Apps on", { exact: true })).toBeVisible();
  });

  test("an app switch routes every surface that app uses", async ({ boot }) => {
    const app = await boot({ proxy: { running: true, ca_trusted: true } });

    // One switch per app, so this is a cascade rather than one write. ChatGPT /
    // Codex holds a config tool and two chatgpt.com surfaces, and the two
    // surfaces carry the user's own session - which the switch used to ask
    // about and no longer does (AG-934).
    await (await app.appSwitch("ChatGPT / Codex")).click();

    // The domains route through the engine's flags, never a config write.
    await expect
      .poll(async () =>
        (await app.state()).proxy.domains
          .filter((d) => d.enabled)
          .map((d) => d.slug)
          .sort(),
      )
      .toEqual(["chatgpt", "chatgpt-apps"]);
    // And the third member, which is the half this used to leave out: the
    // section spans two mechanisms, so asserting only the domains would stay
    // green if the config write stopped firing entirely.
    await expect.poll(() => app.lastCall("connect_tool")).toMatchObject({ slug: "codex" });
  });

  test("an app switch routes a surface you are signed in to, without asking", async ({
    boot,
  }) => {
    const app = await boot({ proxy: { running: true, ca_trusted: true } });

    // The one place the ledger deliberately cascades over a session-credential
    // row. `provider::cascade_domains` still refuses those rows in Rust, so the
    // CLI and the restore path cannot reach them; on THIS path nothing stands
    // in front of it any more.
    //
    // `SessionConsentDialog` used to, and two tests here pinned it: that the
    // switch asked, and that the answer was recorded so it asked only once.
    // Product removed the dialog (AG-934, 2026-09-23), so what needs pinning is
    // the reverse - one click, no question, and `claude-web` routed. Written as
    // a test rather than a deletion because this is a deliberate behaviour
    // change, and it should fail loudly if somebody reinstates the gate without
    // deciding to.
    await (await app.appSwitch("Claude")).click();

    await expect
      .poll(async () =>
        (await app.state()).proxy.domains
          .filter((d) => d.enabled)
          .map((d) => d.slug)
          .sort(),
      )
      .toEqual(["anthropic", "claude-web"]);
    // Settled on the positive fact above first: an empty dialog count read
    // straight after a click passes while the click is still in flight.
    await expect(app.page.getByRole("dialog")).toHaveCount(0);
    await expect(await app.appSwitch("Claude")).toHaveAttribute("aria-checked", "true");
  });

  test("one certificate question for a whole section, not one per surface", async ({
    boot,
  }) => {
    const app = await boot({ proxy: { running: true, ca_trusted: false }, tools: [] });

    // Two members, two writes, and the gate belongs in front of both: asked per
    // member, declining the first left the second one's dialog on screen - the
    // person saying no and being asked again about the same certificate. The
    // decline half is the test above; this is the accept half, and what it pins
    // is that one answer covers the cascade.
    await app.routeApp("Claude");
    await app.page.getByRole("button", { name: "Trust certificate" }).click();

    await expect
      .poll(async () =>
        (await app.state()).proxy.domains
          .filter((d) => d.enabled)
          .map((d) => d.slug)
          .sort(),
      )
      .toEqual(["anthropic", "claude-web"]);
    // One prompt for the cascade, and it is gone.
    await expect(app.page.getByRole("dialog")).toHaveCount(0);
    expect(await callsFor(app.page, "proxy_trust_ca")).toHaveLength(1);
  });

  test("a section made only of session surfaces can be switched back off", async ({
    boot,
  }) => {
    // ChatGPT / Codex with no Codex CLI on the machine is two additive rows and
    // nothing else. Its switch used to count the brokered half, which is empty
    // here, so it read off however the two hosts were set: the click routed
    // them, the switch snapped back to off, and the next click did nothing at
    // all because everything it would flip was already on. The person could turn
    // their signed-in session on and then had no way to turn it off.
    const app = await boot({ proxy: { running: true, ca_trusted: true }, tools: [] });

    await app.routeApp("ChatGPT / Codex");
    await expect(await app.appSwitch("ChatGPT / Codex")).toHaveAttribute("aria-checked", "true");

    await (await app.appSwitch("ChatGPT / Codex")).click();

    await expect(await app.appSwitch("ChatGPT / Codex")).toHaveAttribute("aria-checked", "false");
    await expect
      .poll(async () => (await app.state()).proxy.domains.filter((d) => d.enabled).length)
      .toBe(0);
  });

  test("an app pane with no installed CLI is not called a destination", async ({
    boot,
  }) => {
    // The H in review on #323. `openDomain` is `openTool === null` - "this
    // section has no INSTALLED config tool" - not "this section is a provider
    // endpoint", and a section stays alive on its `domain:` members. So a
    // Claude pane on a machine with no Claude Code takes the unattributed
    // branch too, and "any app on this machine can be pointed here" is false
    // of Claude Desktop: it is one app, and nothing is pointed at it.
    //
    // `tools: []`, which the tests above already boot, because the default
    // fixture ships every CLI as detected and would never see this.
    const app = await boot({
      proxy: { running: true, ca_trusted: true },
      tools: [],
    });

    await app.openApp("Claude");

    await expect(app.page.getByText(/its own activity can/)).toBeVisible();
    await expect(
      app.page.getByText(/can be pointed here/),
    ).toHaveCount(0);
  });

  test("a row with nothing attributable names where its traffic is counted", async ({
    boot,
  }) => {
    // The OpenAI API row, booted ON because `CLI_ONLY_DOMAINS` draws it only
    // while it is. It is the last `providerEndpoint` section left: OpenRouter
    // played this part until 2026-09-23, when Hermes took its domain and the
    // row stopped being drawn in any state.
    const app = await boot({
      proxy: {
        running: true,
        ca_trusted: true,
        domains: [
          {
            slug: "openai",
            display_name: "OpenAI API",
            client: "any-app",
            credential: "brokered",
            scope: "host",
            hosts: ["api.openai.com"],
            upstream_url: "https://api.openai.com",
            rewrite_prefixes: ["/v1/"],
            passthrough_prefixes: [],
            enabled: true,
            supported: true,
          },
        ],
      },
    });

    // A host with no config tool behind it, so no reading exists and none ever
    // will. A different sentence from the one above, and deliberately so: that
    // one caveats a reading, this one names where the requests ARE counted
    // instead (AG-889). The page used to say only that its numbers could not
    // be shown, which read as breakage.
    await app.openApp("OpenAI API");

    // Anchored on the note's own tail. An earlier version matched
    // /counted in the Overview/, which resolved to one element only by luck of
    // capitalisation - three matches the moment either string changed.
    await expect(
      app.page.getByText(/cannot attribute these requests to one app/),
    ).toBeVisible();
    await expect(
      app.page.getByText(/appear in the Overview rather than on this page/),
    ).toBeVisible();
    // The cards are their own string and there are two of them.
    await expect(
      app.page.getByText("Shows in the Overview, not per app"),
    ).toHaveCount(2);
    await expect(app.page.getByText(/These counts cover/)).toHaveCount(0);
  });

  test("a group's eyebrow counts protected rows over rows", async ({ boot }) => {
    const app = await boot({ proxy: { running: true, ca_trusted: true } });

    // The eyebrow is the group rather than the row, because a row is an app and
    // labelling each row's own group would print every name twice. Groups are
    // vendors since 2026-09-22 (Figma `440:1593`), so this reads OpenAI's.
    const openai = app.page
      .getByRole("heading", { name: "OpenAI", exact: true })
      .locator("xpath=following-sibling::span");
    // An exact count, and then the same count after a row moves. `/of \d+$/`
    // passes for "0 of 0", so it went green on a rail that drew no rows at all -
    // and a counter that never changes is not a counter.
    // One: ChatGPT / Codex, which the default catalog draws whether or not a
    // tool is installed. The OpenAI API host is in the catalog too, off, and
    // drawn only while on (`CLI_ONLY_DOMAINS`), so it must not count here.
    await expect(openai).toHaveText("0 of 1");

    await app.routeApp("ChatGPT / Codex");

    await expect(openai).toHaveText("1 of 1");
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

    // A row per tool, under one band. Asserted on the row's own control
    // rather than on a switch: the rail stopped drawing those on 2026-09-22,
    // and a test that checks for one is checking the old design rather than
    // that the row is present.
    for (const name of ["OpenClaw", "OpenCode"]) {
      await expect(app.page.getByRole("button", { name })).toBeVisible();
    }
    // Not the environment channel. It used to be a row here, on the argument
    // that what it routes is a different client from the editor - true, but it
    // is not an app, and the rail is a list of apps. Its control moved to
    // Settings (AG-893), so the fixture keeps it in `list_tools` to prove the
    // filter is what removes it rather than its absence from the fixture.
    await expect(
      app.page.getByRole("button", { name: "Terminal", exact: true }),
    ).toHaveCount(0);
    // The headings are the three vendor groups, and nothing else. The rail has
    // been regrouped more than once - vendors, then clients, then a catch-all,
    // then the apps/tools bands - and the retired names must not linger.
    // "Other apps" is design's wording for the catch-all, not the frame's
    // "Other tools", which is why that one is still in the gone list.
    await expect(
      app.page.getByRole("heading", { name: "Other apps", exact: true }),
    ).toBeVisible();
    for (const gone of [
      "Apps",
      "Tools",
      "Other tools",
      "Experimental",
      "Any app on this machine",
    ]) {
      // `exact`, because the default is substring: without it "Apps" matches
      // the "Other apps" heading that is supposed to be there.
      await expect(
        app.page.getByRole("heading", { name: gone, exact: true }),
      ).toHaveCount(0);
    }
  });
});
