import { afterEach, describe, expect, it } from "vitest";
import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { defaultState } from "../e2e/backend";
import { installFakeTauri } from "../e2e/install";
import { NewUiApp } from "./NewUiApp";

/**
 * Account-scoped state racing the account (AG-1132). The hooks guard their own
 * reads; these pin the two places the shell decides what a reading belongs to:
 * the credential every read is keyed on, and the pane a model write's error is
 * drawn on.
 */
afterEach(cleanup);

const callsTo = (cmd: string) => window.__GATE_E2E__.calls.filter((c) => c.cmd === cmd);

/**
 * Hold `cmd` until the test resolves it. The call still reaches the fake
 * backend, so it is recorded; only its answer is replaced.
 */
function hold(cmd: string) {
  const held: { resolve: (v: unknown) => void; reject: (e: unknown) => void } = {
    resolve: () => undefined,
    reject: () => undefined,
  };
  const internals = (window as any).__TAURI_INTERNALS__;
  const original = internals.invoke;
  internals.invoke = (c: string, args: Record<string, unknown> = {}) => {
    const recorded = original(c, args);
    if (c !== cmd) return recorded;
    recorded.catch(() => {});
    return new Promise((resolve, reject) => {
      held.resolve = resolve;
      held.reject = reject;
    });
  };
  return held;
}

describe("an api-key account", () => {
  it("reads its account-scoped data once at launch, keyed on the key it holds", async () => {
    const state = defaultState();
    state.account = {
      gateway_base_url: "https://gateway.constellationgate.ai",
      has_api_key: true,
      auth_mode: "api_key",
      org_id: null,
      org_name: null,
    };
    state.oauth = { signed_in: false, email: null, sub: null, expires_at_unix: 0 };
    state.accountKeyPrefix = "sk-gw-216c63";
    installFakeTauri(state);
    const prefix = hold("get_account_key_prefix");
    render(<NewUiApp />);

    // The prefix is the last part of the credential, so a read sent before it
    // lands is keyed on a credential that is about to change, and is thrown
    // away with its answer.
    await waitFor(() => expect(callsTo("get_account_key_prefix")).toHaveLength(1));
    expect(callsTo("tool_model_preferences")).toHaveLength(0);

    await act(async () => prefix.resolve("sk-gw-216c63"));
    await waitFor(() => expect(callsTo("tool_model_preferences")).toHaveLength(1));
    await act(() => new Promise((r) => setTimeout(r, 50)));
    expect(callsTo("tool_model_preferences")).toHaveLength(1);
  });
});

describe("a model write that fails", () => {
  it("does not put its error on the pane opened while it was in flight", async () => {
    const state = defaultState();
    state.tools.find((t) => t.slug === "claude-code")!.status = { kind: "connected" };
    state.toolModels.choices["claude-code"] = {
      source: "gate",
      model_ids: ["anthropic/claude-opus-5"],
    };
    state.toolModels.paidAckUnix = 1787740800;
    installFakeTauri(state);
    const write = hold("set_tool_model");
    render(<NewUiApp />);

    const [claude] = await screen.findAllByRole("button", { name: /^Claude/ });
    fireEvent.click(claude);
    const appDefault = await screen.findByRole("radio", { name: /App default/ });
    await waitFor(() => expect((appDefault as HTMLButtonElement).disabled).toBe(false));
    fireEvent.click(appDefault);
    await waitFor(() => expect(callsTo("set_tool_model")).toHaveLength(1));

    // Another pane, then the refusal: it is about Claude Code, not this app.
    const [chatgpt] = await screen.findAllByRole("button", { name: /^ChatGPT/ });
    fireEvent.click(chatgpt);
    await act(async () => write.reject("writing preferences.json: permission denied"));

    expect(screen.queryByText(/Model not changed/)).toBeNull();
  });
});
