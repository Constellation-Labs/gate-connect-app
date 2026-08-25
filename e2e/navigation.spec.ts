import { test, expect } from "./fixtures";

/** Moving around the one room. The popover is 360x520 with a header and a
 *  footer that never scroll, so "can you get there and back" is a real
 *  question here in a way it isn't in a full-page app. */
test.describe("navigation", () => {
  test("the gear opens Settings and Back returns to Home", async ({ boot }) => {
    const app = await boot();

    await app.openSettings();
    await expect(app.page.getByRole("heading", { name: "Settings" })).toBeVisible();

    await app.page.getByRole("button", { name: "Back" }).click();
    await expect(app.page.getByRole("heading", { name: "Routing" })).toBeVisible();
  });

  test("Escape steps back out of Settings", async ({ boot }) => {
    const app = await boot();

    await app.openSettings();
    await app.page.keyboard.press("Escape");

    await expect(app.page.getByRole("heading", { name: "Routing" })).toBeVisible();
  });

  test("Escape steps back out of a family panel", async ({ boot }) => {
    const app = await boot();

    await app.familyRow("Claude").click();
    await expect(app.page.getByRole("switch", { name: /Route Claude Code/ })).toBeVisible();

    await app.page.keyboard.press("Escape");
    await expect(app.page.getByRole("heading", { name: "Routing" })).toBeVisible();
  });

  test("a takeover owns Escape - it cancels the quit, it doesn't navigate", async ({ boot }) => {
    const app = await boot({ pendingQuitTools: ["Claude Code"] });
    await expect(app.page.getByRole("heading", { name: "Quit Gate Connect?" })).toBeVisible();

    await app.page.keyboard.press("Escape");

    await expect(app.page.getByRole("heading", { name: "Quit Gate Connect?" })).toHaveCount(0);
    await expect(app.page.getByRole("heading", { name: "Routing" })).toBeVisible();
    expect((await app.calls()).some((c) => c.cmd === "quit_app")).toBe(false);
  });

  test("the header and the credential footer stay put while the body scrolls", async ({ boot }) => {
    // Enough families to overflow 520px.
    const app = await boot({
      tools: [
        {
          slug: "claude-code",
          name: "Claude Code",
          upstream_provider_name: "Anthropic",
          default_upstream_url: "https://api.anthropic.com",
          status: { kind: "detected" },
        },
        {
          slug: "codex",
          name: "Codex",
          upstream_provider_name: "OpenAI",
          default_upstream_url: "https://api.openai.com/v1",
          status: { kind: "detected" },
        },
        {
          slug: "opencode",
          name: "OpenCode",
          upstream_provider_name: "your existing providers",
          default_upstream_url: "https://api.anthropic.com",
          status: { kind: "detected" },
        },
      ],
    });

    const header = app.page.getByRole("heading", { name: "Gate Connect" });
    // PRODUCT.md's first principle, pinned to every screen.
    const footer = app.page.getByText(/in your keychain/i);
    await expect(header).toBeVisible();
    await expect(footer).toBeVisible();

    await app.page.locator(".gc-scroll").evaluate((el) => el.scrollTo(0, el.scrollHeight));

    await expect(header).toBeVisible();
    await expect(footer).toBeVisible();
  });
});

/** Settings' own controls, which reach commands nothing else does. */
test.describe("settings", () => {
  test("launch at login is its own setting, independent of routing", async ({ boot }) => {
    const app = await boot();

    await app.openSettings();
    const toggle = app.page.getByRole("switch", { name: "Launch at login" });
    await expect(toggle).toHaveAttribute("aria-checked", "false");

    await toggle.click();

    await expect.poll(() => app.lastCall("set_launch_at_login")).toEqual({ enabled: true });
    expect((await app.state()).launchAtLogin.enabled).toBe(true);
    // Routing is untouched: turning one on has never implied the other.
    expect((await app.state()).proxy.running).toBe(false);
  });

  test("the gateway host is shown in full", async ({ boot }) => {
    const app = await boot();

    await app.openSettings();

    // The one identifier that answers "am I on production or staging?" - it
    // used to truncate to "gateway.constellationga…".
    await expect(app.page.getByText("gateway.constellationgate.ai", { exact: true })).toBeVisible();
  });
});
