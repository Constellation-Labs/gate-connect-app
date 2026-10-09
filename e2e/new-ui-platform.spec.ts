import { test, expect } from "./fixtures";

/**
 * The window's platform branches, driven through the fake `app_platform`.
 *
 * What this can prove: that the window asks which OS it is on and uses the
 * right nouns and controls for the answer. How any of it paints in the webview
 * each platform ships is not covered here - these all run in one Chromium.
 */

/**
 * Where the credential lives. The window names the secret store in the
 * one-time OAuth offer, which is the one place it tells a key account where a
 * session would be kept - and a reassurance that names the wrong vault is worth
 * nothing, so each is checked for what it says and for not saying another
 * platform's word.
 */
test.describe("new UI: the secret store, per platform", () => {
  const keyAccount = {
    account: {
      gateway_base_url: "https://gw.example",
      has_api_key: true,
      auth_mode: "api_key" as const,
      org_id: null,
      org_name: null,
    },
    oauth: { signed_in: false, email: null, expires_at_unix: 0 },
    // Never offered, so the offer is on screen at boot.
    localStorage: { "gc.oauth-offer.v1.seen": "" },
  };

  const stores = [
    { platform: "macos", says: "session lives in the keychain", never: /keyring|Credential Manager/i },
    { platform: "windows", says: "session lives in Credential Manager", never: /keychain|keyring/i },
    { platform: "linux", says: "session lives in the keyring", never: /keychain|Credential Manager/i },
  ] as const;

  for (const { platform, says, never } of stores) {
    test(`${platform} names its own secret store in the sign-in offer`, async ({ boot }) => {
      const app = await boot({ ...keyAccount, platform });

      const offer = app.page.getByRole("dialog");
      await expect(
        offer.getByRole("heading", { name: "Sign in instead of pasting a key" }),
      ).toBeVisible();
      await expect(offer).toContainText(says);
      await expect(offer.getByText(never)).toHaveCount(0);
    });
  }
});

test.describe("new UI: launch at login on Linux", () => {
  test("is offered, and reaches the backend", async ({ boot }) => {
    // Linux registers an XDG autostart entry through the same plugin, so the
    // row is not hidden there.
    const app = await boot({ platform: "linux" });

    await app.openSettings();
    const launch = app.page.getByRole("switch", { name: "Launch at login" });
    await expect(launch).toHaveAttribute("aria-checked", "false");

    await launch.click();

    await expect.poll(() => app.lastCall("set_launch_at_login")).toEqual({ enabled: true });
    await expect(launch).toHaveAttribute("aria-checked", "true");
  });
});
