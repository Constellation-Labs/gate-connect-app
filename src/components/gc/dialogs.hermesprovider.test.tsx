import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, render, screen } from "@testing-library/react";
import { HermesProviderDialog } from "./dialogs";

const noop = () => {};
const one = [{ name: "OpenRouter", host: "openrouter.ai", slug: "openrouter" }];

afterEach(cleanup);

/**
 * The question that closes the gap between "Hermes is routed" and "Gate can
 * see any of it".
 *
 * Hermes is one of two rows whose upstream is chosen by the user and lives in
 * another section, so its switch alone routes the tool and inspects nothing.
 * These pin what the dialog owes a reader: which provider, what saying yes
 * reaches beyond Hermes, and that saying no still routes Hermes.
 */
describe("HermesProviderDialog", () => {
  it("names the provider row, not its host and not Gate's slug", () => {
    // "OpenRouter" is how the person thinks of it and what they would look
    // for in the sidebar to change their mind. `openrouter.ai` is an
    // implementation detail of that row and `openrouter` is Gate's key.
    render(<HermesProviderDialog domains={one} onSkip={noop} onConfirm={noop} />);
    expect(screen.getByRole("heading", { name: /OpenRouter/ })).toBeTruthy();
    expect(screen.queryByText(/openrouter\.ai/)).toBeNull();
  });

  it("leads with what is on their machine, then what follows from it", () => {
    // The person's own config first, Gate's mechanism second. An earlier
    // draft opened with "Gate has to inspect that to see them", which is true
    // and is not the thing they need first.
    render(<HermesProviderDialog domains={one} onSkip={noop} onConfirm={noop} />);
    expect(
      screen.getByText(/Your Hermes config uses OpenRouter as its model provider/i),
    ).toBeTruthy();
    // And the consequence, or "Route Hermes only" reads as the cautious choice
    // when it is the one that leaves the traffic unseen.
    expect(screen.getByText(/pass through unseen/i)).toBeTruthy();
  });

  it("discloses the breadth, because yes widens what Gate intercepts", () => {
    // The objection recorded on `integrations::hermes::Coverage` is that
    // enabling a domain from a tool reaches every other client on the machine.
    // Asking answers it only if the question says so.
    render(<HermesProviderDialog domains={one} onSkip={noop} onConfirm={noop} />);
    expect(screen.getByText(/every app on this machine/i)).toBeTruthy();
    expect(screen.getByText(/turn it off again/i)).toBeTruthy();
  });

  it("offers declining as routing Hermes, not as cancelling", () => {
    // Unlike the drift and certificate gates, no is a coherent end state:
    // Hermes routed, provider uninspected. The label has to say which of the
    // two it does, and `useRouting.askOptional` lets the connect proceed.
    const onSkip = vi.fn();
    render(<HermesProviderDialog domains={one} onSkip={onSkip} onConfirm={noop} />);

    const skip = screen.getByRole("button", { name: "Route Hermes only" });
    expect(screen.queryByRole("button", { name: "Cancel" })).toBeNull();
    skip.click();
    expect(onSkip).toHaveBeenCalledOnce();
  });

  it("reads as a list when Hermes points at more than one provider", () => {
    // `config.yaml` carries a base_url per alias, so two providers behind two
    // switched-off domains is reachable. Singular copy there would name one
    // and quietly enable both.
    render(
      <HermesProviderDialog
        domains={[
          { name: "OpenRouter", host: "openrouter.ai", slug: "openrouter" },
          { name: "OpenAI API", host: "api.openai.com", slug: "openai" },
        ]}
        onSkip={noop}
        onConfirm={noop}
      />,
    );
    expect(
      screen.getByRole("heading", { name: /OpenRouter and OpenAI API/ }),
    ).toBeTruthy();
    expect(screen.getByRole("button", { name: "Turn on both" })).toBeTruthy();
  });

  it("confirms with the provider enabled", () => {
    const onConfirm = vi.fn();
    render(<HermesProviderDialog domains={one} onSkip={noop} onConfirm={onConfirm} />);
    screen.getByRole("button", { name: "Turn on" }).click();
    expect(onConfirm).toHaveBeenCalledOnce();
  });
});
