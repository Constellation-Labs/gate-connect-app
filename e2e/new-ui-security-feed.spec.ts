import { test, expect } from "./fixtures";

/**
 * The live security-event feed (AG-578), end to end through the real window.
 *
 * What this covers that `SecurityEvents.test.tsx` cannot: that the feed is on the
 * Overview the window opens on, that the window is actually *listening* for
 * pushed events rather than only rendering props, that a window opening
 * mid-session recovers the buffer it missed, and that the dashboard link is built
 * from the event.
 *
 * **The navigation step is gone as of AG-853.** Every test here used to open the
 * feed by clicking a `Security events` rail entry; the feed is the last section
 * of the Overview now, and the Overview is where the window lands, so there is
 * nothing to click.
 *
 * The fake backend stands in for the connection: the real one holds an SSE stream
 * open in Rust and emits `security-event` / `security-feed-state`, and here a test
 * emits the same two. From the window's side those are the same thing, which is
 * the whole reason the transport lives behind an event boundary.
 */
const useNewUi = { gc: "gc.newUi" };

/**
 * The feed's section on the Overview, and every assertion below is scoped to it.
 *
 * Required rather than tidy, since AG-853. The pane it shares now draws
 * "Blocked" and "Flagged" of its own - the Messages chart's legend and the
 * screen-reader table behind it - so an unscoped `getByText("Blocked")` matches
 * three nodes and fails on strict mode rather than on the feed. Scoping also
 * asserts the thing the move was for: these rows are on the Overview, under the
 * section's own anchor.
 */
const feed = (page: import("@playwright/test").Page) =>
  page.locator("#security-events");

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

  test("the feed is the Overview's last section, and the rail has lost its entry", async ({
    boot,
  }) => {
    // AG-853's first two criteria, which are about placement rather than about
    // the feed: it reads after Token savings, and the standalone way in is gone.
    const app = await boot({});

    const rail = app.page.getByRole("navigation", { name: "Main" });
    await expect(rail.getByRole("button", { name: "Security events" })).toHaveCount(0);
    await expect(rail.getByRole("button", { name: "Overview" })).toHaveCount(1);
    await expect(rail.getByRole("button", { name: "Settings" })).toHaveCount(1);

    // `~` is "a later sibling of", so this matches only while the feed's section
    // really does come after Token savings in the pane. Source order rather than
    // a pixel offset: it is what a keyboard and a screen reader follow down the
    // page, and it does not move when the pane scrolls.
    await expect(app.page.locator("#token-savings ~ #security-events")).toHaveCount(1);
  });

  test("an empty feed says so, rather than saying nothing", async ({ boot }) => {
    const app = await boot({});

    await expect(feed(app.page).getByText("No security events")).toBeVisible();
    // A loaded-and-empty feed is a real answer and must not read as a failure.
    await expect(feed(app.page).getByText("Unavailable")).toHaveCount(0);
  });

  test("a failed catch-up is not reported as an empty feed", async ({ boot }) => {
    // The case that produced the original defect: the stream is Live, so the
    // card looks healthy, but the catch-up that would have answered "is there
    // history?" was refused. Saying "No security events" here is a claim about
    // the user's traffic made by a screen whose question was never answered -
    // the same mistake the whole-feed `Unavailable` state exists to prevent,
    // one layer down.
    const app = await boot({
      securityFeed: { state: "live", events: [], historyOk: false },
    });

    await expect(
      feed(app.page).getByText("Earlier events couldn’t be loaded"),
    ).toBeVisible();
    await expect(feed(app.page).getByText("No security events")).toHaveCount(0);
    // The feed itself is fine, and the pill must go on saying so: the stream
    // and its history fail independently.
    await expect(feed(app.page).getByText("Live")).toBeVisible();
    // Deliberately no recovery action. `retry_now` only wakes the backoff
    // between connection attempts and the catch-up runs once per connection,
    // so while the stream is Live a retry issues no request at all. A control
    // that reliably does nothing teaches the user the feature is broken.
    await expect(feed(app.page).getByRole("button", { name: "Try again" })).toHaveCount(0);
  });

  test("a partial history says so above the rows it does have", async ({ boot }) => {
    // Events on screen and a failed catch-up is not the empty case, but the
    // list is still partial and nothing else in the app would mention it.
    const app = await boot({
      securityFeed: { state: "live", events: [blocked], historyOk: false },
    });

    await expect(
      feed(app.page).getByText("Showing events from this session only."),
    ).toBeVisible();
    await expect(feed(app.page).getByText("Blocked")).toBeVisible();
    await expect(feed(app.page).getByText("No security events")).toHaveCount(0);
  });

  test("an event pushed while the Overview is open appears on it", async ({ boot }) => {
    const app = await boot({});
    await expect(feed(app.page).getByText("No security events")).toBeVisible();

    await app.emit("security-event", blocked);

    await expect(feed(app.page).getByText("Blocked")).toBeVisible();
    await expect(feed(app.page).getByText("credential")).toBeVisible();
    await expect(feed(app.page).getByText("claude-code")).toBeVisible();
    await expect(feed(app.page).getByText("No security events")).toHaveCount(0);
  });

  test("a window opened after the fact recovers the events it missed", async ({ boot }) => {
    // Tauri events only reach a window that is already listening. Without the
    // buffer read on mount this section would say "No security events" about a
    // session that had two, which is a claim about the user's traffic rather
    // than about this window's uptime.
    const app = await boot({
      securityFeed: {
        state: "live",
        events: [blocked, { ...blocked, id: "01B", requestId: "req-11aa", action: "flag" }],
      },
    });

    await expect(feed(app.page).getByText("Blocked")).toBeVisible();
    await expect(feed(app.page).getByText("Flagged")).toBeVisible();
  });

  test("the feed reports its own connection, and routing keeps working", async ({ boot }) => {
    // AC4. The master switch is the check that matters here: a feed that drops
    // must not touch it, because the two are unrelated and conflating them is
    // what makes a user turn routing off to fix a network blip.
    const app = await boot({});
    await expect(feed(app.page).getByRole("status", { name: "Event feed Live" })).toBeVisible();

    const routingBefore = await app.state().then((s) => s.proxy.running);

    await app.emit("security-feed-state", "reconnecting");
    await expect(feed(app.page).getByRole("status", { name: "Event feed Reconnecting" })).toBeVisible();

    await app.emit("security-feed-state", "offline");
    await expect(feed(app.page).getByRole("status", { name: "Event feed Offline" })).toBeVisible();

    // Routing is untouched by any of that. Asserted as "unchanged" rather than
    // as a fixed value: the invariant is that the feed cannot move it, and a
    // test that pinned `true` would pass or fail on the fixture's default
    // instead of on the thing under test.
    const routingAfter = await app.state().then((s) => s.proxy.running);
    expect(routingAfter).toBe(routingBefore);
  });

  test("events already on screen survive a reconnect", async ({ boot }) => {
    const app = await boot({ securityFeed: { state: "live", events: [blocked] } });
    await expect(feed(app.page).getByText("Blocked")).toBeVisible();

    await app.emit("security-feed-state", "reconnecting");

    // A feed having a bad minute is not an empty feed; blanking the table would
    // lose what the user was reading.
    await expect(feed(app.page).getByText("Blocked")).toBeVisible();
    await expect(feed(app.page).getByText("No security events")).toHaveCount(0);
  });

  test("opening an event offers the dashboard and keeps the summary up", async ({ boot }) => {
    const app = await boot({ securityFeed: { state: "live", events: [blocked] } });
    await feed(app.page).getByRole("button", { name: /View/ }).click();

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
    await expect(feed(app.page).getByText("No security events")).toBeVisible();

    for (let i = 0; i < 205; i++) {
      await app.emit("security-event", {
        ...blocked,
        id: `evt-${i}`,
        requestId: `req-${i}`,
        category: `cat-${i}`,
      });
    }

    await expect.poll(() => feed(app.page).getByText("cat-204").count()).toBe(1);
    // The oldest five fell off rather than accumulating.
    expect(await feed(app.page).getByText("cat-0", { exact: true }).count()).toBe(0);
    expect(await feed(app.page).getByText("cat-5", { exact: true }).count()).toBe(1);
  });

  test("the notification switches reach the backend", async ({ boot }) => {
    // Two switches, not the four AG-594 names: Settings draws one Notifications
    // row (`116:29086`) over blocked, flagged and routing alike, plus the
    // undrawn sound. Each must actually stop something, which is why they waited
    // for a feed that could fire them.
    const app = await boot({});
    await app.page.getByRole("button", { name: "Settings" }).click();

    const notificationsSwitch = app.page.getByRole("switch", { name: "Notifications" });
    await expect(notificationsSwitch).toHaveAttribute("aria-checked", "true");
    await notificationsSwitch.click();
    await expect.poll(() => app.lastCall("set_notifications")).toEqual({ enabled: false });

    await app.page.getByRole("switch", { name: "Notification sound" }).click();
    await expect
      .poll(() => app.lastCall("set_security_notification_sound"))
      .toEqual({ enabled: false });

    // The rows the split used to draw are gone.
    await expect(app.page.getByRole("switch", { name: "Blocked requests" })).toHaveCount(0);
    await expect(app.page.getByRole("switch", { name: "Flagged requests" })).toHaveCount(0);
  });
});
