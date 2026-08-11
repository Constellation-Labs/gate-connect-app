import { test, expect } from "./fixtures";

/**
 * The popover's platform branches, all three driven through the fake
 * `app_platform`. What this can prove: that the app asks the backend which OS
 * it is on and renders the right chrome and the right nouns for the answer.
 *
 * What it cannot: how any of it paints in the webview each platform actually
 * ships (WKWebView, WebView2, WebKitGTK). These all run in one Chromium.
 */

/** Window chrome. macOS and Windows are non-activating tray popovers with
 *  rounded windows; Linux is a borderless window that has to draw its own. */
test.describe("window chrome", () => {
  test("linux draws its own title bar and squares the card corners", async ({ boot }) => {
    const app = await boot({ platform: "linux" });

    await expect(app.page.getByRole("button", { name: "Minimize" })).toBeVisible();
    await expect(app.page.getByRole("button", { name: "Close" })).toBeVisible();

    // A rounded card on a square window exposes the window's own corners
    // behind it, so the card must not round here.
    const radius = await app.page
      .locator("#root > div")
      .evaluate((el) => getComputedStyle(el).borderRadius);
    expect(radius).toBe("0px");
  });

  for (const platform of ["macos", "windows"] as const) {
    test(`${platform} has no title bar and rounds to match the window`, async ({ boot }) => {
      const app = await boot({ platform });

      await expect(app.page.getByRole("button", { name: "Minimize" })).toHaveCount(0);
      const radius = await app.page
        .locator("#root > div")
        .evaluate((el) => getComputedStyle(el).borderRadius);
      expect(radius).not.toBe("0px");
    });
  }
});

/** The shell-environment channel. On Linux the variables *are* the system
 *  proxy, so the choice cannot be offered - `env_export_separable` carries
 *  that from the backend rather than the UI guessing at platforms. */
test.describe("command-line switch", () => {
  test("is offered where the choice is separable", async ({ boot }) => {
    const app = await boot({ platform: "macos", proxy: { env_export_separable: true } });

    await expect(
      app.page.getByRole("switch", { name: "Route command-line tools through Gate" }),
    ).toBeVisible();
  });

  test("is absent on linux, where it could not honour itself", async ({ boot }) => {
    const app = await boot({ platform: "linux", proxy: { env_export_separable: false } });

    await expect(
      app.page.getByRole("switch", { name: "Route command-line tools through Gate" }),
    ).toHaveCount(0);
    // The master switch is still there: only the sub-setting drops out.
    await expect(app.routingSwitch).toBeVisible();
  });
});

/**
 * Platform-named nouns. PRODUCT.md's first principle is that the user should
 * always feel where the key lives, and a reassurance that names the wrong
 * vault is worth nothing - so each of these is checked for what it says *and*
 * for not saying another platform's word.
 */
test.describe("platform nouns", () => {
  const vaults = [
    { platform: "macos", says: /your keychain/i, never: /keyring|Credential Manager/i },
    { platform: "windows", says: /Credential Manager/, never: /keychain|keyring/i },
    { platform: "linux", says: /your keyring/i, never: /keychain|Credential Manager/i },
  ] as const;

  for (const { platform, says, never } of vaults) {
    test(`${platform} names the secret store in the pinned footer`, async ({ boot }) => {
      const app = await boot({ platform });

      await expect(app.page.getByText(says)).toBeVisible();
      await expect(app.page.getByText(never)).toHaveCount(0);
    });
  }

  const trustStores = [
    { platform: "macos", says: /trusted in your keychain/i },
    { platform: "windows", says: /trusted in your certificate store/i },
    { platform: "linux", says: /trusted in your certificate store/i },
  ] as const;

  for (const { platform, says } of trustStores) {
    // A different vault from the one above, and a different question: on Linux
    // the CA lives in the system trust store, which is emphatically not a
    // keyring.
    test(`${platform} names the CA trust store in the certificate explainer`, async ({ boot }) => {
      const app = await boot({ platform, proxy: { ca_trusted: true } });

      await app.openSettings();
      await app.page.getByRole("button", { name: "What’s this?" }).click();

      await expect(app.page.getByText(says)).toBeVisible();
    });
  }
});
