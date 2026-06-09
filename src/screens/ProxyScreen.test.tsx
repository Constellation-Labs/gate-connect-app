import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, render, screen } from "@testing-library/react";
import type { Mock } from "vitest";
import type { Platform } from "../lib/platform";
import type { ProxyState } from "../lib/api";
import { ProxyScreen } from "./ProxyScreen";

// The CA-trust notice swaps the trust-store name by platform; drive it by
// mocking usePlatform rather than the async Tauri lookup.
vi.mock("../lib/platform", () => ({ usePlatform: vi.fn() }));
import { usePlatform } from "../lib/platform";

// running + !ca_trusted is the only state that renders the trust notice.
const proxy: ProxyState = {
  running: true,
  port: 8080,
  ca_trusted: false,
  domains: [],
};

function renderOn(platform: Platform) {
  (usePlatform as Mock).mockReturnValue(platform);
  render(
    <ProxyScreen
      proxy={proxy}
      providers={[]}
      busy={false}
      error={null}
      onBack={() => {}}
      onToggleProxy={() => {}}
      onSetProvider={() => {}}
      onTrustCa={() => {}}
    />,
  );
}

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

describe("ProxyScreen CA-trust notice", () => {
  it("names the keychain on macOS", () => {
    renderOn("macos");
    expect(screen.getByText(/isn’t trusted in your keychain yet/)).toBeTruthy();
    expect(screen.queryByText(/certificate store/)).toBeNull();
  });

  it("names the certificate store on Windows", () => {
    renderOn("windows");
    expect(screen.getByText(/isn’t trusted in your certificate store yet/)).toBeTruthy();
    expect(screen.queryByText(/keychain/)).toBeNull();
  });
});
