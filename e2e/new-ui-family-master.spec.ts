import { test, expect } from "./fixtures";

/**
 * The family master switch in the new window UI.
 *
 * What matters here is what the switch refuses to do: it never adopts a config
 * somebody wrote by hand, and it never flips a chat surface. Both are decisions
 * that belong to a single row, not to a switch two levels up.
 */
const useNewUi = { gc: "gc.newUi" };

const anthropicTools = [
  {
    slug: "claude-code",
    name: "Claude Code",
    upstream_provider_name: "Anthropic",
    default_upstream_url: "https://gw.example/claude-code",
    requires_upstream_credential: false,
    status: { kind: "detected" as const },
  },
];

test.describe("new UI family master", () => {
  test.beforeEach(async ({ page }) => {
    await page.addInitScript((k) => localStorage.setItem(k.gc, "1"), useNewUi);
  });

  test("routes every member it governs", async ({ boot }) => {
    const app = await boot({
      proxy: { running: true, ca_trusted: true },
      tools: anthropicTools,
    });

    await app.page.getByRole("button", { name: "Families" }).click();
    await app.page.getByRole("switch", { name: "Route Claude" }).click();

    await expect.poll(() => app.lastCall("connect_tool")).toMatchObject({ slug: "claude-code" });
  });

  test("a drifted member is outside the family switch entirely", async ({ boot }) => {
    // Adopting a hand-written config is the review dialog's decision, not a
    // switch two levels up. `groups.ts` also derives a config member's intent
    // from `connected`, so a drifted member counts as off and the family switch
    // governs nothing here - in either direction. The row and the alert card are
    // the ways back.
    const app = await boot({
      proxy: { running: true, ca_trusted: true },
      tools: [
        {
          ...anthropicTools[0],
          status: { kind: "drifted" as const, reason: "ANTHROPIC_BASE_URL" },
        },
      ],
    });

    await app.page.getByRole("button", { name: "Families" }).click();
    const master = app.page.getByRole("switch", { name: "Route Claude" });
    await expect(master).toHaveAttribute("aria-checked", "false");

    await master.click();

    expect(await app.lastCall("connect_tool")).toBeNull();
    expect(await app.lastCall("disconnect_tool")).toBeNull();
  });

  test("leaves the chat surface to its own row", async ({ boot }) => {
    // claude.ai intercepts a session-cookie surface rather than a key-brokered
    // API, so it must never ride a family switch.
    const app = await boot({
      proxy: { running: true, ca_trusted: true },
      tools: anthropicTools,
    });

    await app.page.getByRole("button", { name: "Families" }).click();
    await app.page.getByRole("switch", { name: "Route Claude" }).click();

    await expect.poll(() => app.lastCall("connect_tool")).not.toBeNull();
    const domainCalls = (await app.calls()).filter((c) => c.cmd === "proxy_set_domain");
    expect(domainCalls.map((c) => c.args.slug)).not.toContain("claude-web");
  });

  test("asks about the certificate once, before any member", async ({ boot }) => {
    const app = await boot({
      proxy: { running: true, ca_trusted: false },
      tools: anthropicTools,
    });

    await app.page.getByRole("button", { name: "Families" }).click();
    await app.page.getByRole("switch", { name: "Route Claude" }).click();

    await expect(
      app.page.getByRole("heading", { name: /Trust the Gate certificate/ }),
    ).toBeVisible();
    expect(await app.lastCall("connect_tool")).toBeNull();

    // Declining aborts the whole family, and says nothing: the user just chose
    // this on our own screen.
    await app.page.getByRole("button", { name: "Not now" }).click();
    await expect(app.page.getByRole("alert")).toHaveCount(0);
    expect(await app.lastCall("connect_tool")).toBeNull();
  });

  test("names the members that failed", async ({ boot }) => {
    const app = await boot({
      proxy: { running: true, ca_trusted: true },
      failures: { connect_tool: "gateway rejected the key" },
      tools: anthropicTools,
    });

    await app.page.getByRole("button", { name: "Families" }).click();
    await app.page.getByRole("switch", { name: "Route Claude" }).click();

    // "Couldn't connect this tool" names nobody; the row that failed should be
    // in the sentence.
    await expect(app.page.getByRole("alert")).toContainText("Claude Code");
  });
});
