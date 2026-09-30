import { test, expect, type App } from "./fixtures";

/**
 * Routing's way on, as the window sees it.
 *
 * The launch enable itself runs in Rust before this webview mounts, so the
 * window never starts it. What reaches the window is the aftermath: a buffered
 * `restore_routing` failure, an engine that is not running, and a
 * `proxy-state-changed` when an enable lands late. The window's own ways to
 * (re)start the engine are the Overview notice's "Turn routing on", the setup
 * confirmation's "Turn on routing", and a switch whose connect needs the
 * certificate or the engine first. Each is driven here from the control a
 * person would use.
 */

const CLAUDE_CLI = {
  slug: "claude-code",
  name: "CLI",
  displayName: "Claude Code",
  upstream_provider_name: "Anthropic",
  default_upstream_url: "https://api.anthropic.com",
  status: { kind: "detected" as const },
};

/** What a launch enable that did not complete leaves: a tool the user asked to
 *  route, and no engine behind it. */
const routingDidNotStart = {
  proxy: { running: false, ca_trusted: true },
  tools: [{ ...CLAUDE_CLI, status: { kind: "connected" as const } }],
};

const didNotStart = /Routing didn.t start when Gate Connect opened/;

test.describe("new UI: a launch whose enable failed", () => {
  test("says routing didn't start, with the reason the backend buffered", async ({ boot }) => {
    const app = await boot({
      ...routingDidNotStart,
      backendErrors: [
        { context: "restore_routing", message: "engine could not bind 127.0.0.1:8899" },
      ],
    });

    // The reason: drained at mount, because it predates the webview.
    const alert = app.page.getByRole("alert");
    await expect(alert).toContainText("Couldn’t restore routing at startup");
    await alert.getByText("Details", { exact: true }).click();
    await expect(alert).toContainText("engine could not bind 127.0.0.1:8899");

    // The consequence, on Overview, with its remedy switched off.
    await expect(app.page.getByText(didNotStart)).toBeVisible();
    await expect(app.page.getByRole("switch", { name: "Turn routing on" })).toHaveAttribute(
      "aria-checked",
      "false",
    );
    expect((await app.state()).proxy.running).toBe(false);
  });

  test("the notice's switch starts the engine, and the notice goes", async ({ boot }) => {
    const app = await boot(routingDidNotStart);

    await app.page.getByRole("switch", { name: "Turn routing on" }).click();

    await expect.poll(async () => (await app.state()).proxy.running).toBe(true);
    await expect(app.page.getByText(didNotStart)).toHaveCount(0);
  });

  // `runNoticeAction` used to catch every failure and drop it, so a failed
  // retry read as a switch that does nothing.
  test("a failed retry from the notice says why", async ({ boot }) => {
    const app = await boot({
      ...routingDidNotStart,
      failures: { proxy_enable: "engine could not bind 127.0.0.1:8899" },
    });

    await app.page.getByRole("switch", { name: "Turn routing on" }).click();

    await expect.poll(() => app.lastCall("proxy_enable")).not.toBeNull();
    await expect(app.page.getByRole("alert")).toBeVisible();
    await expect(app.page.getByText(didNotStart)).toBeVisible();
  });

  // `proxy_enable` trusts the CA itself, so the notice used to raise the OS
  // trust prompt with no question first. Every other path that can prompt asks
  // "Trust the Gate certificate?" before it.
  test("the notice asks about the certificate before the OS does", async ({ boot }) => {
    const app = await boot({
      ...routingDidNotStart,
      proxy: { running: false, ca_trusted: false },
    });

    await app.page.getByRole("switch", { name: "Turn routing on" }).click();

    await expect(
      app.page.getByRole("heading", { name: "Trust the Gate certificate?" }),
    ).toBeVisible();
    expect(await app.lastCall("proxy_enable")).toBeNull();
  });

  test("Not now on the notice's question starts nothing, and says nothing", async ({
    boot,
  }) => {
    const app = await boot({
      ...routingDidNotStart,
      proxy: { running: false, ca_trusted: false },
    });

    await app.page.getByRole("switch", { name: "Turn routing on" }).click();
    await app.page.getByRole("button", { name: "Not now" }).click();

    await expect(app.page.getByRole("dialog")).toHaveCount(0);
    await expect(app.page.getByRole("alert")).toHaveCount(0);
    expect(await app.lastCall("proxy_trust_ca")).toBeNull();
    expect(await app.lastCall("proxy_enable")).toBeNull();
    await expect(app.page.getByText(didNotStart)).toBeVisible();
  });

  test("a domain switch whose engine start fails says why", async ({ boot }) => {
    // The one window path that starts the engine and reports a failure: a
    // host-only row has no config to write, so it needs the engine first.
    const app = await boot({
      proxy: {
        running: false,
        ca_trusted: true,
        domains: [
          {
            slug: "acme-router",
            display_name: "Acme Router",
            client: "any-app",
            credential: "brokered",
            scope: "host",
            hosts: ["api.acme-router.test"],
            upstream_url: "https://api.acme-router.test",
            rewrite_prefixes: ["/v1/"],
            passthrough_prefixes: [],
            enabled: false,
            supported: true,
          },
        ],
      },
      failures: { proxy_enable: "engine could not bind 127.0.0.1:8899" },
    });

    const route = await app.appSwitch("Acme Router");
    await route.click();

    await expect(app.page.getByRole("alert")).toBeVisible();
    // Nothing recorded behind a failed start, and the switch says so.
    expect(await app.lastCall("proxy_set_domain")).toBeNull();
    await expect(route).toHaveAttribute("aria-checked", "false");
    expect((await app.state()).proxy.running).toBe(false);
  });
});

test.describe("new UI: the first enable, from setup", () => {
  const firstRun = {
    account: null,
    oauth: { signed_in: false, email: null, expires_at_unix: 0 },
    orgs: [{ orgId: "org-1", name: "Only Org", slug: "only", role: "admin" }],
  };

  async function reachConfirmation(app: App) {
    await app.page.getByRole("button", { name: "Continue with Gate account" }).click();
    await app.page.getByRole("button", { name: "Skip naming" }).click();
    await expect(app.page.getByRole("heading", { name: "You're connected" })).toBeVisible();
  }

  test("a failed Turn on routing stays on the confirmation, routing off", async ({ boot }) => {
    const app = await boot({
      ...firstRun,
      failures: { proxy_enable: "engine could not bind 127.0.0.1:8899" },
    });
    await reachConfirmation(app);

    await app.page.getByRole("button", { name: "Turn on routing" }).click();

    await expect.poll(() => app.lastCall("proxy_enable")).not.toBeNull();
    // Not waved through to an app shell that would say connected over nothing.
    await expect(app.page.getByRole("heading", { name: "You're connected" })).toBeVisible();
    await expect(app.page.getByRole("button", { name: "Turn on routing" })).toBeEnabled();
    await expect(app.page.getByRole("navigation", { name: "Main" })).toHaveCount(0);
    expect((await app.state()).proxy.running).toBe(false);
  });

  // `useSetup.turnOnRouting` recorded the error and `ConnectedPane` had nowhere
  // to draw it, so the button simply stopped spinning.
  test("a failed Turn on routing says why", async ({ boot }) => {
    const app = await boot({
      ...firstRun,
      failures: { proxy_enable: "failed to trust the CA: User canceled. (-128)" },
    });
    await reachConfirmation(app);

    await app.page.getByRole("button", { name: "Turn on routing" }).click();

    await expect(app.page.getByRole("alert")).toContainText("The system prompt was cancelled");
  });
});

test.describe("new UI: the certificate gate in front of a connect", () => {
  const untrusted = {
    proxy: { running: true, ca_trusted: false },
    tools: [{ ...CLAUDE_CLI }],
  };

  test("refusing the OS trust dialog leaves the app unrouted, and says so", async ({ boot }) => {
    const app = await boot({
      ...untrusted,
      failures: { proxy_trust_ca: "failed to trust the CA: User canceled. (-128)" },
    });

    const route = await app.appSwitch("Claude");
    await route.click();
    await app.page.getByRole("button", { name: "Trust certificate" }).click();

    await expect(app.page.getByRole("alert")).toContainText("The system prompt was cancelled");
    // The connect sits behind the trust, so a refused trust writes nothing.
    expect(await app.lastCall("connect_tool")).toBeNull();
    await expect(route).toHaveAttribute("aria-checked", "false");
    const state = await app.state();
    expect(state.proxy.ca_trusted).toBe(false);
    expect(state.tools.find((t) => t.slug === "claude-code")?.status.kind).toBe("detected");
  });

  test("Not now gives up without asking the OS, and without an error", async ({ boot }) => {
    const app = await boot(untrusted);

    const route = await app.appSwitch("Claude");
    await route.click();
    await expect(
      app.page.getByRole("heading", { name: "Trust the Gate certificate?" }),
    ).toBeVisible();
    await app.page.getByRole("button", { name: "Not now" }).click();

    await expect(app.page.getByRole("dialog")).toHaveCount(0);
    await expect(app.page.getByRole("alert")).toHaveCount(0);
    expect(await app.lastCall("proxy_trust_ca")).toBeNull();
    expect(await app.lastCall("connect_tool")).toBeNull();
    await expect(route).toHaveAttribute("aria-checked", "false");
  });

  test("an already-trusted certificate raises no prompt", async ({ boot }) => {
    const app = await boot({ ...untrusted, proxy: { running: true, ca_trusted: true } });

    const route = await app.appSwitch("Claude");
    await route.click();

    await expect.poll(() => app.lastCall("connect_tool")).toEqual({ slug: "claude-code" });
    await expect(app.page.getByRole("dialog")).toHaveCount(0);
    expect(await app.lastCall("proxy_trust_ca")).toBeNull();
    await expect(route).toHaveAttribute("aria-checked", "true");
  });
});

test.describe("new UI: the engine changing without a click", () => {
  test("proxy-state-changed repaints the window from the engine", async ({ boot }) => {
    // The launch enable finishing after the window mounted, or the CLI turning
    // routing on: the engine announces it and nothing on screen was clicked.
    // OpenCode because its section is the tool alone, so the rail row reads
    // the tool's own verdict.
    const app = await boot({
      proxy: { running: false, ca_trusted: true },
      tools: [
        {
          slug: "opencode",
          name: "OpenCode",
          upstream_provider_name: "your existing providers",
          default_upstream_url: "https://api.anthropic.com",
          status: { kind: "connected" },
        },
      ],
    });
    const row = app.page.getByRole("button", { name: /^OpenCode/ }).first();
    await expect(app.page.getByText(didNotStart)).toBeVisible();
    await expect(row).toHaveAccessibleName("OpenCode Not protected");

    await app.patch({ proxy: { running: true, port: 8899, pac_port: 8898, ca_trusted: true } });
    await app.emit("proxy-state-changed");

    await expect(app.page.getByText(didNotStart)).toHaveCount(0);
    await expect(row).toHaveAccessibleName("OpenCode Protected");
  });
});
