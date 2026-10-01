import { describe, expect, it } from "vitest";
import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import {
  ANALYTICS_CONSENT_EVENT,
  ANALYTICS_IDENTITY_EVENT,
  IDENTITY_NOT_LIVE,
  IDENTITY_UNCONFIRMED,
} from "./analytics";

/**
 * The webview follows two backend broadcasts, and `analytics.test.ts` drives
 * them with a fake `emit`. A fake is only honest if the backend really emits
 * that event, under that name, with that payload, from the commands the tests
 * say it does. This pins each of those against `src-tauri/src/lib.rs` itself,
 * whose own unit tests pin that the sign-out paths call the emitter.
 */
const lib = readFileSync(resolve(__dirname, "../../src-tauri/src/lib.rs"), "utf8").replace(/\r\n/g, "\n");

/** The body of one Rust fn, up to the next `#[tauri::command]` or `fn`. */
function body(signature: string): string {
  const at = lib.indexOf(signature);
  expect(at, signature).toBeGreaterThanOrEqual(0);
  const rest = lib.slice(at + signature.length);
  const end = rest.search(/\n(#\[tauri::command\]|fn |async fn |\/\/\/)/);
  return rest.slice(0, end === -1 ? rest.length : end);
}

describe("the analytics broadcasts the backend emits", () => {
  it("uses the names the webview listens on", () => {
    expect(lib).toContain(`const ANALYTICS_IDENTITY_EVENT: &str = "${ANALYTICS_IDENTITY_EVENT}";`);
    expect(lib).toContain(`const ANALYTICS_CONSENT_EVENT: &str = "${ANALYTICS_CONSENT_EVENT}";`);
  });

  it("announces the identity after a sign-out and a Reset", () => {
    // The core forgets (`oauth::clear`; pinned by `core::analytics`'s tests),
    // and the shell announces after it.
    for (const [cmd, change] of [
      ["async fn oauth_sign_out()", "oauth::clear()"],
      ["async fn clear_account()", "account::clear()"],
    ]) {
      const b = body(cmd);
      const changed = b.indexOf(change);
      const announced = b.indexOf("announce_stored_analytics_identity();");
      expect(changed, cmd).toBeGreaterThanOrEqual(0);
      expect(announced, cmd).toBeGreaterThan(changed);
    }
    expect(body("fn announce_stored_analytics_identity(")).toContain(
      "announce_analytics_identity(gate_connect_core::analytics::load_identity())",
    );
  });

  it("forgets the identity in the core, where the CLI's logout reaches it", () => {
    const oauth = readFileSync(resolve(__dirname, "../../crates/core/src/oauth.rs"), "utf8").replace(/\r\n/g, "\n");
    const at = oauth.indexOf("pub fn clear() -> Result<()> {");
    expect(oauth.slice(at, oauth.indexOf("\n}\n", at))).toContain(
      "crate::analytics::forget_identity()",
    );
  });

  it("announces from inside the save, and rejects with the names the webview reads", () => {
    const b = body("fn set_analytics_identity(");
    // Announced through the core's `on_stored`, inside its lock.
    expect(b).toMatch(/save_identity\(\s*identity,\s*\|stored\|\s*\{?\s*announce_analytics_identity\(stored\.clone\(\)\)/);
    expect(lib).toContain(`const ANALYTICS_IDENTITY_NOT_LIVE: &str = "${IDENTITY_NOT_LIVE}";`);
    expect(lib).toContain(`const ANALYTICS_IDENTITY_UNCONFIRMED: &str = "${IDENTITY_UNCONFIRMED}";`);
    expect(body("fn announce_analytics_identity(")).toContain("emit(ANALYTICS_IDENTITY_EVENT, identity)");
  });

  it("announces a consent answer with the payload the webview reads", () => {
    const b = body("fn set_share_diagnostics(");
    expect(b).toContain("ANALYTICS_CONSENT_EVENT");
    expect(b).toContain('"share_diagnostics": enabled');
    expect(b).toContain('"recorded": true');
  });
});
