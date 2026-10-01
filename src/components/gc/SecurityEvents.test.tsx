import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { SecurityEvents } from "./SecurityEvents";
import type { SecurityEvent } from "../../lib/api";
import { adaptModels, modelLabelsFor } from "../../lib/toolModels";

afterEach(cleanup);

const blocked: SecurityEvent = {
  id: "01A",
  requestId: "req-1",
  at: "2026-08-31T14:03:00Z",
  action: "block",
  category: "credential",
  tool: "claude-code",
  model: "claude-opus-4",
  provider: "anthropic",
};

function section(props: Partial<Parameters<typeof SecurityEvents>[0]> = {}) {
  return (
    <SecurityEvents
      events={[]}
      loading={false}
      unavailable={false}
      onRetry={() => {}}
      onOpenInDashboard={() => {}}
      {...props}
    />
  );
}

/**
 * AG-853 moved the feed from a pane of its own onto the Overview. What has to
 * survive that is the heading level: a second `h1` inside the Overview would
 * give the pane two titles and put the feed on a level with "Overview" itself,
 * which is the opposite of what moving it below Token savings says.
 */
describe("the section it became", () => {
  it("titles itself at the level of the cards beside it, not the pane's", () => {
    render(section());

    expect(screen.getByRole("heading", { level: 2, name: "Security events" })).toBeTruthy();
    expect(screen.queryByRole("heading", { level: 1 })).toBeNull();
  });
});

describe("the three states a period can be in", () => {
  // AC6 turns on these three being distinguishable. A zero is a reading, an
  // unavailable feed is not, and a feed still being read is neither.
  it("says No security events when the feed loaded and there were none", () => {
    render(section());
    expect(screen.getByText("No security events")).toBeTruthy();
    expect(screen.queryByText("Unavailable")).toBeNull();
  });

  it("says Unavailable and offers a way out when the feed could not load", () => {
    render(section({ unavailable: true }));
    expect(screen.getByText("Unavailable")).toBeTruthy();
    expect(screen.getByRole("button", { name: "Try again" })).toBeTruthy();
    // The empty sentence is a claim about the user's traffic and must not be
    // made by a screen that failed to ask.
    expect(screen.queryByText("No security events")).toBeNull();
  });

  it("claims neither while the first read is still in flight", () => {
    render(section({ loading: true }));
    expect(screen.queryByText("No security events")).toBeNull();
    expect(screen.queryByText("Unavailable")).toBeNull();
  });
});

/**
 * The feed's connection pill is gone (2026-09-23).
 *
 * Three tests lived here: that Live / Reconnecting / Offline each drew their
 * label, and that no pill was claimed before the first read answered. Product
 * asked for the badge to be removed, so what is left to pin is the
 * consequence - the section says nothing about its own connection in any
 * state, and in particular a reconnecting feed still shows the rows it has
 * rather than blanking the table.
 */
describe("the feed's own connection state", () => {
  it("makes no claim about the connection, in any state", () => {
    render(section({ events: [blocked] }));
    expect(screen.queryByRole("status", { name: /^Event feed/ })).toBeNull();
    for (const label of ["Live", "Reconnecting", "Offline"]) {
      expect(screen.queryByText(label)).toBeNull();
    }
  });

  it("keeps showing the events it has while the stream is unhappy", () => {
    // A feed having a bad minute is not an empty feed, and blanking the table
    // would lose what the user was reading. The section cannot tell the two
    // apart any more, which is exactly why it must not guess.
    render(section({ events: [blocked] }));
    expect(screen.getByText("Blocked")).toBeTruthy();
    expect(screen.queryByText("No security events")).toBeNull();
  });
});

/**
 * Ten rows, then ten more per click (2026-09-23), matching the App pane's
 * recent-activity table.
 *
 * Client-side here: the feed arrives over a stream and the whole session is
 * already in memory, so there is no page to fetch. A session left open was
 * drawing every event it had ever seen.
 */
describe("how many rows it draws", () => {
  const many = (n: number) =>
    Array.from({ length: n }, (_, i) => ({ ...blocked, requestId: `req-${i}` }));

  it("draws ten however many it holds", () => {
    render(section({ events: many(25) }));
    expect(screen.getAllByRole("row")).toHaveLength(11); // 10 + the header
  });

  it("reveals ten more per click", () => {
    render(section({ events: many(25) }));
    fireEvent.click(screen.getByRole("button", { name: "Load more" }));
    expect(screen.getAllByRole("row")).toHaveLength(21);
  });

  it("drops the control once everything is on screen", () => {
    render(section({ events: many(12) }));
    fireEvent.click(screen.getByRole("button", { name: "Load more" }));
    expect(screen.getAllByRole("row")).toHaveLength(13);
    expect(screen.queryByRole("button", { name: "Load more" })).toBeNull();
  });

  it("offers nothing to load when ten is all there is", () => {
    render(section({ events: many(10) }));
    expect(screen.queryByRole("button", { name: "Load more" })).toBeNull();
  });
});

describe("what a row shows, and what it must not", () => {
  it("names the verdict, category, tool and model", () => {
    render(section({ events: [blocked] }));
    expect(screen.getByText("Blocked")).toBeTruthy();
    expect(screen.getByText("Credential")).toBeTruthy();
    expect(screen.getByText("claude-code")).toBeTruthy();
    expect(screen.getByText("claude-opus-4")).toBeTruthy();
  });

  it("labels the categories the frame draws, and the two it does not", () => {
    // `phi` is the PII/PHI scanner's other half and takes its glyph; `other`
    // is a word with no glyph rather than one chosen by eye.
    render(
      section({
        events: [
          { ...blocked, id: "01A", category: "phi" },
          { ...blocked, id: "01B", category: "other" },
        ],
      }),
    );
    const phi = screen.getByText("PHI");
    expect(phi.querySelector("svg")).toBeTruthy();
    const other = screen.getByText("Other");
    expect(other.querySelector("svg")).toBeNull();
  });

  it("prints a category it does not know as received, including prototype names", () => {
    // The value is the gateway's string. A bare object index would answer
    // "constructor" with a function and render nothing.
    render(
      section({
        events: [
          { ...blocked, id: "01A", category: "constructor" },
          { ...blocked, id: "01B", category: "jailbreak" },
        ],
      }),
    );
    expect(screen.getByText("constructor")).toBeTruthy();
    expect(screen.getByText("jailbreak")).toBeTruthy();
  });

  it("draws a flagged event as Flagged, not Blocked", () => {
    render(section({ events: [{ ...blocked, id: "01B", action: "flag" }] }));
    expect(screen.getByText("Flagged")).toBeTruthy();
    expect(screen.queryByText("Blocked")).toBeNull();
  });

  it("renders an unattributed tool or model as an ordinary dash", () => {
    // Null is the normal outcome for an agent the gateway's allowlist does not
    // name. It is not an error state and must not read as one.
    render(section({ events: [{ ...blocked, tool: null, model: null, category: null }] }));
    expect(screen.queryByText("Unknown")).toBeNull();
    expect(screen.queryByText("Unavailable")).toBeNull();
    expect(screen.getAllByText("-").length).toBe(3);
  });

  it("shows newest first", () => {
    const older = { ...blocked, id: "01A", at: "2026-08-31T10:00:00Z", category: "pii" };
    const newer = { ...blocked, id: "01B", at: "2026-08-31T14:00:00Z", category: "injection" };
    render(section({ events: [older, newer] }));
    const cells = screen.getAllByText(/^(PII|Injection)$/);
    expect(cells[0].textContent).toBe("Injection");
  });
});

describe("opening an event", () => {
  it("hands the whole event to the caller rather than a bare id", () => {
    // The row leads straight to the dashboard since 2026-09-23; the summary
    // dialog that used to sit between them is gone. Still the whole event
    // rather than a request id, so the shell is not the only thing that could
    // ever build a URL from it.
    const onOpenInDashboard = vi.fn();
    render(section({ events: [blocked], onOpenInDashboard }));
    fireEvent.click(screen.getByRole("button", { name: /View/ }));
    expect(onOpenInDashboard).toHaveBeenCalledWith(blocked);
  });

  it("retries on demand when the feed is unavailable", () => {
    const onRetry = vi.fn();
    render(section({ unavailable: true, onRetry }));
    fireEvent.click(screen.getByRole("button", { name: "Try again" }));
    expect(onRetry).toHaveBeenCalled();
  });
});

/**
 * The Model cell draws the catalogue's label where it has one, the way the
 * dashboard's Messages list does, and the id stays on hover because it is what
 * the reader can search for. The event itself is never rewritten: `model` on
 * `SecurityEvent` is the wire contract.
 */
describe("the model cell", () => {
  const names = modelLabelsFor(
    adaptModels({
      data: [{ id: "anthropic/claude-opus-4-5", owned_by: "anthropic", name: "Claude Opus 4.5" }],
    }),
  );
  const row: SecurityEvent = { ...blocked, model: "anthropic/claude-opus-4-5" };

  it("names the model from the catalogue and keeps the id on hover", () => {
    render(section({ events: [row], modelLabels: names }));
    const cell = screen.getByText("Claude Opus 4.5");
    expect(cell.getAttribute("title")).toBe("anthropic/claude-opus-4-5");
    expect(screen.queryByText("anthropic/claude-opus-4-5")).toBeNull();
  });

  it("keeps the id when the catalogue does not list the model, or was not read", () => {
    render(section({ events: [{ ...row, model: "aion-labs/aion-2-0" }], modelLabels: names }));
    expect(screen.getByText("aion-labs/aion-2-0")).toBeTruthy();
    cleanup();
    render(section({ events: [row] }));
    expect(screen.getByText("anthropic/claude-opus-4-5")).toBeTruthy();
  });

  it("still says unattributed for a row with no model", () => {
    render(section({ events: [{ ...row, model: null }], modelLabels: names }));
    expect(screen.getAllByText("-").length).toBeGreaterThan(0);
  });

  it("reads the pipeline's unknown sentinel as no model", () => {
    // `resolved_model` holds the literal on rows whose body could not be read,
    // and the security bus passes it through where tool-events maps it to null.
    render(section({ events: [{ ...row, model: "unknown" }], modelLabels: names }));
    expect(screen.queryByText("unknown")).toBeNull();
    expect(screen.queryByTitle("unknown")).toBeNull();
  });

  it("names a provider-native id from the catalogue, and draws its vendor's mark", () => {
    // What the security feed actually carries: `resolved_model`, which on
    // Anthropic Direct is the canonical id without its vendor. This is the row
    // the Overview showed as an id while the app pane showed a name.
    const { container } = render(
      section({ events: [{ ...row, provider: null, model: "claude-opus-4-5" }], modelLabels: names }),
    );
    expect(screen.getByText("Claude Opus 4.5")).toBeTruthy();
    expect(screen.getByTitle("claude-opus-4-5")).toBeTruthy();
    expect(container.querySelector('svg path[fill="#E8704E"]')).toBeTruthy();
  });
});

/**
 * The Tool cell (`1402:18013`): the tool's logo in colour at 20px and its
 * product name, where it printed the gateway's slug alone.
 */
describe("the tool cell", () => {
  const toolNames = new Map([["claude-code", "Claude Code"]]);

  it("draws the tool's mark and product name, with the slug on hover", () => {
    const { container } = render(section({ events: [blocked], toolNames }));
    expect(screen.getByText("Claude Code")).toBeTruthy();
    expect(screen.getByTitle("claude-code")).toBeTruthy();
    expect(screen.queryByText("claude-code")).toBeNull();
    // The Claude Code mark, in the colour the frame draws it (`1402:18014`).
    const mark = container.querySelector('[style*="E8704E"], [style*="232, 112, 78"]');
    expect(mark).toBeTruthy();
    expect(mark!.querySelector("svg")).toBeTruthy();
  });

  it("prints the slug when the registry has no name for it", () => {
    render(section({ events: [{ ...blocked, tool: "cursor" }], toolNames }));
    expect(screen.getByText("cursor")).toBeTruthy();
  });

  it("gives a gateway platform id the mark of the product it belongs to", () => {
    render(section({ events: [{ ...blocked, tool: "codex-desktop" }] }));
    const cell = screen.getByTitle("codex-desktop").parentElement!;
    expect(cell.querySelector("svg")).toBeTruthy();
  });

  it("keeps the slot empty for a platform with no mark", () => {
    render(section({ events: [{ ...blocked, tool: "cursor" }] }));
    const cell = screen.getByTitle("cursor").parentElement!;
    expect(cell.querySelector("svg")).toBeNull();
  });

  it("does not reach a prototype member for a hostile slug", () => {
    render(section({ events: [{ ...blocked, tool: "constructor" }] }));
    const cell = screen.getByTitle("constructor").parentElement!;
    expect(cell.querySelector("svg")).toBeNull();
  });

  it("still says unattributed for a row with no tool", () => {
    render(section({ events: [{ ...blocked, tool: null }], toolNames }));
    expect(screen.getAllByText("-").length).toBeGreaterThan(0);
  });
});

/**
 * The provider mark beside the model (`1402:18017`), and the order it falls
 * back in. The security bus passes `provider` through raw, so it is a marketplace
 * account string or the pipeline's `unknown` on a large share of rows; a chain
 * that stopped at the provider would draw a cube beside a model id that names
 * its vendor plainly.
 */
describe("the model cell's provider mark", () => {
  const ANTHROPIC = 'svg path[fill="#E8704E"]';
  const CUBE = 'svg[data-icon="cube"]';
  const row: SecurityEvent = { ...blocked, model: "anthropic/claude-opus-4-5" };

  it("draws the provider's brand mark, and names the provider for a screen reader", () => {
    const { container } = render(section({ events: [row] }));
    expect(container.querySelector(ANTHROPIC)).toBeTruthy();
    expect(screen.getByText("anthropic", { selector: ".sr-only" })).toBeTruthy();
  });

  it("falls back to the model id's namespace when the provider has no mark", () => {
    const { container } = render(
      section({ events: [{ ...row, provider: "openai_compatible:Marcus OpenRouter" }] }),
    );
    expect(container.querySelector(ANTHROPIC)).toBeTruthy();
    expect(container.querySelector(CUBE)).toBeNull();
  });

  it("draws the namespace's mark and says nothing when no provider was named", () => {
    const { container } = render(section({ events: [{ ...row, provider: null }] }));
    expect(container.querySelector(ANTHROPIC)).toBeTruthy();
    // The header row keeps its own hidden "Action" label; the rows say nothing.
    expect(container.querySelector("tbody .sr-only")).toBeNull();
  });

  it("reads the pipeline's unknown sentinel as no provider", () => {
    const { container } = render(section({ events: [{ ...row, provider: "unknown" }] }));
    expect(container.querySelector(ANTHROPIC)).toBeTruthy();
    expect(screen.queryByText("unknown")).toBeNull();
  });

  it("draws the cube when the provider is named but unmapped and the id names no vendor", () => {
    // The third step of the chain: nothing to draw a brand for, but a provider
    // was named, so the slot is a glyph rather than left empty.
    const { container } = render(
      section({ events: [{ ...row, provider: "openai_compatible:Marcus OpenRouter", model: "gpt-5" }] }),
    );
    expect(container.querySelector(CUBE)).toBeTruthy();
    expect(screen.getByTitle("openai_compatible:Marcus OpenRouter")).toBeTruthy();
  });

  it("draws the cube for a vendor with no published mark", () => {
    const { container } = render(
      section({ events: [{ ...row, provider: null, model: "sao10k/l3-euryale-70b" }] }),
    );
    expect(container.querySelector(CUBE)).toBeTruthy();
  });

  it("leaves the slot empty when nothing names a vendor", () => {
    render(section({ events: [{ ...row, provider: null, model: "gpt-5" }] }));
    // The cell, not the row: the Category glyph and the View button draw SVGs
    // of their own in other cells.
    const cell = screen.getByTitle("gpt-5").parentElement!;
    expect(cell.querySelector("svg")).toBeNull();
    expect(cell.querySelector('[aria-hidden="true"]')).toBeTruthy();
  });
});
