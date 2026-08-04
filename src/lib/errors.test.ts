import { describe, it, expect } from "vitest";
import { backendErrorContext, classifyError } from "./errors";

// These tests assert the *contract* of classifyError rather than exact copy,
// so they stay green as the wording is tuned. The load-bearing guarantees:
//   1. it never lets a non-string payload collapse to "[object Object]"
//   2. structured/object errors are classified by their inner message/field
//   3. unsupported / unregistered command errors surface a platform message
//   4. the original payload is always preserved (stringified) in `raw`

describe("classifyError", () => {
  it("returns the { title, hint, raw } shape", () => {
    const result = classifyError("boom", "generic");
    expect(result).toHaveProperty("title");
    expect(result).toHaveProperty("hint");
    expect(result).toHaveProperty("raw");
    expect(typeof result.title).toBe("string");
    expect(typeof result.hint).toBe("string");
    expect(result.title.length).toBeGreaterThan(0);
    expect(result.hint.length).toBeGreaterThan(0);
  });

  describe("string errors", () => {
    it("preserves the raw string payload verbatim", () => {
      const raw = "connection refused by gateway";
      expect(classifyError(raw, "generic").raw).toBe(raw);
    });

    it("classifies a network failure with a gateway-reach title", () => {
      const result = classifyError("connection refused", "generic");
      expect(result.title.toLowerCase()).toContain("gateway");
    });

    it("classifies a 401 as a rejected API key", () => {
      const result = classifyError("HTTP 401 Unauthorized", "save_api_key");
      expect(result.title.toLowerCase()).toContain("api key");
    });

    it("classifies a cancelled system prompt without naming one OS", () => {
      const result = classifyError("User canceled (-128)", "generic");
      expect(result.title.toLowerCase()).toContain("cancelled");
      // The branch fires on Windows and Linux too, so the copy must not say
      // "macOS" the way it used to.
      expect(result.title.toLowerCase()).not.toContain("macos");
      expect(result.hint.toLowerCase()).not.toContain("macos");
    });

    it("names the button the user actually pressed", () => {
      expect(classifyError("User canceled (-128)", "trust_ca").hint).toContain(
        "Trust certificate",
      );
      expect(classifyError("User canceled (-128)", "forget").hint).toContain("Reset");
    });

    it("falls back to a context-specific generic title when nothing matches", () => {
      const result = classifyError("totally unexpected gibberish", "generic");
      expect(result.title.length).toBeGreaterThan(0);
      // Generic fallback must never leak the "[object" sentinel.
      expect(result.title).not.toContain("[object");
    });
  });

  describe("Error instances", () => {
    it("classifies an Error by its message, not its [object] form", () => {
      const result = classifyError(new Error("connection refused"), "generic");
      expect(result.raw).not.toContain("[object");
      expect(result.raw).toContain("connection refused");
      expect(result.title.toLowerCase()).toContain("gateway");
    });
  });

  describe("object / JSON errors", () => {
    it("never collapses a plain object to the [object Object] sentinel", () => {
      const result = classifyError({ code: 7, detail: "nope" }, "generic");
      expect(result.title).not.toContain("[object");
      expect(result.hint).not.toContain("[object");
      expect(result.raw).not.toContain("[object Object]");
    });

    it("classifies an object by its inner message field", () => {
      const result = classifyError({ message: "connection refused" }, "generic");
      expect(result.title.toLowerCase()).toContain("gateway");
    });

    it("classifies an object by its inner error field", () => {
      const result = classifyError({ error: "HTTP 401 Unauthorized" }, "save_api_key");
      expect(result.title.toLowerCase()).toContain("api key");
    });

    it("serializes an opaque object to JSON in raw so details are reportable", () => {
      const result = classifyError({ code: 42, kind: "weird" }, "generic");
      expect(result.raw).toContain("42");
      expect(result.raw).toContain("weird");
    });
  });

  describe("unsupported / unavailable command errors", () => {
    it("surfaces a platform-availability message for an unregistered command", () => {
      const result = classifyError("command not_found is not registered", "generic");
      expect(result.title.toLowerCase()).toContain("platform");
    });

    it("surfaces a platform-availability message for an unknown command", () => {
      const result = classifyError("unknown command: do_thing", "generic");
      expect(result.title.toLowerCase()).toContain("platform");
    });

    it("surfaces a platform message when the command is not available on this platform", () => {
      const result = classifyError("this command is not available on this platform", "generic");
      expect(result.title.toLowerCase()).toContain("platform");
    });
  });
});

// The allowlist seam between the backend's `report_backend_error` labels and
// what leaves the machine: a backend context added without a frontend
// counterpart must degrade to "generic" rather than forward an unvetted label
// to analytics.
describe("backendErrorContext", () => {
  it("passes through every vetted backend context", () => {
    for (const ctx of [
      "account_reconcile",
      "provider_restore",
      "provider_disable",
      "provider_reconcile",
      "routing_intent",
      "restore_routing",
      "launch_at_login",
    ]) {
      expect(backendErrorContext(ctx)).toBe(ctx);
    }
  });

  it("degrades an unknown backend label to generic", () => {
    expect(backendErrorContext("brand_new_backend_site")).toBe("generic");
    expect(backendErrorContext("")).toBe("generic");
  });

  it("does not forward frontend-only contexts the backend never reports", () => {
    // e.g. proxy_toggle is a valid *frontend* context but not a backend one;
    // a backend claiming it would be a labeling bug, so it degrades too.
    expect(backendErrorContext("proxy_toggle")).toBe("generic");
  });
});

describe("cancelled prompt names the control the user actually touched", () => {
  // These two contexts fire from a role=switch, not a button, and they are the
  // paths users actually hit: the enable path prompts for admin every time the
  // system proxy changes. They used to fall through to "Click Connect again",
  // and there is no Connect button on Home.
  it("names the Routing switch for the master toggle", () => {
    const hint = classifyError("User canceled (-128)", "proxy_toggle").hint;
    expect(hint).toContain("the Routing switch");
    expect(hint).not.toContain("Connect");
    expect(hint).not.toContain("Click");
  });

  it("names a switch for a member toggle", () => {
    const hint = classifyError("User canceled (-128)", "provider_toggle").hint;
    expect(hint).toContain("switch");
    expect(hint).not.toContain("Connect");
  });

  it("still says Click for the paths that really are buttons", () => {
    expect(classifyError("User canceled (-128)", "trust_ca").hint).toContain(
      "Click Trust certificate",
    );
    expect(classifyError("User canceled (-128)", "forget").hint).toContain("Click Reset");
  });
});
