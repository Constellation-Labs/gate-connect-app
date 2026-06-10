import { describe, it, expect } from "vitest";
import { secretStoreName, type Platform } from "./platform";

describe("secretStoreName", () => {
  it("names the macOS keychain", () => {
    expect(secretStoreName("macos")).toBe("the macOS keychain");
  });

  it("names the Windows Credential Manager", () => {
    expect(secretStoreName("windows")).toBe("Windows Credential Manager");
  });

  it("names the linux secret service", () => {
    expect(secretStoreName("linux")).toBe("the system secret service");
  });

  it("falls back to a generic name for unknown platforms", () => {
    expect(secretStoreName("unknown")).toBe("your system's secure store");
  });

  it("returns a non-empty string for every known platform", () => {
    const platforms: Platform[] = ["macos", "windows", "linux", "unknown"];
    for (const p of platforms) {
      expect(secretStoreName(p).length).toBeGreaterThan(0);
    }
  });
});
