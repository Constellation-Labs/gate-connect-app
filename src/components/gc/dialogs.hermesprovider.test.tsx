import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, render, screen } from "@testing-library/react";
import { HermesProviderDialog } from "./dialogs";

const noop = () => {};
const one = [{ host: "openrouter.ai", slug: "openrouter" }];

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
  it("names the provider host, not Gate's slug for it", () => {
    // `openrouter.ai` is what the person put in `config.yaml`. `openrouter` is
    // the catalog's internal name for the row and means nothing to them.
    render(<HermesProviderDialog domains={one} onSkip={noop} onConfirm={noop} />);
    expect(
      screen.getByRole("heading", { name: /openrouter\.ai/ }),
    ).toBeTruthy();
  });

  it("says what happens without it, which is the whole bug", () => {
    // Routed and uninspected is the state that read as Protected for an
    // afternoon while every request tunnelled past. The dialog has to name it,
    // or "Route Hermes only" sounds like the cautious choice.
    render(<HermesProviderDialog domains={one} onSkip={noop} onConfirm={noop} />);
    expect(screen.getByText(/passes straight out, uninspected/i)).toBeTruthy();
  });

  it("discloses the breadth, because yes widens what Gate intercepts", () => {
    // The objection recorded on `integrations::hermes::Coverage` is that
    // enabling a domain from a tool reaches every other client on the machine.
    // Asking answers it only if the question says so.
    render(<HermesProviderDialog domains={one} onSkip={noop} onConfirm={noop} />);
    expect(screen.getByText(/any app on this machine/i)).toBeTruthy();
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
          { host: "openrouter.ai", slug: "openrouter" },
          { host: "api.openai.com", slug: "openai" },
        ]}
        onSkip={noop}
        onConfirm={noop}
      />,
    );
    expect(
      screen.getByRole("heading", { name: /openrouter\.ai and api\.openai\.com/ }),
    ).toBeTruthy();
    expect(screen.getByRole("button", { name: "Inspect both" })).toBeTruthy();
  });

  it("confirms with the provider enabled", () => {
    const onConfirm = vi.fn();
    render(<HermesProviderDialog domains={one} onSkip={noop} onConfirm={onConfirm} />);
    screen.getByRole("button", { name: "Inspect it" }).click();
    expect(onConfirm).toHaveBeenCalledOnce();
  });
});
