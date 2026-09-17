import { test, expect } from "./fixtures";

/** The master switch and the per-member switches behind it. The invariant
 *  worth an e2e: what the UI *does* to the backend after a click, and what it
 *  re-reads before repainting - App re-syncs from `list_tools` / `proxy_status`
 *  rather than trusting the command's return value. */
test.describe("routing", () => {
  /*
   * The master switch's own tests lived here and are gone with it: nine of
   * them, all clicking a control neither shell draws now. What they covered and
   * where it went, because "deleted with the switch" is only a good answer if
   * it is true:
   *
   * - The certificate pre-flight and its ordering - trust before the engine is
   *   asked for anything, the popover pinned while the OS dialog is up, Not now
   *   abandoning it - is still reached through the setup screen's "turn routing
   *   on", which `signin.spec.ts` drives, and through the routing notice's
   *   remedy. Both call `toggleProxy`, which is what those tests were about.
   *   The per-member pre-flight is its own path and is covered below.
   * - The close-running-agents takeover is covered nowhere, because it no
   *   longer exists. The master switch was the only caller that passed
   *   `takeover`, and the arm went with it; the two survivors always took the
   *   inline-hint arm.
   */

  test("an untrusted certificate is fixed from the card, not from the row", async ({ boot }) => {
    const app = await boot({
      proxy: {
        running: true,
        port: 8899,
        pac_port: 8898,
        ca_trusted: false,
        env_export_opted_in: false,
        env_export_separable: true,
        domains: [
          {
            slug: "anthropic",
            display_name: "API",
            client: "claude-desktop",
            credential: "brokered",
            scope: "host",
            hosts: ["api.anthropic.com"],
            upstream_url: "https://gateway.constellationgate.ai",
            rewrite_prefixes: ["/v1"],
            passthrough_prefixes: [],
            enabled: true,
            supported: true,
          },
        ],
      },
    });

    // An enabled domain behind an untrusted certificate is not carrying
    // traffic, and the header says so rather than claiming routing is on.
    await expect(app.page.getByText("Needs trust").first()).toBeVisible();
    // The remedy belongs to the card, so the row must not repeat it.
    await expect(app.page.getByText("certificate not trusted")).toHaveCount(0);

    await app.page.getByRole("button", { name: "Trust", exact: true }).click();

    await expect.poll(async () => (await app.state()).proxy.ca_trusted).toBe(true);
    await expect(app.page.getByText("Routing on").first()).toBeVisible();
  });
});

/** A family panel, opened from Home's ledger. */
test.describe("family panel", () => {
  test("a member switch connects that tool and Home repaints", async ({ boot }) => {
    const app = await boot({
      proxy: { running: true, port: 8899, pac_port: 8898, ca_trusted: true },
    });

    await app.familyRow("Claude").click();
    const member = app.page.getByRole("switch", { name: "Route CLI through Gate" });
    await expect(member).toHaveAttribute("aria-checked", "false");

    await member.click();

    await expect(member).toHaveAttribute("aria-checked", "true");
    // The tool's own default upstream, taken from the catalog rather than
    // hardcoded by the frontend.
    expect(await app.lastCall("connect_tool")).toEqual({
      slug: "claude-code",
      upstreamUrl: "https://api.anthropic.com",
    });
    expect((await app.state()).tools.find((t) => t.slug === "claude-code")?.status).toEqual({
      kind: "connected",
    });

    await app.page.getByRole("button", { name: "Back" }).click();
    await expect(app.page.getByText(/1 of \d+ routing/).first()).toBeVisible();
  });

  test("a proxy member goes through proxy_set_domain, not connect_tool", async ({ boot }) => {
    const app = await boot({
      proxy: { running: true, port: 8899, pac_port: 8898, ca_trusted: true },
    });

    await app.familyRow("Claude").click();
    await app.page.getByRole("switch", { name: "Route API through Gate" }).click();

    await expect.poll(() => app.lastCall("proxy_set_domain")).toEqual({
      slug: "anthropic",
      enabled: true,
    });
    // The UI drives members one at a time and must never reach for the
    // provider shortcut - the ledger groups by client, and `provider_enable`
    // is a vendor-shaped command the renderer has no handle on.
    expect(await app.lastCall("provider_enable")).toBeNull();
  });

  test("the subscription surface has its own switch, and the family switch never touches it", async ({
    boot,
  }) => {
    // chatgpt.com's Responses endpoint is reached with the user's ChatGPT
    // subscription bearer rather than a brokered key, so this shell's group
    // switch leaves it where it is. `cascadeTargets` reaches such a row only
    // for a caller that passes `sessions`, meaning it has asked - which the
    // window shell's app switch does, after a confirmation, and this popover
    // does not, having no dialog to ask with. `provider::cascade_domains`
    // refuses them in Rust either way.
    const app = await boot({
      proxy: { running: true, port: 8899, pac_port: 8898, ca_trusted: true },
    });

    await app.familyRow("ChatGPT / Codex").click();
    const subscription = app.page.getByRole("switch", {
      name: "Route Subscription through Gate",
    });
    const apps = app.page.getByRole("switch", { name: "Route Chat through Gate" });
    await expect(subscription).toHaveAttribute("aria-checked", "false");
    await expect(apps).toHaveAttribute("aria-checked", "false");

    // The whole group on: neither of these rows moves, and nothing else in this
    // section is brokered, so the switch reaches nothing at all here.
    const family = app.page.getByRole("switch", { name: "Route ChatGPT / Codex through Gate" });
    await family.click();
    // The family switch reads its own state back from `proxy_status`, which
    // `setGroupRouted` calls after the member loop - so "checked" is the point
    // where every member has been attempted and the call log below is final.
    // Waiting on the domain state alone would not be: a regression's stray
    // call can arrive after the cascaded one has already landed.
    await expect(family).toHaveAttribute("aria-checked", "true");
    expect(
      (await app.calls()).filter((c) => c.cmd === "proxy_set_domain").map((c) => c.args.slug),
    ).toEqual([]);
    const domains = (await app.state()).proxy.domains;
    expect(domains.find((d) => d.slug === "chatgpt")?.enabled).toBe(false);
    expect(domains.find((d) => d.slug === "chatgpt-apps")?.enabled).toBe(false);
    await expect(subscription).toHaveAttribute("aria-checked", "false");
    await expect(apps).toHaveAttribute("aria-checked", "false");

    // Its own switch is the only thing that routes it.
    await subscription.click();
    await expect.poll(() => app.lastCall("proxy_set_domain")).toEqual({
      slug: "chatgpt",
      enabled: true,
    });
  });

  test("the chat surface has its own switch, and the family switch never touches it", async ({
    boot,
  }) => {
    // claude.ai carries the user's session cookie rather than a brokered key,
    // so it is the one member the family switch must leave where it is. Same
    // rule as the subscription row above, on the other family: the backend
    // keeps the slug out of the provider's `proxy_domain_slugs`, and this is
    // the frontend half of it.
    const app = await boot({
      proxy: { running: true, port: 8899, pac_port: 8898, ca_trusted: true },
    });

    await app.familyRow("Claude").click();
    const chat = app.page.getByRole("switch", { name: "Route Chat through Gate" });
    await expect(chat).toHaveAttribute("aria-checked", "false");

    // The whole group on: the API surface routes, the chat row does not move.
    // Same completion signal as the test above.
    const family = app.page.getByRole("switch", { name: "Route Claude through Gate" });
    await family.click();
    await expect(family).toHaveAttribute("aria-checked", "true");
    expect(
      (await app.calls()).filter((c) => c.cmd === "proxy_set_domain").map((c) => c.args.slug),
    ).toEqual(["anthropic"]);
    expect((await app.state()).proxy.domains.find((d) => d.slug === "claude-web")?.enabled).toBe(
      false,
    );
    await expect(chat).toHaveAttribute("aria-checked", "false");

    // Its own switch is the only thing that routes it.
    await chat.click();
    await expect.poll(() => app.lastCall("proxy_set_domain")).toEqual({
      slug: "claude-web",
      enabled: true,
    });
  });

  test("a member that fails to connect names itself and stays off", async ({ boot }) => {
    const app = await boot({
      proxy: { running: true, port: 8899, pac_port: 8898, ca_trusted: true },
      failures: { connect_tool: "permission denied writing ~/.claude/settings.json" },
    });

    await app.familyRow("Claude").click();
    const member = app.page.getByRole("switch", { name: "Route CLI through Gate" });
    await member.click();

    await expect(app.page.getByText(/couldn|permission|denied/i).first()).toBeVisible();
    await expect(member).toHaveAttribute("aria-checked", "false");
  });
});
