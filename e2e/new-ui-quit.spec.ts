import { test, expect } from "./fixtures";

/**
 * The window shell's quit flow as the file draws it (`Flows / Overview`,
 * `overview-quit`, read 2026-08-28): choose how to quit, then a confirmation
 * that reports what happened and holds the button that actually exits.
 *
 * AG-596's rule survives the redraw and is the interesting half: Gate Connect
 * "does not claim cleanup completed", so a teardown that leaves a tool behind
 * must reach the left-behind dialog rather than the confirmation, whose copy
 * would be a false claim.
 *
 * What this covers that the unit tests cannot: that the deferred quit actually
 * reaches this window, that the dialog outranks the other overlays, and that
 * nothing exits before the last button.
 *
 * Opts into the new shell per-test; the suite default is the popover.
 */
const useNewUi = { gc: "gc.newUi" };

const connectedTools = [
  {
    slug: "claude-code",
    name: "CLI",
    upstream_provider_name: "Anthropic",
    default_upstream_url: "https://api.anthropic.com",
    status: { kind: "connected" as const },
  },
  {
    slug: "codex",
    name: "CLI",
    upstream_provider_name: "OpenAI",
    default_upstream_url: "https://api.openai.com",
    status: { kind: "connected" as const },
  },
];

test.describe("new UI quit", () => {
  test.beforeEach(async ({ page }) => {
    await page.addInitScript((k) => localStorage.setItem(k.gc, "1"), useNewUi);
  });

  test("a deferred quit raises the confirmation, with one way out", async ({
    boot,
  }) => {
    const app = await boot({
      proxy: { running: true, ca_trusted: true },
      tools: connectedTools,
      pendingQuitTools: { tools: ["Claude Code", "Codex"], reverting: ["Claude Code", "Codex"] },
    });

    const dialog = app.page.getByRole("dialog");
    await expect(dialog).toBeVisible();
    await expect(dialog.getByText("2 protected apps are still routed through Gate")).toBeVisible();

    // One outcome since 2026-09-24: the second row, its "Safest" pill and the
    // note about closing the window were all removed, so there is nothing to
    // choose between and the sentence carries the whole answer.
    await expect(dialog).toContainText(
      "Restores your configurations and turns routing off",
    );
    // And no restart warning: quit-and-disconnect drains the forwarder rather
    // than stopping it (#363), so a tool already holding its address keeps
    // working until the login session ends. The claim is asserted ABSENT
    // because it was true until that landed.
    await expect(dialog).not.toContainText("Save your work");
    await expect(dialog).not.toContainText("restarting");
    await expect(dialog.getByRole("radio")).toHaveCount(0);
    await expect(dialog.getByRole("button", { name: "Disconnect" })).toBeVisible();
    await expect(dialog.getByRole("button", { name: "Cancel" })).toBeVisible();
  });

  test("disconnecting reports it, and only then closes the app", async ({ boot }) => {
    const app = await boot({
      proxy: { running: true, ca_trusted: true },
      tools: connectedTools,
      pendingQuitTools: { tools: ["Claude Code", "Codex"], reverting: ["Claude Code", "Codex"] },
    });

    await app.page.getByRole("button", { name: "Disconnect" }).click();

    await expect.poll(() => app.lastCall("disconnect_tools_for_quit")).not.toBeNull();
    // The teardown ran; the exit has not. That gap is what lets the
    // confirmation speak in the past tense.
    await expect(app.page.getByText(/Tools are disconnected/)).toBeVisible();
    expect(await app.lastCall("quit_app")).toBeNull();

    await app.page.getByRole("button", { name: "Close Gate Connect" }).click();
    await expect.poll(() => app.lastCall("quit_app")).not.toBeNull();
  });

  /**
   * Choosing to leave runs no teardown *here* - which is not the same as
   * touching nothing, and used to be described as if it were. The exit itself
   * puts back any config naming an address that dies with the process, so both
   * screens on this branch name those tools rather than promising the whole set
   * stays pointed at Gate.
   */

  /**
   * The behaviour AG-596 turns on. The teardown succeeds overall but leaves
   * Codex on Gate's settings, so the app must not exit, and must not show the
   * confirmation that says everything was restored.
   */
  test("a tool left on Gate is named instead of confirmed", async ({ boot }) => {
    const app = await boot({
      proxy: { running: true, ca_trusted: true },
      tools: connectedTools,
      pendingQuitTools: { tools: ["Claude Code", "Codex"], reverting: ["Claude Code", "Codex"] },
      quitLeftBehind: ["Codex"],
    });

    await app.page.getByRole("button", { name: "Disconnect" }).click();

    await expect(app.page.getByText(/Couldn’t put Codex back/)).toBeVisible();
    await expect(app.page.getByText(/Tools are disconnected/)).toHaveCount(0);
    expect(await app.lastCall("quit_app")).toBeNull();
    // Retrying only retouches what failed; quitting stays available. `exact`
    // because the Overview's activity notices carry their own retry, named
    // "Try again: <section>" so assistive tech can tell them apart.
    await expect(
      app.page.getByRole("button", { name: "Try again", exact: true }),
    ).toBeVisible();
    await expect(app.page.getByRole("button", { name: "Quit anyway" })).toBeVisible();
  });

  test("cancelling leaves the window open with nothing changed", async ({ boot }) => {
    const app = await boot({
      proxy: { running: true, ca_trusted: true },
      tools: connectedTools,
      pendingQuitTools: { tools: ["Claude Code"], reverting: ["Claude Code"] },
    });

    await app.page.getByRole("button", { name: "Cancel" }).click();

    await expect(app.page.getByRole("dialog")).toHaveCount(0);
    expect(await app.lastCall("quit_app")).toBeNull();
    expect(await app.lastCall("disconnect_tools_for_quit")).toBeNull();
  });

  /**
   * The menu entry the file drew into `topnav/menu` on 2026-08-28. Its list of
   * routed tools is derived here rather than swept from the backend buffer,
   * which only fills when the *tray* deferred a quit.
   *
   * The other half it cannot derive: whether a plain quit puts a config back
   * depends on the address that config holds, not on the tool, so it asks
   * `tools_stranded_by_quit` rather than keeping a second copy of the rule.
   */

  /** A read that did not complete must not borrow the "nothing reverts"
   * sentence: that one tells the user their configs stay put, and the exit may
   * be about to rewrite them. */

  /** Linux mirrors the tray's `request_quit`, which exits outright there: the
   * engine is a detached daemon that outlives the window, so a quit strands
   * nothing and the chooser's first row would tear down routing the user never
   * needed to lose. Its second row also promised a revert `quit_app` skips on
   * that platform. */
  test("on Linux the menu quits outright, even with tools routed", async ({ boot }) => {
    const app = await boot({
      platform: "linux",
      proxy: { running: true, ca_trusted: true },
      tools: connectedTools,
      strandedByQuit: ["Codex"],
    });

    await app.page.getByRole("button", { name: "More" }).click();
    await app.page.getByRole("menuitem", { name: "Quit Gate Connect" }).click();

    await expect.poll(() => app.lastCall("quit_app")).not.toBeNull();
    await expect(app.page.getByRole("dialog")).toHaveCount(0);
    expect(await app.lastCall("tools_stranded_by_quit")).toBeNull();
  });

  test("with nothing routed, the menu quits without asking how", async ({ boot }) => {
    // No teardown to choose between, which is the rule the tray's own Quit
    // already follows - Rust exits outright on an empty list.
    const app = await boot({ proxy: { running: true, ca_trusted: true } });

    await app.page.getByRole("button", { name: "More" }).click();
    await app.page.getByRole("menuitem", { name: "Quit Gate Connect" }).click();

    await expect.poll(() => app.lastCall("quit_app")).not.toBeNull();
    await expect(app.page.getByRole("dialog")).toHaveCount(0);
  });
});
