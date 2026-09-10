import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, render, screen, fireEvent, waitFor } from "@testing-library/react";
import type { Mock } from "vitest";
import { DEFAULT_GATEWAY_BASE_URL } from "../lib/config";

// The screen's two submit paths call straight into the backend; mock the two
// commands (and keep everything else real) so a test can hold them pending,
// resolve them, or fail them on cue.
vi.mock("../lib/api", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../lib/api")>();
  return {
    ...actual,
    saveAccount: vi.fn(async () => {}),
    oauthBeginLogin: vi.fn(async () => {}),
  };
});
vi.mock("../lib/platform", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../lib/platform")>()),
  usePlatform: () => "macos",
}));
vi.mock("../lib/analytics", () => ({ track: vi.fn(), trackError: vi.fn() }));
vi.mock("../lib/openExternal", () => ({ openExternal: vi.fn(async () => {}) }));
vi.mock("../lib/oauthOffer", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../lib/oauthOffer")>()),
  markOAuthOfferSeen: vi.fn(),
}));

import { saveAccount, oauthBeginLogin } from "../lib/api";
import { markOAuthOfferSeen } from "../lib/oauthOffer";
import { FirstRun } from "./FirstRun";

/** A promise a test can settle by hand, to hold a submit in flight. */
function deferred<T>() {
  let resolve!: (v: T) => void;
  let reject!: (e: unknown) => void;
  const promise = new Promise<T>((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

function renderFirstRun(props: Partial<React.ComponentProps<typeof FirstRun>> = {}) {
  const onConnected = vi.fn();
  render(<FirstRun onConnected={onConnected} {...props} />);
  return onConnected;
}

const signInButton = () => screen.getByRole("button", { name: /Sign in with Constellation/ });

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
  // `clearAllMocks` clears calls but keeps implementations; the error tests
  // below make the api commands reject on purpose, and left in place those
  // rejections would fake failures in every later test.
  (saveAccount as Mock).mockImplementation(async () => {});
  (oauthBeginLogin as Mock).mockImplementation(async () => {});
});

describe("FirstRun initial render", () => {
  it("leads with sign-in and keeps the key form behind the disclosure", () => {
    renderFirstRun();
    expect(screen.getByRole("heading", { level: 1, name: /Welcome to Gate/ })).toBeTruthy();
    expect(signInButton()).toBeTruthy();
    expect(screen.getByRole("button", { name: "Use an API key instead" })).toBeTruthy();
    // No key input until the disclosure is opened.
    expect(screen.queryByPlaceholderText("sk-gw-…")).toBeNull();
    // The footer names the gateway the account will be saved against.
    expect(screen.getByText(DEFAULT_GATEWAY_BASE_URL)).toBeTruthy();
  });

  it("swaps the copy for an expired session on reauth", () => {
    renderFirstRun({ reauth: true });
    expect(screen.getByRole("heading", { level: 1, name: "Welcome back" })).toBeTruthy();
    expect(screen.getByText(/Your session expired/)).toBeTruthy();
  });

  it("opens straight on the key form when startOnKey is set", () => {
    renderFirstRun({ startOnKey: true });
    expect(screen.getByPlaceholderText("sk-gw-…")).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Use an API key instead" })).toBeNull();
  });
});

describe("FirstRun sign-in path", () => {
  it("persists the gateway first, then begins the browser flow, then advances", async () => {
    const onConnected = renderFirstRun();
    fireEvent.click(signInButton());
    await waitFor(() => expect(onConnected).toHaveBeenCalledTimes(1));
    // The account (gateway only, no key) must exist before the OAuth flow
    // starts, so the backend can record the auth mode against it.
    expect(saveAccount).toHaveBeenCalledWith(DEFAULT_GATEWAY_BASE_URL, null);
    expect(oauthBeginLogin).toHaveBeenCalledTimes(1);
    // Choosing sign-in is not choosing the key; the one-time offer stays live.
    expect(markOAuthOfferSeen).not.toHaveBeenCalled();
  });

  it("locks the screen while waiting for the browser", async () => {
    (oauthBeginLogin as Mock).mockImplementation(() => new Promise(() => {}));
    renderFirstRun();
    fireEvent.click(signInButton());
    await screen.findByRole("button", { name: "Waiting for browser…" });
    expect((screen.getByRole("button", { name: "Waiting for browser…" }) as HTMLButtonElement).disabled).toBe(true);
    expect(screen.getByText(/Finish signing in on the page/)).toBeTruthy();
    expect(screen.getByRole("button", { name: "Cancel" })).toBeTruthy();
  });

  it("surfaces a classified error and unlocks when the flow fails", async () => {
    (oauthBeginLogin as Mock).mockImplementation(async () => {
      throw "boom";
    });
    const onConnected = renderFirstRun();
    fireEvent.click(signInButton());
    const alert = await screen.findByRole("alert");
    // The sign_in fallback title, not the raw string.
    expect(alert.textContent).toContain("Couldn’t complete sign-in");
    expect(onConnected).not.toHaveBeenCalled();
    expect((signInButton() as HTMLButtonElement).disabled).toBe(false);
  });

  it("cancel unlocks the screen and swallows the stale attempt's error", async () => {
    const pending = deferred<never>();
    (oauthBeginLogin as Mock).mockImplementation(() => pending.promise);
    renderFirstRun();
    fireEvent.click(signInButton());
    fireEvent.click(await screen.findByRole("button", { name: "Cancel" }));
    // Unlocked immediately: the label is back and the button is pressable.
    expect((signInButton() as HTMLButtonElement).disabled).toBe(false);
    // The abandoned attempt failing later must not re-surface as an error.
    pending.reject("browser flow abandoned");
    await waitFor(() => expect(screen.queryByRole("alert")).toBeNull());
  });
});

describe("FirstRun API-key path", () => {
  function openKeyForm() {
    fireEvent.click(screen.getByRole("button", { name: "Use an API key instead" }));
    return screen.getByPlaceholderText("sk-gw-…");
  }

  it("names the vault and keeps Connect disabled until a key is typed", () => {
    renderFirstRun();
    const input = openKeyForm();
    // The reassurance at the moment the secret is in the user's hands.
    expect(screen.getByText(/Saved to your keychain/)).toBeTruthy();
    const connect = screen.getByRole("button", { name: "Connect with key" }) as HTMLButtonElement;
    expect(connect.disabled).toBe(true);
    fireEvent.change(input, { target: { value: "sk-gw-abc" } });
    expect(connect.disabled).toBe(false);
  });

  it("saves the trimmed key, marks the OAuth offer answered, and advances", async () => {
    const onConnected = renderFirstRun();
    const input = openKeyForm();
    fireEvent.change(input, { target: { value: "  sk-gw-abc  " } });
    fireEvent.click(screen.getByRole("button", { name: "Connect with key" }));
    await waitFor(() => expect(onConnected).toHaveBeenCalledTimes(1));
    expect(saveAccount).toHaveBeenCalledWith(DEFAULT_GATEWAY_BASE_URL, "sk-gw-abc");
    // Pasting a key answers the "would you rather sign in?" question, so the
    // one-time offer must not come back on the next launch.
    expect(markOAuthOfferSeen).toHaveBeenCalledTimes(1);
  });

  it("submits on Enter", async () => {
    const onConnected = renderFirstRun();
    const input = openKeyForm();
    fireEvent.change(input, { target: { value: "sk-gw-abc" } });
    fireEvent.keyDown(input, { key: "Enter" });
    await waitFor(() => expect(onConnected).toHaveBeenCalledTimes(1));
  });

  it("surfaces a save failure without marking the offer seen", async () => {
    (saveAccount as Mock).mockImplementation(async () => {
      throw "keychain said no";
    });
    const onConnected = renderFirstRun();
    const input = openKeyForm();
    fireEvent.change(input, { target: { value: "sk-gw-abc" } });
    fireEvent.click(screen.getByRole("button", { name: "Connect with key" }));
    await screen.findByRole("alert");
    expect(onConnected).not.toHaveBeenCalled();
    expect(markOAuthOfferSeen).not.toHaveBeenCalled();
    // Unlocked for another try.
    expect(
      (screen.getByRole("button", { name: "Connect with key" }) as HTMLButtonElement).disabled,
    ).toBe(false);
  });
});

describe("FirstRun gateway picker", () => {
  it("connects against the server picked in dev mode", async () => {
    renderFirstRun();
    fireEvent.click(screen.getByRole("button", { name: "change" }));
    fireEvent.click(screen.getByRole("button", { name: /Staging/ }));
    fireEvent.click(signInButton());
    await waitFor(() =>
      expect(saveAccount).toHaveBeenCalledWith("https://gateway-staging.constellationgate.ai", null),
    );
  });

  it("starts in dev mode when pre-pointed at a non-default gateway", () => {
    renderFirstRun({ initialGateway: "https://gateway-staging.constellationgate.ai" });
    // The picker is already open (no "change" link) with the staging row active.
    expect(screen.queryByRole("button", { name: "change" })).toBeNull();
    const staging = screen.getByRole("button", { name: /Staging/ }) as HTMLButtonElement;
    expect(staging.disabled).toBe(true);
  });
});
