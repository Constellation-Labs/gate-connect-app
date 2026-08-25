import { describe, it, expect } from "vitest";
import {
  browserScopeNote,
  secretStoreName,
  trustPromptHint,
  trustPromptWaiting,
  trustStoreName,
  type Platform,
} from "./platform";

const PLATFORMS: Platform[] = ["macos", "windows", "linux", "unknown"];

describe("secretStoreName", () => {
  it("names the macOS keychain", () => {
    expect(secretStoreName("macos")).toBe("your keychain");
  });

  it("names the Windows Credential Manager in full", () => {
    expect(secretStoreName("windows")).toBe("Credential Manager");
  });

  it("says keyring on Linux, not secret service", () => {
    expect(secretStoreName("linux")).toBe("your keyring");
  });

  it("falls back to a generic name for unknown platforms", () => {
    expect(secretStoreName("unknown")).toBe("your system’s secure store");
  });

  it("swaps the determiner where a nearby 'your' is already doing the work", () => {
    expect(secretStoreName("macos", "the")).toBe("the keychain");
    expect(secretStoreName("linux", "the")).toBe("the keyring");
  });

  it("gives Windows no determiner either way, because it is a proper noun", () => {
    expect(secretStoreName("windows", "the")).toBe("Credential Manager");
    expect(secretStoreName("windows", "your")).toBe("Credential Manager");
  });

  it("returns a non-empty string for every known platform", () => {
    for (const p of PLATFORMS) {
      expect(secretStoreName(p).length).toBeGreaterThan(0);
      expect(secretStoreName(p, "the").length).toBeGreaterThan(0);
    }
  });
});

describe("trustStoreName", () => {
  it("is the keychain only on macOS", () => {
    expect(trustStoreName("macos")).toBe("keychain");
  });

  it("is the certificate store on Windows and Linux", () => {
    // Linux installs the CA into the system trust store, not a keyring, so
    // calling it a keychain here was wrong on two platforms out of three.
    expect(trustStoreName("windows")).toBe("certificate store");
    expect(trustStoreName("linux")).toBe("certificate store");
  });

  it("never confuses the CA trust store with the secret store", () => {
    for (const p of PLATFORMS) {
      expect(trustStoreName(p)).not.toBe(secretStoreName(p));
    }
  });
});

describe("trustPromptHint", () => {
  it("warns the Windows user that a security warning is coming, and that it's expected", () => {
    const hint = trustPromptHint("windows");
    expect(hint).toContain("security warning");
    expect(hint).toContain("expected");
    // The button that ends it. "Confirm"/"OK" name nothing on that dialog.
    expect(hint).toContain("Yes");
  });

  it("names the password prompt on macOS and Linux, not a Yes button", () => {
    expect(trustPromptHint("macos")).toContain("password");
    expect(trustPromptHint("linux")).toContain("password");
    expect(trustPromptHint("macos")).not.toContain("Yes");
    expect(trustPromptHint("linux")).not.toContain("Yes");
  });

  it("still sets an expectation on an unresolved platform", () => {
    // `unknown` is the state for the first async tick, so this string does
    // reach the screen: it must not be empty, and must not guess a mechanism.
    const hint = trustPromptHint("unknown");
    expect(hint.length).toBeGreaterThan(0);
    expect(hint).not.toContain("password");
    expect(hint).not.toContain("Windows");
  });
});

describe("trustPromptWaiting", () => {
  it("names the certificate Windows is quoting back, so the user can match it", () => {
    // Exactly the CN in cert_authority.rs. A near-miss here is worse than
    // silence: the dialog quotes the real name, and a mismatch is what a
    // careful user would read as "this is not the app that asked".
    expect(trustPromptWaiting("windows")).toContain("Gate Connect Local CA");
    expect(trustPromptWaiting("windows")).toContain("Yes");
  });

  it("is present tense on every platform, because the dialog is already up", () => {
    for (const p of PLATFORMS) {
      const waiting = trustPromptWaiting(p);
      expect(waiting.length).toBeGreaterThan(0);
      expect(waiting).not.toContain("will ask");
    }
  });

  it("differs from the pre-click hint everywhere, so the swap is visible", () => {
    for (const p of PLATFORMS) {
      expect(trustPromptWaiting(p)).not.toBe(trustPromptHint(p));
    }
  });
});

describe("browserScopeNote", () => {
  it("includes the browser where the system proxy is the browser's proxy", () => {
    // macOS wires a PAC through `networksetup` and trusts the CA in the system
    // keychain; Windows sets the WinINET proxy and trusts it in the Windows
    // certificate store. Both are what a browser reads, so a host-matched row
    // genuinely covers the browser and the copy has to say so.
    expect(browserScopeNote("macos")).toContain("browser");
    expect(browserScopeNote("windows")).toContain("browser");
  });

  it("says nothing at all where the browser is not covered", () => {
    // Not a shorter sentence: no sentence. `system_proxy_linux.rs` wires only
    // environment variables, which a browser never reads, so there is no
    // browser claim to make - and the mechanism behind that is three clauses
    // the user cannot act on. The host sentence this appends to has already
    // bounded the scope. `unknown` is empty for the same reason plus one: it is
    // the first async tick, which is no time to guess at interception.
    expect(browserScopeNote("linux")).toBe("");
    expect(browserScopeNote("unknown")).toBe("");
  });

  it("is a whole sentence wherever it says anything, so it can be appended", () => {
    for (const p of PLATFORMS) {
      const note = browserScopeNote(p);
      if (note === "") continue;
      expect(note.endsWith(".")).toBe(true);
      expect(note).toContain("browser");
    }
  });
});
