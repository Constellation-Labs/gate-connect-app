import { test, expect } from "./fixtures";

/** Events pushed from the backend. The popover webview outlives every tray
 *  hide/show, so these are the only way state that changed while the window
 *  was closed - a token that expired, an engine that moved -
 *  ever reaches the screen. Nothing below is reachable from a unit test:
 *  each one starts in Rust and ends in a repaint. */
test.describe("backend events", () => {
  test("proxy-state-changed repaints Home from the engine, not from a click", async ({ boot }) => {
    const app = await boot();
    await expect(app.page.getByText("Didn’t start")).toBeVisible();

    // The CLI (or the helper daemon) turned routing on behind the popover's
    // back; the engine announces it.
    await app.patch({ proxy: { running: true, port: 8899, pac_port: 8898, ca_trusted: true } });
    await app.emit("proxy-state-changed");

    await expect(app.page.getByText(/^On( ·|$)/)).toBeVisible();
  });

  test("backend-error-pending drains the buffered failures", async ({ boot }) => {
    const app = await boot();

    await app.emit("backend-error-pending");

    // Swept once at mount and again on each nudge - two calls, not one.
    await expect
      .poll(async () => (await app.calls()).filter((c) => c.cmd === "drain_backend_errors").length)
      .toBeGreaterThan(1);
  });

  test("a session that died while the popover was closed drops to re-sign-in", async ({ boot }) => {
    const app = await boot();
    await expect(app.page.getByRole("heading", { name: "Routing" })).toBeVisible();

    // Refresh token revoked while the window was hidden: the tray reopens the
    // popover, focus returns, and the stale Home must not survive it.
    await app.patch({ oauth: { signed_in: false, email: null, expires_at_unix: 0 } });
    await app.emit("tauri://focus", true);

    await expect(app.page.getByRole("heading", { name: "Welcome back" })).toBeVisible();
    await expect(app.page.getByText(/session expired/i)).toBeVisible();
  });

  test("a key account is not dropped on focus - it has no session to expire", async ({ boot }) => {
    const app = await boot({
      account: { auth_mode: "api_key", has_api_key: true, org_id: null, org_name: null },
      oauth: { signed_in: false, email: null, expires_at_unix: 0 },
    });

    await app.emit("tauri://focus", true);

    await expect(app.page.getByRole("heading", { name: "Routing" })).toBeVisible();
  });
});
