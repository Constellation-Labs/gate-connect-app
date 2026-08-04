import { describe, it, expect } from "vitest";
import { secretStoreName, trustStoreName, type Platform } from "./platform";

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
