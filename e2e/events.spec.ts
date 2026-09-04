import { test, expect } from "./fixtures";

/** Events pushed from the backend. The popover webview outlives every tray
 *  hide/show, so these are the only way state that changed while the window
 *  was closed - a quit request, a token that expired, an engine that moved -
 *  ever reaches the screen. Nothing below is reachable from a unit test:
 *  each one starts in Rust and ends in a repaint. */
test.describe("backend events", () => {
  test("a deferred quit surfaces the takeover, and disconnecting reverts the tools", async ({
    boot,
  }) => {
    // Buffered by the tray before the listener existed: App sweeps once at
    // mount, so a Quit clicked before the webview was ready isn't lost.
    const app = await boot({ pendingQuitTools: ["Claude Code", "Codex"] });

    await expect(app.page.getByRole("heading", { name: "Quit Gate Connect?" })).toBeVisible();
    await expect(app.page.getByText(/Claude Code and Codex still route/)).toBeVisible();

    await app.page.getByRole("button", { name: "Disconnect tools and quit" }).click();

    await expect
      .poll(async () => {
        const cmds = (await app.calls()).map((c) => c.cmd);
        return cmds.includes("disconnect_tools_for_quit") && cmds.includes("quit_app");
      })
      .toBe(true);
  });

  test("a quit-requested nudge raises the takeover mid-session", async ({ boot }) => {
    const app = await boot();
    await expect(app.page.getByRole("heading", { name: "Routing" })).toBeVisible();

    await app.patch({ pendingQuitTools: ["Claude Code"] });
    await app.emit("quit-requested");

    await expect(app.page.getByRole("heading", { name: "Quit Gate Connect?" })).toBeVisible();
    // Cancel is the focused control: Enter on an unread panel must not decide
    // how to quit.
    await app.page.getByRole("button", { name: "Cancel" }).click();
    await expect(app.page.getByRole("heading", { name: "Quit Gate Connect?" })).toHaveCount(0);
    expect((await app.calls()).some((c) => c.cmd === "quit_app")).toBe(false);
  });

  test("a failed disconnect keeps the takeover up instead of quitting", async ({ boot }) => {
    const app = await boot({
      pendingQuitTools: ["Claude Code"],
      failures: { disconnect_tools_for_quit: "failed to restore ~/.codex/config.toml" },
    });

    await app.page.getByRole("button", { name: "Disconnect tools and quit" }).click();

    await expect(app.page.getByText(/couldn|failed|restore/i).first()).toBeVisible();
    // Quitting with tool configs half-reverted is the one outcome this panel
    // exists to prevent.
    expect((await app.calls()).some((c) => c.cmd === "quit_app")).toBe(false);
    await expect(app.page.getByRole("heading", { name: "Quit Gate Connect?" })).toBeVisible();
  });

  test("proxy-state-changed repaints Home from the engine, not from a click", async ({ boot }) => {
    const app = await boot();
    await expect(app.routingSwitch).toHaveAttribute("aria-checked", "false");

    // The CLI (or the helper daemon) turned routing on behind the popover's
    // back; the engine announces it.
    await app.patch({ proxy: { running: true, port: 8899, pac_port: 8898, ca_trusted: true } });
    await app.emit("proxy-state-changed");

    await expect(app.routingSwitch).toHaveAttribute("aria-checked", "true");
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
