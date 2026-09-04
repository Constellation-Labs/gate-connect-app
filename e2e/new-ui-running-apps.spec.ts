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
  // The surface, not the product: `integrations/claude_code.rs`. The rail's
  // eyebrow is what says "Anthropic", which is why the switch names below
  // carry it.
  name: "CLI",
  upstream_provider_name: "Anthropic",
  default_upstream_url: "https://gw.example/claude-code",
  status: { kind: "detected" as const },
};

const CODEX = {
  slug: "codex",
  name: "CLI",
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

    await app.page.getByRole("switch", { name: "Anthropic CLI" }).click();

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

    await app.page.getByRole("switch", { name: "Anthropic CLI" }).click();

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

    await app.page.getByRole("switch", { name: "Anthropic CLI" }).click();
    await app.page.getByRole("button", { name: "Yes, close affected apps" }).click();

    // Still nothing closed: this is the confirmation, not the action.
    await expect(app.page.getByRole("heading", { name: "Close affected apps now?" })).toBeVisible();
    expect(await app.lastCall("close_running_agents")).toBeNull();

    await app.page.getByRole("button", { name: /^Yes, close apps$/ }).click();

    await expect.poll(() => app.lastCall("close_running_agents")).not.toBeNull();
    // Closed is not applied: Gate cannot reopen a terminal tool, so the account
    // says whose move it is rather than claiming the route is live.
    await expect(app.page.getByRole("heading", { name: "What happened" })).toBeVisible();
    await expect(app.page.getByRole("dialog")).toContainText("Waiting for you to reopen");
  });

  test("backing out of the confirmation closes nothing", async ({ boot }) => {
    const app = await boot({
      proxy: { running: true, ca_trusted: true },
      tools: [CLAUDE_CODE],
      runningAgentNames: ["claude"],
    });

    await app.page.getByRole("switch", { name: "Anthropic CLI" }).click();
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

    await app.page.getByRole("switch", { name: "Anthropic CLI" }).click();
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

    await app.page.getByRole("switch", { name: "OpenAI CLI" }).click();

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

    await app.page.getByRole("switch", { name: "OpenAI CLI" }).click();

    // The product name, not the rail's row label: both tools are a "CLI" there,
    // and a flat list of two CLIs names neither.
    const dialog = app.page.getByRole("dialog");
    await expect(dialog).toContainText("Codex");
    await expect(dialog).not.toContainText("Claude Code");
  });

  test("names the route in use and the route asked for", async ({ boot }) => {
    // "Reopen required" without the pair does not say what reopening would
    // change, which is the whole reason this step exists rather than a sentence.
    const app = await boot({
      proxy: { running: true, ca_trusted: true },
      tools: [CODEX],
      runningAgentNames: ["codex"],
      staleAgents: 1,
    });

    await app.page.getByRole("switch", { name: "OpenAI CLI" }).click();

    const dialog = app.page.getByRole("dialog");
    await expect(dialog).toContainText("In use:");
    await expect(dialog).toContainText("Requested:");
    // Who reopens it, read off the backend rather than written into the copy.
    await expect(dialog).toContainText("you reopen it");
  });

  test("the confirmation asks for a save it cannot check itself", async ({ boot }) => {
    const app = await boot({
      proxy: { running: true, ca_trusted: true },
      tools: [CODEX],
      runningAgentNames: ["codex"],
    });

    await app.page.getByRole("switch", { name: "OpenAI CLI" }).click();
    await app.page.getByRole("button", { name: "Yes, close affected apps" }).click();

    const dialog = app.page.getByRole("dialog");
    await expect(dialog).toContainText("Save your work before continuing");
    await expect(dialog).toContainText("cannot tell whether");
    await expect(dialog).toContainText("You reopen Codex yourself");
  });

  test("a tool that comes back is verified before it reads as routing", async ({ boot }) => {
    // AG-566 AC 8: it is the reopen that gets checked. The tool is closed, then
    // launched again, and only then does the account call it applied.
    const app = await boot({
      proxy: { running: true, ca_trusted: true },
      tools: [CODEX],
      runningAgentNames: ["codex"],
    });

    await app.page.getByRole("switch", { name: "OpenAI CLI" }).click();
    await app.page.getByRole("button", { name: "Yes, close affected apps" }).click();
    await app.page.getByRole("button", { name: /^Yes, close apps$/ }).click();
    await expect(app.page.getByRole("dialog")).toContainText("Waiting for you to reopen");

    // The user opens it again.
    await app.patch({ runningAgentNames: ["codex"] });

    await expect(app.page.getByRole("heading", { name: "Change is ready" })).toBeVisible({
      timeout: 15_000,
    });
  });

  /**
   * AG-566 AC 3: the invitation belongs on Overview too, not only on the pane
   * of the tool it is about. The pane's own card covers that tool, so the
   * banner names the ones whose panes are not open.
   */
  test("Overview offers the reopen without opening the tool", async ({ boot }) => {
    const app = await boot({
      proxy: { running: true, ca_trusted: true },
      tools: [{ ...CODEX, status: { kind: "connected" as const } }],
      staleAgents: 1,
      runningAgentNames: ["codex"],
    });

    const banner = app.page.getByRole("status").filter({ hasText: "Reopen to finish" });
    await expect(banner).toBeVisible();
    await expect(banner).toContainText("Codex");

    await banner.getByRole("button", { name: "Reopen tool" }).click();

    await expect(
      app.page.getByRole("heading", { name: "Apply changes to running apps" }),
    ).toBeVisible();
    await expect.poll(() => app.lastCall("running_agents")).toMatchObject({
      only: ["codex"],
    });
  });

  /**
   * The other half of "a running tool keeps its old route", for the surfaces
   * Gate routes through the system proxy rather than through a config file.
   *
   * Linux only, and that is the substance: Windows refreshes WinINET after its
   * registry write and macOS's auto-proxy URL applies to new connections, so on
   * those two the line would be wrong rather than merely cautious. Gate cannot
   * see these apps at all, so it is drawn as advice and says so.
   */
  test("a proxy-routed row warns about open apps on Linux, and only there", async ({
    boot,
  }) => {
    const app = await boot({
      platform: "linux",
      proxy: { running: true, ca_trusted: true },
    });

    // The Anthropic family's "App" row: Claude Desktop and Cowork, routed by
    // domain rather than by a config file of their own.
    await app.page.getByRole("button", { name: "App" }).first().click();

    await expect(
      app.page.getByText("Apps already open may need reopening"),
    ).toBeVisible();
    await expect(app.page.getByText(/advice rather than a reading/)).toBeVisible();
  });

  for (const platform of ["macos", "windows"] as const) {
    test(`${platform} says nothing about reopening a proxy-routed app`, async ({
      boot,
    }) => {
      const app = await boot({
        platform,
        proxy: { running: true, ca_trusted: true },
      });

      await app.page.getByRole("button", { name: "App" }).first().click();

      await expect(
        app.page.getByText("Apps already open may need reopening"),
      ).toHaveCount(0);
    });
  }

  test("a config-routed tool gets the measured verdict, not the advice", async ({
    boot,
  }) => {
    // It has a file to re-read and a sweep that says whether it did. Advice
    // beside a reading would invite the reader to weigh a guess against a
    // measurement.
    const app = await boot({
      platform: "linux",
      proxy: { running: true, ca_trusted: true },
      tools: [{ ...CODEX, status: { kind: "connected" as const } }],
      staleAgents: 1,
      runningAgentNames: ["codex"],
    });

    await app.page.getByRole("button", { name: "CLI" }).first().click();

    await expect(app.page.getByText(/Reopen .* to finish/)).toBeVisible();
    await expect(
      app.page.getByText("Apps already open may need reopening"),
    ).toHaveCount(0);
  });

  test("a declined review never reaches the sequence", async ({ boot }) => {
    // Nothing was written, so there is nothing for a running app to pick up.
    const app = await boot({
      proxy: { running: true, ca_trusted: true },
      runningAgentNames: ["codex"],
      tools: [
        {
          slug: "codex",
          name: "CLI",
          upstream_provider_name: "OpenAI",
          default_upstream_url: "https://gw.example/codex",
          status: { kind: "drifted" as const, reason: "API base URL: https://api.openai.com/v1" },
        },
      ],
    });

    // The Overview's drift notice, not the rail row. A drifted row renders on -
    // that is the user's intent, which drift does not revoke - so its switch
    // asks to turn routing OFF and never reaches the gate. Reconnecting is what
    // asks to write the config, and the gate is on that path.
    await app.page.getByRole("switch", { name: "Let Gate Connect manage CLI" }).click();
    await app.page.getByRole("button", { name: "Keep existing config" }).click();

    await expect(app.page.getByRole("dialog")).toHaveCount(0);
    expect(await app.lastCall("connect_tool")).toBeNull();
  });
});
