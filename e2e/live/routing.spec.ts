import { test, expect, MOCK_GATEWAY, type Harness } from "./fixtures";
import * as fs from "node:fs";
import * as path from "node:path";

/**
 * The arc this whole harness exists for, driven entirely by clicks:
 *
 *   turn routing on -> a request really reaches the gateway with the Gate
 *   credential injected -> turn routing off -> the same request is not routed
 *   -> shut Gate Connect down -> the tool can still reach its provider.
 *
 * Every assertion is on something outside the UI: the capture log the mock
 * gateway writes, the tool's config file on disk, and what the backend reports
 * when asked directly. The UI's own account of itself is never the evidence -
 * that is what `e2e/new-ui-routing.spec.ts` covers, against a fake backend.
 *
 * ## Why Codex
 *
 * `connect()` refuses a tool that is not installed, and installing real CLIs is
 * what makes `ci/e2e/run.sh` slow and drift-prone. Codex (like OpenCode) also
 * detects on its config directory existing, so `mkdir $HOME/.codex` in the
 * throwaway home is enough - no npm, no network. And it routes through the
 * loopback RELAY rather than the forward proxy, so a plain HTTP request from
 * Node is exactly the shape its own request has. Claude Code would need
 * `HTTPS_PROXY` plus a CA in Node's own bundle, which tests the test.
 *
 * ## Why this can skip
 *
 * `manager::enable()` calls `ca::ensure_trusted()`, which installs a root CA
 * and needs root on Linux and macOS. `ci/e2e/ui-harness.sh` does that up front
 * when `GATE_UI_HARNESS_ROUTING=1`, which `ui-e2e` in ci.yml sets and a local
 * run does not. Skipping is the honest outcome: the alternative is a suite that
 * fails on every developer machine for a reason that is not a bug.
 */
// Serial, and generous. The arc starts a real engine, installs a certificate
// and spawns a helper daemon; 30s is the default and not enough for the first
// test on a cold runner.
test.describe.configure({ mode: "serial", timeout: 120_000 });

const SECTION = "ChatGPT / Codex";
const codexConfig = (h: Harness) => path.join(h.home, ".codex", "config.toml");

/** The relay origin + path Codex was told to send to, read from its own config. */
function codexBaseUrl(h: Harness): string | null {
  if (!fs.existsSync(codexConfig(h))) return null;
  const toml = fs.readFileSync(codexConfig(h), "utf8");
  return /base_url\s*=\s*"([^"]+)"/.exec(toml)?.[1] ?? null;
}

/** Send what Codex sends: a POST to `<base_url>/responses` with its own bearer. */
async function sendAsCodex(baseUrl: string): Promise<{ ok: boolean; status: number | null }> {
  try {
    const res = await fetch(`${baseUrl}/responses`, {
      method: "POST",
      headers: { "content-type": "application/json", authorization: "Bearer codex-own-token" },
      body: JSON.stringify({ model: "gpt-5", input: "ping" }),
    });
    return { ok: res.ok, status: res.status };
  } catch {
    // Connection refused: nothing is listening where the config pointed.
    return { ok: false, status: null };
  }
}

test.describe("routing, end to end", () => {
  test.skip(
    process.env.GATE_UI_HARNESS_ROUTING !== "1",
    "installs a root CA on this machine; opt in with GATE_UI_HARNESS_ROUTING=1",
  );

  test("the switch routes real traffic through the gateway", async ({ boot, harness }) => {
    // Signed in, pointed at the mock gateway. `save_account` never contacts it -
    // it is the value the relay will forward to.
    await harness.invoke("save_account", {
      baseUrl: MOCK_GATEWAY,
      apiKey: "sk-gw-000000000000000000000000",
    });
    // The shell is gated on the diagnostics question having been answered, and
    // `boot.spec.ts` answers it by clicking. Arranging it again here is what
    // lets this file run alone, or survive a failure in that one: the
    // alternative is a locator timeout naming a switch, on a screen that is
    // actually the consent step. Setting the same value twice is a no-op.
    await harness.invoke("set_share_diagnostics", { enabled: true });

    // Codex is "installed" as far as `detect()` is concerned (it falls back to
    // the config directory existing), and "logged in" as far as `connect()` is:
    // it reads `auth.json` to choose the API-key vs ChatGPT path, and refuses
    // outright when the file is missing. `apikey` is the mode that makes the
    // relay base end in `/v1`, which is what OpenAI's own layout wants.
    fs.mkdirSync(path.join(harness.home, ".codex"), { recursive: true });
    fs.writeFileSync(
      path.join(harness.home, ".codex", "auth.json"),
      JSON.stringify({ auth_mode: "apikey", OPENAI_API_KEY: "sk-codex-own-upstream-key" }),
    );

    // Matched on method and path rather than counted. The capture log is
    // global: the Overview's own boot reads go to the same gateway, and they
    // only stay out of it today because `gateway_api` does not read
    // `GATE_CONNECT_TEST_CA` (only the relay and engine do) so their TLS fails.
    // A count would turn that implementation detail into this test's premise.
    const isOurs = (c: { method: string; path: string }) =>
      c.method === "POST" && c.path.endsWith("/responses");
    const before = harness.captured().filter(isOurs).length;
    const app = await boot();

    await app.routeApp(SECTION);

    // The engine is really up and the certificate really trusted - neither of
    // which the page could fake, and the second of which is what the gate in
    // `routeApp` is for. `enable()` refuses to start without it.
    await expect
      .poll(() => harness.invoke<{ running: boolean; ca_trusted: boolean }>("proxy_status"), {
        timeout: 60_000,
      })
      .toMatchObject({ running: true, ca_trusted: true });

    // And Codex's own config on disk now names the relay.
    const baseUrl = codexBaseUrl(harness);
    expect(baseUrl, "Codex config should name the relay").toMatch(/^http:\/\/127\.0\.0\.1:\d+\//);

    // The message. This is the assertion the whole harness is for.
    const sent = await sendAsCodex(baseUrl!);
    expect(sent.status, "the relay should have forwarded and answered 200").toBe(200);

    // Polled, not read once: `mock-gateway.mjs` appends on header arrival, and
    // the relay answers us on its own schedule.
    await expect
      .poll(() => harness.captured().filter(isOurs).length, { timeout: 10_000 })
      .toBe(before + 1);
    const arrived = harness.captured().filter(isOurs).slice(before);
    // The credential was injected by the relay, never written into a config
    // file - the product's central claim, asserted at the gateway.
    expect(arrived[0].headers["x-gate-api-key"]).toBe("sk-gw-000000000000000000000000");
    // And Codex's own bearer was left intact rather than replaced.
    expect(arrived[0].headers["authorization"]).toBe("Bearer codex-own-token");
    expect(arrived[0].path).toContain("/responses");

    // Nothing in the tool's config is a credential.
    expect(fs.readFileSync(codexConfig(harness), "utf8")).not.toContain("sk-gw-");
  });

  test("turning the app off leaves no residue", async ({ boot, harness }) => {
    const routedUrl = codexBaseUrl(harness);
    expect(routedUrl, "the previous test should have left Codex routed").not.toBeNull();

    const app = await boot();

    await app.appSwitch(SECTION).click();
    await expect.poll(() => app.lastCall("disconnect_tool")).not.toBeNull();

    // Zero residue: the config no longer points at the relay. Codex keeps a
    // `[model_providers.gate]` stub aimed at OpenAI - the one documented
    // exception, because it stores the provider name in each thread - so this
    // asserts the relay origin is gone rather than that the block is.
    await expect.poll(() => codexBaseUrl(harness)).not.toBe(routedUrl);
    expect(fs.readFileSync(codexConfig(harness), "utf8")).not.toContain("127.0.0.1");

    // What is NOT asserted, and used to be: that the engine stops. Since #317
    // routing runs for exactly as long as the app is open - the rail has no
    // master switch to click, and this spec once clicked one here - so the
    // relay stays up by design, and "off" for one app means its config no
    // longer names it. Whether the port still answers is therefore not a fact
    // about this app; the backend's own reading of the config is, and it is
    // the next test's subject.
  });

  test("what Gate leaves behind is a config the tool can still use", async ({ harness }) => {
    // What "shut down" leaves behind, asserted on disk rather than by killing
    // the harness - Playwright owns that process, and killing it would end the
    // run rather than test it.
    //
    // The teardown path itself (`snapshot_and_disable_everything` on quit) is
    // only reachable from the UI on macOS and Windows: on Linux `request_quit`
    // exits outright, because the engine is a detached helper daemon that
    // outlives the GUI and routing is meant to continue. So what is common to
    // all three - and what the person actually cares about - is that the config
    // Gate leaves behind is one the tool can still use.
    const toml = fs.readFileSync(codexConfig(harness), "utf8");
    expect(toml).not.toContain("127.0.0.1");
    expect(toml).not.toContain("sk-gw-");
    expect(toml).toContain("api.openai.com");

    // And the backend agrees the tool is no longer routed.
    const tools = await harness.invoke<{ slug: string; status: { kind: string } }[]>("list_tools");
    const codex = tools.find((t) => t.slug === "codex");
    expect(codex?.status.kind).not.toBe("connected");
  });
});
