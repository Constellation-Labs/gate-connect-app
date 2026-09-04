import { test, expect } from "./fixtures";

/**
 * The tray popover (window label `tray`, Figma `Flows / Tray`, built
 * 2026-08-28): the compact quick-status surface the tray icon toggles beside
 * the full window.
 *
 * Reached by pointing the fake `getCurrentWindow()` label at "tray" - the
 * surface is picked per window, not by the `gc.newUi` shell flag, so no
 * localStorage opt-in is involved. What these pin is the seam the component
 * tests cannot see: whether the controls are connected to the backend at all.
 */
test.describe("tray popover", () => {
  test("a row switch routes its tool through the same dispatch as the rail", async ({
    boot,
  }) => {
    const app = await boot({
      windowLabel: "tray",
      // Engine on and certificate trusted, so the toggle reaches the config
      // write without raising the trust dialog first.
      proxy: { running: true, ca_trusted: true },
    });

    // The card's eyebrow in front of the row label: rows are named for the
    // surface they cover, so "CLI" alone names Claude Code and Codex both.
    await app.page.getByRole("switch", { name: "Anthropic CLI", exact: true }).click();

    await expect.poll(() => app.lastCall("connect_tool")).toMatchObject({
      slug: "claude-code",
    });
  });

  /**
   * AG-570 AC 4: the recovery action stays reachable from Overview, tool detail
   * *and* the tray. The tray gets the action, not just the fact - a surface that
   * reported the problem and offered nothing would be the one place the user
   * could see it and not act.
   */
  test("an interrupted restore is offered from the tray, not just reported", async ({
    boot,
  }) => {
    const app = await boot({
      windowLabel: "tray",
      proxy: { running: true, ca_trusted: true },
      pendingRestore: {
        providers: [],
        tools: [{ slug: "opencode", name: "OpenCode" }],
      },
    });

    await expect(app.page.getByText("Routing didn’t finish")).toBeVisible();
    await expect(app.page.getByText(/OpenCode is still waiting/)).toBeVisible();

    await app.page.getByRole("button", { name: "Resume now" }).click();

    // The batch call here, not the window's per-entry walk: a 400px popover has
    // nowhere to put a progress list, and one that closes on focus loss must not
    // leave a pass half driven.
    await expect.poll(() => app.lastCall("resume_restore")).not.toBeNull();
  });

  test("the tray sends the per-tool account to the window", async ({ boot }) => {
    const app = await boot({
      windowLabel: "tray",
      proxy: { running: true, ca_trusted: true },
      pendingRestore: {
        providers: [],
        tools: [{ slug: "opencode", name: "OpenCode" }],
      },
    });

    await app.page.getByRole("button", { name: "Review details" }).click();

    // Reveals the window rather than drawing a second, shorter version of the
    // same operation.
    await expect.poll(() => app.lastCall("reveal_popover")).not.toBeNull();
  });

  test("Expand app reveals the main window", async ({ boot }) => {
    const app = await boot({ windowLabel: "tray" });

    await app.page.getByRole("button", { name: "Expand app" }).click();

    await expect.poll(() => app.lastCall("reveal_popover")).not.toBeNull();
  });

  test("the menu's Quit goes through the tray-menu quit path", async ({ boot }) => {
    const app = await boot({ windowLabel: "tray" });

    await app.page.getByRole("button", { name: "More" }).click();
    await app.page.getByRole("menuitem", { name: "Quit Gate Connect" }).click();

    // `request_app_quit` defers to the main window's three-way dialog when
    // config-routed tools are still managed; the tray never raises its own.
    await expect.poll(() => app.lastCall("request_app_quit")).not.toBeNull();
  });

  test("signed out, it hands over instead of painting empty groups", async ({ boot }) => {
    const app = await boot({ windowLabel: "tray", account: null });

    await expect(
      app.page.getByRole("heading", { name: "Sign in to get started" }),
    ).toBeVisible();

    await app.page.getByRole("button", { name: "Open Gate Connect" }).click();
    await expect.poll(() => app.lastCall("reveal_popover")).not.toBeNull();
  });

  test("a row counts the feed's events against its own tool", async ({ boot }) => {
    // The seam the component test cannot see: that the popover reads the feed's
    // buffer on open and attributes it per row. Two events for claude-code, one
    // unattributed - which must be counted against nobody.
    const event = {
      id: "01A",
      requestId: "req-8f3c",
      at: "2026-08-31T14:03:00Z",
      action: "block" as const,
      category: "credential",
      tool: "claude-code",
      model: "claude-opus-4",
      provider: "anthropic",
    };
    const app = await boot({
      windowLabel: "tray",
      securityFeed: {
        state: "live",
        events: [
          event,
          { ...event, id: "01B", requestId: "req-11aa", action: "flag" as const },
          { ...event, id: "01C", requestId: "req-22bb", tool: null },
        ],
      },
    });

    const row = app.page
      .getByRole("listitem")
      .filter({ has: app.page.getByRole("switch", { name: "Anthropic CLI", exact: true }) });
    await expect(row).toContainText("2 alerts");

    // And it moves without a reopen, because the popover is listening.
    await app.emit("security-event", { ...event, id: "01D", requestId: "req-33cc" });
    await expect(row).toContainText("3 alerts");
  });

  test("the popover re-reads on tools-changed rather than on a timer", async ({ boot }) => {
    // The tray ran the same 5s poll behind a surface the tray icon opens and
    // closes all day. It listens now, like the window shell.
    const app = await boot({ windowLabel: "tray", tools: [] });
    const row = app.page.getByRole("switch", { name: "OpenAI CLI", exact: true });
    await expect(row).toHaveCount(0);

    await app.patch({
      tools: [
        {
          slug: "codex",
          name: "CLI",
          upstream_provider_name: "OpenAI",
          default_upstream_url: "https://api.openai.com/v1",
          status: { kind: "detected" as const },
        },
      ],
    });
    await expect(row).toHaveCount(0);

    await app.emit("tools-changed");

    await expect(row).toBeVisible();
  });

  test("an org switch in the other window reaches the popover", async ({ boot }) => {
    // The popover renders one account's everything - the footer's org, the
    // security count, the per-row figures - and all of it is keyed on state read
    // at mount. None of the account-mutating commands emitted anything, so the
    // tray kept the previous org until something unrelated woke it. `set_org` and
    // its siblings emit `session-changed` now; this is the listener.
    const app = await boot({ windowLabel: "tray" });
    await expect(app.page.getByText("Constellation Labs")).toBeVisible();

    // Switched in the main window, which this popover cannot see happen.
    await app.patch({ account: { org_name: "Side Project" } });
    await expect(app.page.getByText("Constellation Labs")).toBeVisible();

    await app.emit("session-changed");

    await expect(app.page.getByText("Side Project")).toBeVisible();
    await expect(app.page.getByText("Constellation Labs")).toHaveCount(0);
  });
});
