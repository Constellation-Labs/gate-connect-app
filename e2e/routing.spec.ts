import { test, expect } from "./fixtures";

/** The master switch and the per-member switches behind it. The invariant
 *  worth an e2e: what the UI *does* to the backend after a click, and what it
 *  re-reads before repainting - App re-syncs from `list_tools` / `proxy_status`
 *  rather than trusting the command's return value. */
test.describe("routing", () => {
  test("the master switch turns the engine on and reports what is routing", async ({ boot }) => {
    const app = await boot();

    await app.routingSwitch.click();

    await expect(app.routingSwitch).toHaveAttribute("aria-checked", "true");
    const state = await app.state();
    expect(state.proxy.running).toBe(true);
    // Enabling trusts the CA in the same step, so the header must not sit on
    // "Needs trust" afterwards.
    expect(state.proxy.ca_trusted).toBe(true);
    await expect(app.page.getByText("Routing on").first()).toBeVisible();
  });

  test("turning it off leaves the certificate trusted", async ({ boot }) => {
    const app = await boot({
      proxy: { running: true, port: 8899, pac_port: 8898, ca_trusted: true },
    });

    await app.routingSwitch.click();

    await expect(app.routingSwitch).toHaveAttribute("aria-checked", "false");
    const state = await app.state();
    expect(state.proxy.running).toBe(false);
    // Re-enabling has to stay promptless; untrusting is its own explicit act.
    expect(state.proxy.ca_trusted).toBe(true);
  });

  test("with agents running, the first toggle offers to close them", async ({ boot }) => {
    const app = await boot({ runningAgents: 2 });

    await app.routingSwitch.click();

    // Two steps: the offer, then the confirm. Closing someone's editor is not
    // a one-click act.
    await app.page.getByRole("button", { name: "Close them…" }).click();
    await app.page.getByRole("button", { name: "Close them", exact: true }).click();

    await expect
      .poll(async () => (await app.calls()).some((c) => c.cmd === "close_running_agents"))
      .toBe(true);
  });

  test("with nothing running, the toggle is silent", async ({ boot }) => {
    const app = await boot({ runningAgents: 0 });

    await app.routingSwitch.click();
    await expect(app.routingSwitch).toHaveAttribute("aria-checked", "true");

    // Nothing to close means no takeover: the popover stays on Home.
    await expect(app.page.getByRole("button", { name: "Close them…" })).toHaveCount(0);
    await expect(app.page.getByRole("heading", { name: "Routing" })).toBeVisible();
  });

  test("a failed enable says why and re-syncs the switch", async ({ boot }) => {
    const app = await boot({
      failures: { proxy_enable: "failed to trust the CA: user cancelled the admin prompt" },
    });

    await app.routingSwitch.click();

    await expect(app.page.getByText(/couldn|cancel|trust/i).first()).toBeVisible();
    // Not left showing on over a backend that never started.
    await expect(app.routingSwitch).toHaveAttribute("aria-checked", "false");
    expect((await app.state()).proxy.running).toBe(false);
  });

  test("the master switch trusts the certificate before it enables", async ({ boot }) => {
    const app = await boot();

    await app.routingSwitch.click();

    // Settled first: the pin is released in a `finally` that lands after the
    // switch repaints, and reading the log before that makes the bracket below
    // straddle the launch pin instead.
    await expect(app.routingSwitch).toHaveAttribute("aria-checked", "true");
    await expect
      .poll(async () => {
        const cmds = (await app.calls()).map((c) => c.cmd);
        return cmds.lastIndexOf("unpin_popover") > cmds.indexOf("proxy_trust_ca");
      })
      .toBe(true);

    // The invariant: `enable()` trusts the CA itself (manager*.rs), which used
    // to spring the OS dialog with nothing on screen naming it - a red security
    // warning on Windows, over a popover that dismisses on focus loss. Trusting
    // first means the app is holding `trustPending` while the dialog is up, so
    // the screens can say which button ends it and the popover stays pinned.
    const cmds = (await app.calls()).map((c) => c.cmd);
    const trust = cmds.indexOf("proxy_trust_ca");
    const enable = cmds.indexOf("proxy_enable");
    expect(trust).toBeGreaterThanOrEqual(0);
    expect(enable).toBeGreaterThan(trust);
    // Pinned for the dialog, released after it: the pin exists because the
    // dialog steals focus and the popover would otherwise hide itself.
    expect(cmds.indexOf("pin_popover")).toBeLessThan(trust);
  });

  test("an already-trusted certificate raises no second prompt", async ({ boot }) => {
    const app = await boot({
      proxy: { running: false, ca_trusted: true },
    });

    await app.routingSwitch.click();

    await expect(app.routingSwitch).toHaveAttribute("aria-checked", "true");
    // Nothing to trust, so nothing to warn about: the pre-flight has to stay
    // out of the way of the promptless path.
    const cmds = (await app.calls()).map((c) => c.cmd);
    expect(cmds).not.toContain("proxy_trust_ca");
    expect(cmds).not.toContain("pin_popover");
  });

  test("declining the certificate leaves routing off and says so", async ({ boot }) => {
    const app = await boot({
      failures: {
        proxy_trust_ca: "couldn’t trust the proxy CA: the certificate trust dialog was cancelled or denied",
      },
    });

    await app.routingSwitch.click();

    // Aborted before the engine was asked for anything - the same outcome the
    // implicit trust produced (`ensure_trusted()?` fails the whole enable),
    // reached without a second unexplained dialog.
    const cmds = (await app.calls()).map((c) => c.cmd);
    expect(cmds).not.toContain("proxy_enable");
    await expect(app.page.getByText(/certificate|trust/i).first()).toBeVisible();
    await expect(app.routingSwitch).toHaveAttribute("aria-checked", "false");
  });

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
            display_name: "Claude apps",
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
    const member = app.page.getByRole("switch", { name: "Route Claude Code through Gate" });
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
    await app.page.getByRole("switch", { name: "Route Claude apps through Gate" }).click();

    await expect.poll(() => app.lastCall("proxy_set_domain")).toEqual({
      slug: "anthropic",
      enabled: true,
    });
    // The catalog only maps Claude Code to Anthropic, so the UI drives members
    // one at a time and must never reach for the provider shortcut.
    expect(await app.lastCall("provider_enable")).toBeNull();
  });

  test("a member that fails to connect names itself and stays off", async ({ boot }) => {
    const app = await boot({
      proxy: { running: true, port: 8899, pac_port: 8898, ca_trusted: true },
      failures: { connect_tool: "permission denied writing ~/.claude/settings.json" },
    });

    await app.familyRow("Claude").click();
    const member = app.page.getByRole("switch", { name: "Route Claude Code through Gate" });
    await member.click();

    await expect(app.page.getByText(/couldn|permission|denied/i).first()).toBeVisible();
    await expect(member).toHaveAttribute("aria-checked", "false");
  });
});
