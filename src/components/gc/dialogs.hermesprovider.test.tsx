import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, render, screen } from "@testing-library/react";
import { HermesProviderDialog } from "./dialogs";

const noop = () => {};
const one = [{ name: "OpenRouter", slug: "openrouter", tools: [] }];
const props = { domains: one, defaulted: false, onCancel: noop, onConfirm: noop };

afterEach(cleanup);

/**
 * The question that closes the gap between "Hermes is routed" and "Gate can
 * see any of it".
 *
 * Hermes is one of two rows whose upstream is chosen by the user and lives in
 * another section, so its switch alone routes the tool and inspects nothing.
 * These pin what the dialog owes a reader: which provider, whether that came
 * from their config or from Hermes' default, what saying yes reaches beyond
 * Hermes, and that saying no writes nothing.
 */
describe("HermesProviderDialog", () => {
  it("names the provider row, not its host and not Gate's slug", () => {
    // "OpenRouter" is how the person thinks of it and what they would look
    // for in the sidebar to change their mind. `openrouter.ai` is an
    // implementation detail of that row and `openrouter` is Gate's key.
    render(<HermesProviderDialog {...props} />);
    expect(screen.getByRole("heading", { name: /OpenRouter/ })).toBeTruthy();
    expect(screen.queryByText(/openrouter\.ai/)).toBeNull();
  });

  it("leads with what is on their machine, then what follows from it", () => {
    // The person's own config first, Gate's mechanism second. An earlier
    // draft opened with "Gate has to inspect that to see them", which is true
    // and is not the thing they need first.
    render(<HermesProviderDialog {...props} />);
    expect(
      screen.getByText(/Your Hermes config uses OpenRouter as its model provider/i),
    ).toBeTruthy();
    // And the consequence, because without it Cancel reads as the cautious
    // choice when it is the one that leaves Hermes unrouted.
    expect(screen.getByText(/pass through unseen/i)).toBeTruthy();
  });

  it("says it is Hermes' default when the config named no provider", () => {
    // A missing or unparseable `config.yaml` still means Hermes will call
    // OpenRouter - that is its documented default - but "your config uses"
    // would be a claim about a file nobody read.
    render(<HermesProviderDialog {...props} defaulted />);
    expect(screen.getByText(/Hermes has no provider in its config/i)).toBeTruthy();
    expect(screen.queryByText(/Your Hermes config uses/i)).toBeNull();
  });

  it("names the tools a provider's own switch also reaches", () => {
    // `anthropic` on means the Anthropic provider is on, and
    // `reconcile_enabled` connects a detected Claude Code at the next launch.
    // That is a config write to another tool, which "interception" does not
    // cover, so it is said where it applies and nowhere else.
    render(
      <HermesProviderDialog
        {...props}
        domains={[{ name: "Anthropic", slug: "anthropic", tools: ["Claude Code"] }]}
      />,
    );
    expect(screen.getByText(/also covers Claude Code/i)).toBeTruthy();

    cleanup();
    render(<HermesProviderDialog {...props} />);
    expect(screen.queryByText(/also covers/i)).toBeNull();
  });

  it("discloses the breadth, because yes widens what Gate intercepts", () => {
    // The objection recorded on `integrations::hermes::Coverage` is that
    // enabling a domain from a tool reaches every other client on the machine.
    // Asking answers it only if the question says so.
    render(<HermesProviderDialog {...props} />);
    expect(screen.getByText(/every app on this machine/i)).toBeTruthy();
    expect(screen.getByText(/turn it off again/i)).toBeTruthy();
  });

  it("offers cancelling, and does not offer routing Hermes uninspected", () => {
    // The secondary used to read "Route Hermes only", which connected Hermes
    // and left the provider off. That state - routed, reporting Protected,
    // inspected by nothing - is the defect this dialog exists to prevent, so
    // it must not be a button. No means nothing is written.
    const onCancel = vi.fn();
    render(<HermesProviderDialog {...props} onCancel={onCancel} />);

    expect(screen.queryByRole("button", { name: /Route Hermes only/i })).toBeNull();
    screen.getByRole("button", { name: "Cancel" }).click();
    expect(onCancel).toHaveBeenCalledOnce();
  });

  it("reads as a list when Hermes points at more than one provider", () => {
    // `config.yaml` carries a base_url per alias, so two providers behind two
    // switched-off domains is reachable. Singular copy there would name one
    // and quietly enable both.
    render(
      <HermesProviderDialog
        {...props}
        domains={[
          { name: "OpenRouter", slug: "openrouter", tools: [] },
          { name: "OpenAI API", slug: "openai", tools: [] },
        ]}
      />,
    );
    expect(
      screen.getByRole("heading", { name: /OpenRouter and OpenAI API/ }),
    ).toBeTruthy();
    // The same words as `OpenCodeEnvDialog`'s primary, in the same order.
    expect(screen.getByRole("button", { name: "Turn both on" })).toBeTruthy();
  });

  it("does not say both about three", () => {
    render(
      <HermesProviderDialog
        {...props}
        domains={[
          { name: "OpenRouter", slug: "openrouter", tools: [] },
          { name: "OpenAI API", slug: "openai", tools: [] },
          { name: "Anthropic", slug: "anthropic", tools: [] },
        ]}
      />,
    );
    expect(
      screen.getByRole("heading", { name: /OpenRouter, OpenAI API and Anthropic/ }),
    ).toBeTruthy();
    expect(screen.getByRole("button", { name: "Turn all on" })).toBeTruthy();
  });

  it("confirms with the provider enabled", () => {
    const onConfirm = vi.fn();
    render(<HermesProviderDialog {...props} onConfirm={onConfirm} />);
    screen.getByRole("button", { name: "Turn on" }).click();
    expect(onConfirm).toHaveBeenCalledOnce();
  });
});
