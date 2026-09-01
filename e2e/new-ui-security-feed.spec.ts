import { test, expect } from "./fixtures";

/**
 * The live security-event feed (AG-578), end to end through the real window.
 *
 * What this covers that `SecurityPane.test.tsx` cannot: that the pane is reachable
 * from the rail at all, that the window is actually *listening* for pushed events
 * rather than only rendering props, that a window opening mid-session recovers the
 * buffer it missed, and that the dashboard link is built from the event.
 *
 * The fake backend stands in for the connection: the real one holds an SSE stream
 * open in Rust and emits `security-event` / `security-feed-state`, and here a test
 * emits the same two. From the window's side those are the same thing, which is
 * the whole reason the transport lives behind an event boundary.
 */
const useNewUi = { gc: "gc.newUi" };

/** One event, in the wire shape. Note what is not here - no prompt, no response,
 *  no matched value. The gateway omits them; there is nothing to hide. */
const blocked = {
  id: "01A",
  requestId: "req-8f3c",
  at: "2026-08-31T14:03:00Z",
  action: "block" as const,
  category: "credential",
  tool: "claude-code",
  model: "claude-opus-4",
  provider: "anthropic",
};

test.describe("new UI security feed", () => {
  test.beforeEach(async ({ page }) => {
    await page.addInitScript((k) => localStorage.setItem(k.gc, "1"), useNewUi);
  });

  test("an empty feed says so, rather than saying nothing", async ({ boot }) => {
    const app = await boot({});

    await app.page.getByRole("button", { name: "Security events" }).click();

    await expect(app.page.getByText("No security events")).toBeVisible();
    // A loaded-and-empty feed is a real answer and must not read as a failure.
    await expect(app.page.getByText("Unavailable")).toHaveCount(0);
  });

  test("an event pushed while the pane is open appears on it", async ({ boot }) => {
    const app = await boot({});
    await app.page.getByRole("button", { name: "Security events" }).click();
    await expect(app.page.getByText("No security events")).toBeVisible();

    await app.emit("security-event", blocked);

    await expect(app.page.getByText("Blocked")).toBeVisible();
    await expect(app.page.getByText("credential")).toBeVisible();
    await expect(app.page.getByText("claude-code")).toBeVisible();
    await expect(app.page.getByText("No security events")).toHaveCount(0);
  });

  test("a window opened after the fact recovers the events it missed", async ({ boot }) => {
    // Tauri events only reach a window that is already listening. Without the
    // buffer read on mount this pane would say "No security events" about a
    // session that had two, which is a claim about the user's traffic rather
    // than about this window's uptime.
    const app = await boot({
      securityFeed: {
        state: "live",
        events: [blocked, { ...blocked, id: "01B", requestId: "req-11aa", action: "flag" }],
      },
    });

    await app.page.getByRole("button", { name: "Security events" }).click();

    await expect(app.page.getByText("Blocked")).toBeVisible();
    await expect(app.page.getByText("Flagged")).toBeVisible();
  });

  test("the feed reports its own connection, and routing keeps working", async ({ boot }) => {
    // AC4. The master switch is the check that matters here: a feed that drops
    // must not touch it, because the two are unrelated and conflating them is
    // what makes a user turn routing off to fix a network blip.
    const app = await boot({});
    await app.page.getByRole("button", { name: "Security events" }).click();
    await expect(app.page.getByRole("status", { name: "Event feed Live" })).toBeVisible();

    const routingBefore = await app.state().then((s) => s.proxy.running);

    await app.emit("security-feed-state", "reconnecting");
    await expect(app.page.getByRole("status", { name: "Event feed Reconnecting" })).toBeVisible();

    await app.emit("security-feed-state", "offline");
    await expect(app.page.getByRole("status", { name: "Event feed Offline" })).toBeVisible();

    // Routing is untouched by any of that. Asserted as "unchanged" rather than
    // as a fixed value: the invariant is that the feed cannot move it, and a
    // test that pinned `true` would pass or fail on the fixture's default
    // instead of on the thing under test.
    const routingAfter = await app.state().then((s) => s.proxy.running);
    expect(routingAfter).toBe(routingBefore);
  });

  test("events already on screen survive a reconnect", async ({ boot }) => {
    const app = await boot({ securityFeed: { state: "live", events: [blocked] } });
    await app.page.getByRole("button", { name: "Security events" }).click();
    await expect(app.page.getByText("Blocked")).toBeVisible();

    await app.emit("security-feed-state", "reconnecting");

    // A feed having a bad minute is not an empty feed; blanking the table would
    // lose what the user was reading.
    await expect(app.page.getByText("Blocked")).toBeVisible();
    await expect(app.page.getByText("No security events")).toHaveCount(0);
  });

  test("opening an event offers the dashboard and keeps the summary up", async ({ boot }) => {
    const app = await boot({ securityFeed: { state: "live", events: [blocked] } });
    await app.page.getByRole("button", { name: "Security events" }).click();

    await app.page.getByRole("button", { name: /View/ }).click();

    // AC7: the summary is what stays visible until the dashboard has the event.
    await expect(app.page.getByRole("heading", { name: "Blocked request" })).toBeVisible();
    await expect(app.page.getByText("req-8f3c")).toBeVisible();

    await app.page.getByRole("button", { name: "Open in dashboard" }).click();
    await expect
      .poll(() => app.lastCall("plugin:opener|open_url"))
      .toMatchObject({ url: expect.stringContaining("messages/req-8f3c") });
  });

  test("a long-running window keeps a bounded feed", async ({ boot }) => {
    // A window stays open for days. Without a cap the array grows for as long as
    // the app runs and every render walks it. 200 matches the backend buffer, so
    // this window shows the same depth of history as one opened a moment ago.
    const app = await boot({});
    await app.page.getByRole("button", { name: "Security events" }).click();
    await expect(app.page.getByText("No security events")).toBeVisible();

    for (let i = 0; i < 205; i++) {
      await app.emit("security-event", {
        ...blocked,
        id: `evt-${i}`,
        requestId: `req-${i}`,
        category: `cat-${i}`,
      });
    }

    await expect.poll(() => app.page.getByText("cat-204").count()).toBe(1);
    // The oldest five fell off rather than accumulating.
    expect(await app.page.getByText("cat-0", { exact: true }).count()).toBe(0);
    expect(await app.page.getByText("cat-5", { exact: true }).count()).toBe(1);
  });

  test("the notification switches reach the backend", async ({ boot }) => {
    // The three AG-594 asked for and AG-578 made real. Each must actually stop
    // something, which is why they waited for a feed that could fire them.
    const app = await boot({});
    await app.page.getByRole("button", { name: "Settings" }).click();

    const blockedSwitch = app.page.getByRole("switch", { name: "Blocked requests" });
    await expect(blockedSwitch).toHaveAttribute("aria-checked", "true");
    await blockedSwitch.click();
    await expect
      .poll(() => app.lastCall("set_blocked_event_notifications"))
      .toEqual({ enabled: false });

    await app.page.getByRole("switch", { name: "Flagged requests" }).click();
    await expect
      .poll(() => app.lastCall("set_flagged_event_notifications"))
      .toEqual({ enabled: false });

    await app.page.getByRole("switch", { name: "Notification sound" }).click();
    await expect
      .poll(() => app.lastCall("set_security_notification_sound"))
      .toEqual({ enabled: false });
  });
});
