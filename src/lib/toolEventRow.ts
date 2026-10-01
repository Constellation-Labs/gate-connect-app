/**
 * One row of a tool's activity feed, as the pane renders it (AG-574).
 *
 * In `lib/` rather than beside the component that draws it, because the adapter in
 * `toolEvents.ts` produces these and importing a type from `components/` into
 * `lib/` inverts the layering every other adapter here follows: `activity.ts`
 * imports `UsageStats` and `MessagesBucket` the same way, and those live with their
 * component only because they predate this rule. A shared type also means an
 * `onView` handler could be supplied by the surface that owns a destination,
 * instead of the adapter having to invent one.
 */

import type { IconName } from "../components/gc/Icon";

export type ActivityStatus = "success" | "error";
export type ActivitySecurity = "allow" | "flagged" | "redacted" | "blocked";

export interface ActivityEntry {
  id: string;
  /** Pre-formatted upstream so the pane stays locale-agnostic. */
  time: string;
  status: ActivityStatus;
  /**
   * What the guardrails did, or `null` when the gateway said nothing - either no
   * decision was recorded, or the row is not this caller's and security detail is
   * self-only for every role. The two are deliberately indistinguishable here;
   * both draw no pill. Null is *not* `allow`: rendering it as one would report a
   * blocked request as permitted.
   */
  security: ActivitySecurity | null;
  /**
   * Which guardrail category the row is about, as the gateway named it, or
   * null when it named none.
   *
   * The frame's Type column (`table/recent-activity` on `Flows / App`) draws
   * this beside a 20px glyph. Rendered as the gateway spelled it, the same way
   * `SecurityEvents` renders the same field: relabelling it here would invent a
   * display vocabulary for values only the gateway knows.
   */
  category: string | null;
  /** The glyph for {@link category}, chosen by the adapter the way
   *  `Policy.icon` is. Null when there is no category to draw one for. */
  categoryIcon: IconName | null;
  /**
   * A hover explanation for {@link category}, where the label alone does not
   * say what it means. Set only for "Regular", which is Connect's word rather
   * than the gateway's: the dash cell beside it has always carried a title
   * saying why it is a dash, and a reader wondering what "Regular" means had
   * nowhere to hover. A gateway-named category needs none; it is the gateway's
   * spelling and there is nothing to add to it.
   */
  categoryTitle?: string | null;
  /** The model that served the request, or copy saying none was attributed. */
  model: string;
  /** The raw id behind {@link model}, for the cell's hover. Null when the row
   *  names no model.
   *
   *  Once the gateway sends a label, `model` stops being the id, and the label
   *  can be a prettified guess ("Claude Opus 4 5" for
   *  `anthropic/claude-opus-4-5`). The id is what the reader can search for, so
   *  it stays one hover away rather than leaving the table. */
  modelId: string | null;
  /** Which upstream served it (`anthropic`, `openai`). Null when the request
   *  never reached one - which the gateway reports honestly and often.
   *
   *  A reading. It is what the row is allowed to *say*: the tooltip and the
   *  screen-reader text come from here and nowhere else. */
  provider: string | null;
  /** The namespace `VendorMark` draws when `provider` has no mark - because
   *  the gateway named none, or named a marketplace account no mark is keyed
   *  on. The model id's own where it has one; where it does not, `labelEntries`
   *  fills it from the catalogue entry that names the model.
   *
   *  Separate from `provider` because it is not the same claim: it identifies
   *  *the model beside it* and not who served the request. It used to hold the
   *  provider where there was one; `VendorMark` took that step over when the
   *  Security events table began sharing it, so that a named provider with no
   *  mark of its own still falls through to the model's. Rows with no provider
   *  are disproportionately the ones that never reached one - blocked, or
   *  failed before routing - so announcing a derived value as the upstream
   *  would put "Anthropic" in a
   *  screen reader beside a `blocked` pill for a request Anthropic never saw.
   *  The mark is decorative and sits next to the id it came from; the words
   *  are not, so they stay with `provider`. */
  vendor: string | null;
  /**
   * What the conversation was about, or null when there is nothing to show.
   *
   * This is the user's own prompt, shortened upstream. The design asks for it
   * (Figma 272:3286) and product accepted that; the gateway gates it per row so a
   * colleague's prompt never arrives here in the first place. Null covers three
   * different cases the row does not distinguish - no session, a session still
   * holding its placeholder name, and a row this caller may not see into - because
   * all three mean the same thing to the reader: nothing to show.
   */
  title: string | null;
  /** Conversation identifier, rendered mono, e.g. `824bd2c0-4123`. */
  reference: string;
  /**
   * Open this request in the web dashboard.
   *
   * Optional, and supplied by the surface rather than the adapter: the adapter has
   * no business knowing where a request can be looked at. Absent means no Action
   * control is drawn - which is what happened for the whole of the last round,
   * when `dashboard-web` had no route to send anyone to. It has one:
   * `/messages/:requestId` opens the request's detail.
   */
  onView?: () => void;
}
