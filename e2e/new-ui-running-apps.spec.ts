import { test, expect } from "./fixtures";

/**
 * What happens after a tool's config is rewritten while that tool is running.
 *
 * The sequence only exists because Gate can close an app but cannot reopen it,
 * so what these tests care about is that **nothing is killed without two
 * answers**, and that walking away leaves the saved config alone.
 */
const useNewUi = { gc: "gc.newUi" };

const CLAUDE_CODE = {
  slug: "claude-code",
  name: "Claude Code",
  upstream_provider_name: "Anthropic",
  default_upstream_url: "https://gw.example/claude-code",
  status: { kind: "detected" as const },
};

const CODEX = {
  slug: "codex",
  name: "Codex",
  upstream_provider_name: "OpenAI",
  default_upstream_url: "https://gw.example/codex",
  status: { kind: "detected" as const },
};

test.describe("new UI running apps", () => {
  test.beforeEach(async ({ page }) => {
    await page.addInitScript((k) => localStorage.setItem(k.gc, "1"), useNewUi);
  });

  test("offers to close an app that is running when its config changes", async ({ boot }) => {
    const app = await boot({
      proxy: { running: true, ca_trusted: true },
      tools: [CLAUDE_CODE],
      runningAgentNames: ["claude"],
    });

    await app.page.getByRole("switch", { name: "Claude Code" }).click();

    // The config is already written; this is only about the running process.
    await expect.poll(() => app.calls().then((c) => c.some((x) => x.cmd === "connect_tool"))).toBe(
      true,
    );
    await expect(
      app.page.getByRole("heading", { name: "Apply changes to running apps" }),
    ).toBeVisible();
  });

  test("says nothing when the app is not running", async ({ boot }) => {
    const app = await boot({
      proxy: { running: true, ca_trusted: true },
      tools: [CLAUDE_CODE],
      runningAgentNames: [],
    });

    await app.page.getByRole("switch", { name: "Claude Code" }).click();

    await expect.poll(() => app.calls().then((c) => c.some((x) => x.cmd === "connect_tool"))).toBe(
      true,
    );
    await expect(app.page.getByRole("dialog")).toHaveCount(0);
  });

  test("killing anything takes two answers", async ({ boot }) => {
    const app = await boot({
      proxy: { running: true, ca_trusted: true },
      tools: [CLAUDE_CODE],
      runningAgentNames: ["claude"],
    });

    await app.page.getByRole("switch", { name: "Claude Code" }).click();
    await app.page.getByRole("button", { name: "Yes, close affected apps" }).click();

    // Still nothing closed: this is the confirmation, not the action.
    await expect(app.page.getByRole("heading", { name: "Close affected apps now?" })).toBeVisible();
    expect(await app.lastCall("close_running_agents")).toBeNull();

    await app.page.getByRole("button", { name: /^Yes, close apps$/ }).click();

    await expect.poll(() => app.lastCall("close_running_agents")).not.toBeNull();
    await expect(app.page.getByRole("heading", { name: "Change is ready" })).toBeVisible();
  });

  test("backing out of the confirmation closes nothing", async ({ boot }) => {
    const app = await boot({
      proxy: { running: true, ca_trusted: true },
      tools: [CLAUDE_CODE],
      runningAgentNames: ["claude"],
    });

    await app.page.getByRole("switch", { name: "Claude Code" }).click();
    await app.page.getByRole("button", { name: "Yes, close affected apps" }).click();
    await app.page.getByRole("button", { name: "No, I will close later" }).click();

    await expect(
      app.page.getByRole("heading", { name: "Apply changes to running apps" }),
    ).toBeVisible();
    expect(await app.lastCall("close_running_agents")).toBeNull();
  });

  test("reopening later keeps the config that was just saved", async ({ boot }) => {
    // The write already happened. Declining only decides when the running
    // process picks it up.
    const app = await boot({
      proxy: { running: true, ca_trusted: true },
      tools: [CLAUDE_CODE],
      runningAgentNames: ["claude"],
    });

    await app.page.getByRole("switch", { name: "Claude Code" }).click();
    await app.page.getByRole("button", { name: "No, I will reopen later" }).click();

    await expect(app.page.getByRole("dialog")).toHaveCount(0);
    expect(await app.lastCall("close_running_agents")).toBeNull();
    expect(await app.lastCall("connect_tool")).not.toBeNull();
  });

  test("says nothing about a tool whose config was not touched", async ({ boot }) => {
    // The regression: the probe asked about every tool, so switching Codex on
    // offered to close a running `claude` that nothing had reconfigured - and
    // the confirmation behind that offer would have killed it.
    const app = await boot({
      proxy: { running: true, ca_trusted: true },
      tools: [CLAUDE_CODE, CODEX],
      runningAgentNames: ["claude"],
    });

    // Exact: "Codex" is a substring of the ChatGPT domain rows' labels too, and
    // those route through the proxy rather than a config file.
    await app.page.getByRole("switch", { name: "Codex", exact: true }).click();

    await expect
      .poll(() => app.calls().then((c) => c.some((x) => x.cmd === "connect_tool")))
      .toBe(true);
    await expect(app.page.getByRole("dialog")).toHaveCount(0);
  });

  test("offers only the app that was reconfigured", async ({ boot }) => {
    const app = await boot({
      proxy: { running: true, ca_trusted: true },
      tools: [CLAUDE_CODE, CODEX],
      runningAgentNames: ["claude", "codex"],
    });

    await app.page.getByRole("switch", { name: "Codex", exact: true }).click();

    const dialog = app.page.getByRole("dialog");
    await expect(dialog).toContainText("codex");
    await expect(dialog).not.toContainText("claude");
  });

  test("a declined review never reaches the sequence", async ({ boot }) => {
    // Nothing was written, so there is nothing for a running app to pick up.
    const app = await boot({
      proxy: { running: true, ca_trusted: true },
      runningAgentNames: ["codex"],
      tools: [
        {
          slug: "codex",
          name: "Codex",
          upstream_provider_name: "OpenAI",
          default_upstream_url: "https://gw.example/codex",
          status: { kind: "drifted" as const, reason: "API base URL: https://api.openai.com/v1" },
        },
      ],
    });

    await app.page.getByRole("switch", { name: "Codex" }).last().click();
    await app.page.getByRole("button", { name: "Keep existing config" }).click();

    await expect(app.page.getByRole("dialog")).toHaveCount(0);
    expect(await app.lastCall("connect_tool")).toBeNull();
  });
});
