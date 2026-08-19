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
   * The model that served the request, or copy saying none was attributed.
   *
   * This column used to hold a conversation title. It cannot: the only
   * human-readable label the gateway holds is the user's own prompt, stored
   * unredacted, and AG-574 excludes prompt text. So the row identifies the request
   * by what served it and which conversation it belonged to.
   */
  model: string;
  /** Conversation identifier, rendered mono. */
  reference: string;
}
