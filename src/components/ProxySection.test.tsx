import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, render, screen } from "@testing-library/react";
import type { Mock } from "vitest";
import type { Platform } from "../lib/platform";

// The "off" explanatory paragraph swaps proxy + trust-store wording by
// platform. Mock usePlatform to set the platform, and the api module so the
// self-fetching section mounts without hitting Tauri.
vi.mock("../lib/platform", () => ({ usePlatform: vi.fn() }));
vi.mock("../lib/api", () => ({
  proxyStatus: vi.fn().mockResolvedValue({
    running: false,
    port: null,
    ca_trusted: false,
    domains: [],
  }),
  proxyEnable: vi.fn(),
  proxyDisable: vi.fn(),
  proxySetDomain: vi.fn(),
}));

import { usePlatform } from "../lib/platform";
import { ProxySection } from "./ProxySection";

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

async function renderOn(platform: Platform) {
  (usePlatform as Mock).mockReturnValue(platform);
  render(<ProxySection />);
  // Let the async proxyStatus() load settle (flushes its setState in act).
  await screen.findByText(/Routes supported apps through Gate/);
}

describe("ProxySection off-state copy", () => {
  it("says system proxy + keychain on macOS", async () => {
    await renderOn("macos");
    expect(
      screen.getByText(/installs a trusted certificate in your keychain and points your system proxy/),
    ).toBeTruthy();
    expect(screen.queryByText(/Windows proxy/)).toBeNull();
  });

  it("says Windows proxy + certificate store on Windows", async () => {
    await renderOn("windows");
    expect(
      screen.getByText(/installs a trusted certificate in your certificate store and points your Windows proxy/),
    ).toBeTruthy();
    expect(screen.queryByText(/system proxy/)).toBeNull();
  });
});
