import { test, expect } from "./fixtures";

/** Boot: what the popover resolves to for each account shape it can find on
 *  disk. This is App's initial-load effect, which no unit test can reach -
 *  it reads six commands and decides a screen from all of them together. */
test.describe("boot", () => {
  test("a signed-in account with an org lands on Home", async ({ boot }) => {
    const app = await boot();

    await expect(app.page.getByRole("heading", { name: "Gate Connect" })).toBeVisible();
    await expect(app.page.getByRole("heading", { name: "Routing" })).toBeVisible();
    // The org is the header's sub-label: the one thing that says who you are
    // on this gateway.
    await expect(app.page.getByText("Constellation Labs")).toBeVisible();
    await expect(app.routingSwitch).toHaveAttribute("aria-checked", "false");
  });

  test("no account at all lands on first run", async ({ boot }) => {
    const app = await boot({ account: null, oauth: { signed_in: false, email: null, expires_at_unix: 0 } });

    await expect(app.page.getByRole("heading", { name: /Welcome to Gate/ })).toBeVisible();
    await expect(app.page.getByRole("button", { name: /Sign in with Constellation/ })).toBeVisible();
  });

  test("signed in with no org picked goes straight to the org picker", async ({ boot }) => {
    const app = await boot({ account: { org_id: null, org_name: null } });

    await expect(app.page.getByText("Side Project")).toBeVisible();
    // Not back through sign-in: the session is fine, only the org is missing.
    await expect(app.page.getByRole("button", { name: /Sign in with Constellation/ })).toHaveCount(0);
  });

  test("routing already on shows the count of what is routing", async ({ boot }) => {
    const app = await boot({
      proxy: { running: true, port: 8899, pac_port: 8898, ca_trusted: true },
      providers: [
        {
          slug: "anthropic",
          display_name: "Anthropic",
          subtitle: "Claude Code + Claude Desktop",
          enabled: true,
          available: true,
          tool_slugs: ["claude-code"],
          domain_slugs: ["anthropic"],
          chat_domain_slugs: [],
        },
      ],
      tools: [
        {
          slug: "claude-code",
          name: "CLI",
          upstream_provider_name: "Anthropic",
          default_upstream_url: "https://api.anthropic.com",
          status: { kind: "connected" },
        },
      ],
    });

    await expect(app.routingSwitch).toHaveAttribute("aria-checked", "true");
    await expect(app.page.getByText(/^On · 1 of \d+ routing$/)).toBeVisible();
  });
});
