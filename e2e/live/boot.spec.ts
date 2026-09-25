import { test, expect } from "./fixtures";
import * as fs from "node:fs";

/**
 * The walking skeleton for the live suite: prove the seam end to end before
 * anything is built on it.
 *
 * What "end to end" means here, and what each of the three steps buys:
 *
 *  1. The UI's first screen is decided by the REAL backend. No account exists
 *     on the throwaway home, so `get_account` answers null from disk and the
 *     window lands on sign-in. Nothing in the page decided that.
 *  2. A real write through a real command changes what the UI shows. The
 *     account is seeded through the backend rather than the UI because the
 *     sign-in flow under test later is the OAuth one; what matters here is that
 *     the shell re-derives its stage from disk.
 *  3. A CLICK reaches the backend and lands on disk. This is the half neither
 *     existing suite covers: `e2e/*.spec.ts` would assert the command was
 *     called, and `ci/e2e/run.sh` would assert the file, and neither one
 *     connects the two.
 *
 * Serial, and sharing one backend process, for the reason in `fixtures.ts`.
 */
test.describe.configure({ mode: "serial" });

test.describe("live backend", () => {
  test("first run is decided by what is on disk, not by the page", async ({ boot, harness }) => {
    expect(await harness.invoke("get_account")).toBeNull();

    const app = await boot();

    await expect(app.page.getByRole("heading", { name: "Gate Connect" })).toBeVisible();
    // No sidebar yet: there is nothing to navigate to before a credential.
    await expect(app.page.getByRole("navigation", { name: "Main" })).toHaveCount(0);

    // The boot commands really ran against Rust. `app_platform` is the one with
    // an answer only the backend can give, and it has to match this runner.
    expect(await harness.invoke("app_platform")).toBe(
      process.platform === "win32" ? "windows" : process.platform === "darwin" ? "macos" : "linux",
    );
  });

  test("an account on disk puts the app shell on screen", async ({ boot, harness }) => {
    // https, because `account::save` rejects a plaintext gateway. Nothing is
    // sent to it in this spec; it is the stored value the UI reads back.
    await harness.invoke("save_account", {
      baseUrl: "https://gateway.invalid",
      apiKey: "sk-gw-000000000000000000000000",
    });

    const account = await harness.invoke<{ has_api_key: boolean; auth_mode: string }>("get_account");
    expect(account).toMatchObject({ has_api_key: true, auth_mode: "api_key" });

    // The key went to the secrets seam, never to a config file. That is the
    // product's central claim and it is cheap to assert here.
    expect(fs.readdirSync(harness.secrets).length).toBeGreaterThan(0);

    const app = await boot();

    // Signed in with an unanswered diagnostics question, so the derived stage is
    // the consent step rather than the shell. Asserting the real derivation,
    // not a fixture's idea of it.
    await expect(app.page.getByRole("heading", { name: "Share diagnostic data" })).toBeVisible();
  });

  test("answering the consent step writes through to disk and opens the shell", async ({
    boot,
    harness,
  }) => {
    const app = await boot();

    await expect(app.page.getByRole("heading", { name: "Share diagnostic data" })).toBeVisible();

    // The click under test. "Finish setup" records the value on screen, which
    // defaults to on - so this asserts the default the pane draws, not a
    // toggle the test performed.
    await app.page.getByRole("button", { name: "Finish setup" }).click();

    await expect
      .poll(() => harness.invoke<{ share_diagnostics: boolean | null }>("get_preferences"))
      .toMatchObject({ share_diagnostics: true });

    // And the shell is now reachable, with the rail drawn from the real
    // registry's reading of the throwaway home.
    await expect(app.page.getByRole("navigation", { name: "Main" })).toBeVisible();
  });
});
