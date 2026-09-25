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

/**
 * The pane's reopen card. `ReopenAlert` takes `role="status"` - it is raised by
 * a sweep rather than by a click - and it is the only status region inside the
 * pane, so the role scopes a click to the card rather than to whatever else
 * happens to hold a "Close tool" button.
 */
function reopenCard(app: { page: import("@playwright/test").Page }) {
  return app.page.getByRole("status").filter({ hasText: /^Reopen .+ to finish/ });
}

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

    await app.routeApp("Claude");

    // The config is already written; this is only about the running process.
    await expect.poll(() => app.calls().then((c) => c.some((x) => x.cmd === "connect_tool"))).toBe(
      true,
    );
    await expect(
      app.page.getByRole("heading", { name: "Apply changes to running apps" }),
    ).toBeVisible();
  });

  /**
   * AG-900: closing Claude left the Claude desktop app running while the dialog
   * reported it closed.
   *
   * The Claude switch is a section - `claude-code` + `anthropic` +
   * `claude-web` - and the offer was built from `moved.filter(m => m.kind ===
   * "config")`, so only the CLI's row survived. The desktop app is routed
   * through the system proxy rather than by a config write, and it resolves
   * that proxy at its own launch, which makes it exactly as stale after the
   * switch as the CLI is. The entry's own copy promises to cover it: "Claude
   * Code in your terminal, and the Claude desktop app".
   */
  test("offers the desktop app too, not only the CLI beside it", async ({ boot }) => {
    const app = await boot({
      proxy: { running: true, ca_trusted: true },
      tools: [CLAUDE_CODE],
      // `Claude` is the desktop app and `claude` is the CLI. The case is the
      // whole difference, in the harness as in `agent_name_of`.
      runningAgentNames: ["claude", "Claude"],
    });

    await app.routeApp("Claude");

    const dialog = app.page.getByRole("dialog");
    await expect(
      app.page.getByRole("heading", { name: "Apply changes to running apps" }),
    ).toBeVisible();
    await expect(dialog.getByText("Claude Code").first()).toBeVisible();
    await expect(dialog.getByText("Claude Desktop").first()).toBeVisible();
    // And the dialog draws the difference between the two, which is the reason
    // the desktop app belongs here rather than being quietly left running: Gate
    // can put it back, and cannot put a shell session back.
    await expect(
      dialog.getByText("Gate Connect will reopen Claude Desktop"),
    ).toBeVisible();

    // The scan has to have been asked about it, which is the half a rendered
    // row cannot prove: the slug reaches Rust, where `agent_names_for` turns it
    // into the `Claude` process name.
    await expect
      .poll(async () => ((await app.lastCall("running_agents"))?.only as string[]) ?? [])
      .toContain("anthropic");
  });

  test("says nothing when the app is not running", async ({ boot }) => {
    const app = await boot({
      proxy: { running: true, ca_trusted: true },
      tools: [CLAUDE_CODE],
      runningAgentNames: [],
    });

    await app.routeApp("Claude");

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

    await app.routeApp("Claude");
    await app.page.getByRole("button", { name: "Yes, close affected apps" }).click();

    // Still nothing closed: this is the confirmation, not the action.
    await expect(app.page.getByRole("heading", { name: "Close affected apps now?" })).toBeVisible();
    expect(await app.lastCall("close_running_agents")).toBeNull();

    await app.page.getByRole("button", { name: /^Yes, close apps$/ }).click();

    await expect.poll(() => app.lastCall("close_running_agents")).not.toBeNull();
    // Closed is not applied: Gate cannot reopen a terminal tool, so there is
    // no all-clear to show. The flow ends and the rail carries the outcome.
    await expect(app.page.getByRole("dialog")).toHaveCount(0);
    await expect(app.page.getByRole("heading", { name: "Change is ready" })).toHaveCount(0);
  });

  test("backing out of the confirmation closes nothing", async ({ boot }) => {
    const app = await boot({
      proxy: { running: true, ca_trusted: true },
      tools: [CLAUDE_CODE],
      runningAgentNames: ["claude"],
    });

    await app.routeApp("Claude");
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

    await app.routeApp("Claude");
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

    await app.routeApp("ChatGPT / Codex");

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

    await app.routeApp("ChatGPT / Codex");

    // The product name, not the section's name: the dialog lists tools, and
    // "ChatGPT / Codex" is the row they sit under rather than a program anyone
    // can close.
    const dialog = app.page.getByRole("dialog");
    await expect(dialog).toContainText("Codex");
    await expect(dialog).not.toContainText("Claude Code");
  });

  test("names the running tool and who reopens it", async ({ boot }) => {
    // The route pair used to be asserted here. It is development-only now, and
    // Playwright serves the dev server (`playwright.config.ts`), so those two
    // lines would have stayed green forever while proving nothing about a
    // release build - a test named for AC 1 asserting AC 1 after AC 1 stopped
    // shipping. Both sides of the build split are covered properly in
    // `dialogs.apply.test.tsx`, which stubs the flag; what is left here is what
    // the frame actually draws.
    const app = await boot({
      proxy: { running: true, ca_trusted: true },
      tools: [CODEX],
      runningAgentNames: ["codex"],
      staleAgents: 1,
    });

    await app.routeApp("ChatGPT / Codex");

    const dialog = app.page.getByRole("dialog");
    // Who reopens it, read off the backend rather than written into the copy -
    // and said ONCE, in the note, rather than repeated on every row. The rows
    // carried their own copy of this until the frame (`130:58427`) settled that
    // they draw a name, a description and a pill and nothing else.
    //
    // One assertion, not two: a separate `toContainText("Codex")` above this
    // proved nothing the sentence does not already contain.
    await expect(dialog).toContainText("reopen Codex yourself");
  });

  test("the confirmation asks for a save it cannot check itself", async ({ boot }) => {
    const app = await boot({
      proxy: { running: true, ca_trusted: true },
      tools: [CODEX],
      runningAgentNames: ["codex"],
    });

    await app.routeApp("ChatGPT / Codex");
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

    await app.routeApp("ChatGPT / Codex");
    await app.page.getByRole("button", { name: "Yes, close affected apps" }).click();
    await app.page.getByRole("button", { name: /^Yes, close apps$/ }).click();
    // Nothing on screen while Gate waits for the user: only the all-clear is
    // drawn for this stage.
    await expect.poll(() => app.lastCall("close_running_agents")).not.toBeNull();
    await expect(app.page.getByRole("dialog")).toHaveCount(0);

    // The user opens it again.
    await app.patch({ runningAgentNames: ["codex"] });

    await expect(app.page.getByRole("heading", { name: "Change is ready" })).toBeVisible({
      timeout: 15_000,
    });
  });

  /**
   * The invitation is the tool's, so it is drawn on the tool's pane and nowhere
   * else. AG-566 AC 3 asked for it on Overview as well and a shell banner did
   * that for a while; one tool's pending reopen in shell chrome then stood over
   * Overview, Settings and every other tool's pane. The rail row still names
   * the tool, which is what gets the user here.
   */
  test("the tool's pane offers the reopen, and Overview does not", async ({ boot }) => {
    const app = await boot({
      proxy: { running: true, ca_trusted: true },
      tools: [{ ...CODEX, status: { kind: "connected" as const } }],
      staleAgents: 1,
      runningAgentNames: ["codex"],
    });

    // Positive first, and that ordering is the assertion. `boot` waits only for
    // the first heading while `refreshVerdicts` is still in flight, so a bare
    // count of zero passes against a page that has not heard about the reopen
    // yet - which would let the banner come back unnoticed. The pane's card is
    // what proves the sweep landed; the rail row reads a bare "Not protected"
    // both before and after it.
    await app.openSection("ChatGPT / Codex");
    const card = reopenCard(app);
    await expect(card).toBeVisible();

    // And with the fact known, nothing on Overview offers the action.
    await app.page.getByRole("button", { name: "Overview" }).click();
    await expect(app.page.getByRole("button", { name: "Close tool" })).toHaveCount(0);

    await app.openSection("ChatGPT / Codex");
    await expect(card).toBeVisible();

    await card.getByRole("button", { name: "Close tool" }).click();

    await expect(
      app.page.getByRole("heading", { name: "Apply changes to running apps" }),
    ).toBeVisible();
    await expect.poll(() => app.lastCall("running_agents")).toMatchObject({
      only: ["codex"],
    });
  });

  /**
   * The reopen is what resolves this, and nothing tells the app it happened.
   *
   * `tool_watch.rs` watches config files and binaries - it has no view of the
   * process table - so replacing the old 5s poll with `tools-changed` left the
   * *process* half of detection with no event behind it. The banner then stood
   * there naming a tool the user had already reopened, and because the shell
   * never re-swept, it stood there for the rest of the session.
   */
  test("clears the invitation on its own once the tool has been reopened", async ({
    boot,
  }) => {
    const app = await boot({
      proxy: { running: true, ca_trusted: true },
      tools: [{ ...CODEX, status: { kind: "connected" as const } }],
      staleAgents: 1,
      runningAgentNames: ["codex"],
    });

    await app.openSection("ChatGPT / Codex");
    const card = reopenCard(app);
    await expect(card).toBeVisible();

    // The user opens it again, somewhere the app cannot see. No click, no
    // event, no visibility change - the standing sweep is the only thing that
    // can notice.
    await app.patch({ staleAgents: 0 });

    await expect(card).toBeHidden({ timeout: 25_000 });
    // And the rail row it was about reads as routing, off the same sweep.
    // Read off the row rather than off the page: the pane this test now opens
    // says "Protected" in its own header too, and the sidebar's eyebrow reads
    // "Protected apps".
    await expect(
      app.page.getByRole("button", { name: "ChatGPT / Codex Protected" }),
    ).toBeVisible({ timeout: 25_000 });
  });

  /**
   * The button on a reading that has gone stale. It cannot open the offer -
   * there is no process to offer anything about - and returning in silence is
   * what made it look broken. The answer is that the invitation was wrong.
   */
  test("a stale invitation takes itself down rather than doing nothing", async ({
    boot,
  }) => {
    const app = await boot({
      proxy: { running: true, ca_trusted: true },
      tools: [{ ...CODEX, status: { kind: "connected" as const } }],
      staleAgents: 1,
      runningAgentNames: ["codex"],
    });

    await app.openSection("ChatGPT / Codex");
    const card = reopenCard(app);
    await expect(card).toBeVisible();

    // Quit between the sweep that raised the card and the press.
    await app.patch({ staleAgents: 0, runningAgentNames: [] });
    await card.getByRole("button", { name: "Close tool" }).click();

    // No dialog about a tool that is not running...
    await expect(
      app.page.getByRole("heading", { name: "Apply changes to running apps" }),
    ).toBeHidden();
    // ...and the card goes, because the empty scan is the answer to it.
    await expect(card).toBeHidden({ timeout: 25_000 });
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
  
  for (const platform of ["macos", "windows"] as const) {
    test(`${platform} says nothing about reopening a proxy-routed app`, async ({
      boot,
    }) => {
      const app = await boot({
        platform,
        proxy: { running: true, ca_trusted: true },
      });

      await app.page.getByRole("button", { name: "Claude" }).first().click();
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

    // Codex's row is the ChatGPT / Codex app row: one switch, and the config
    // tool inside it is what a reopen is about.
    await app.openSection("ChatGPT / Codex");

    await expect(reopenCard(app)).toBeVisible();
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

    // The pane's drift notice, not the rail row. A drifted row renders on -
    // that is the user's intent, which drift does not revoke - so its switch
    // asks to turn routing OFF and never reaches the gate. Reconnecting is what
    // asks to write the config, and the gate is on that path. The card is drawn
    // on Codex's own pane, reached through its section's rail row.
    await app.openSection("ChatGPT / Codex");
    await app.page.getByRole("switch", { name: "Let Gate Connect manage CLI" }).click();
    await app.page.getByRole("button", { name: "Keep existing config" }).click();

    await expect(app.page.getByRole("dialog")).toHaveCount(0);
    expect(await app.lastCall("connect_tool")).toBeNull();
  });
});
