import { useCallback, useEffect, useRef, useState } from "react";
import { activityToolEvents } from "./api";
import { toFailure, type ActivityFailure } from "./activity";
import type { ActivityEntry } from "./toolEventRow";
import type { ModelLabels } from "./toolModels";
import type { IconName } from "../components/gc/Icon";

/**
 * Adapter between `GET /v1/me/tool-events` and the app pane's activity feed
 * (AG-574).
 *
 * The gateway sends one row per request: a timestamp, a couple of enums,
 * identifiers, and one human-readable string. That string is the user's own
 * prompt, shortened upstream. An earlier round of this module refused to carry it
 * and said so here, on the reading that showing prompt text was out of scope;
 * Figma 272:3286 restored the column and product accepted what the label is. The
 * gateway gates it per row, so a colleague's prompt never arrives here to be
 * mishandled in the first place.
 *
 * So this module's job stays narrow - format a time, name a state, pass the
 * strings through - and it still has no copy to invent.
 */

/** Per-row shape as the gateway sends it. */
interface RawEvent {
  requestId: string;
  at: string;
  status: "success" | "error";
  /** Null either because no decision was recorded for this request, or because
   *  the caller may not see this row's security detail. The gateway deliberately
   *  does not distinguish them - saying *that* something was withheld would leak
   *  that a colleague's request was acted on - and both want the same thing on
   *  screen. What null is not, in either case, is `allow`. */
  securityAction: "allow" | "flag" | "redact" | "block" | null;
  securityCategory: string | null;
  model: string | null;
  /**
   * The model's display name, as the gateway spells it (AG-951).
   *
   * Optional because the gateway does not send it yet. When it does, it is
   * what the Recent activity table draws: design asked for "the same model
   * labels we used in Gate currently", and those live in the catalogue the
   * gateway owns, not in the id this row already carries.
   *
   * **Rendered verbatim, whatever it says.** The catalogue's names are
   * currently uneven - "Claude Opus 4.1" beside "Claude Opus 4 5", and some
   * carrying a vendor prefix - and the client deliberately does not tidy
   * them: the gateway is the source of truth for this string, so a client
   * that stripped prefixes or restored punctuation would be a second,
   * disagreeing opinion about the same field. Fixing those values is the
   * gateway-side half of AG-951.
   *
   * Absent falls back to the id, which is what shipped before this. After
   * the gateway sends the field, it is null only when `model` is, so a model
   * the catalogue does not name arrives as a prettified id ("Claude Opus 4 5")
   * rather than as the id itself. That is expected, not a bug here; the id is
   * kept on the entry as `modelId` for the cell's hover.
   */
  modelName?: string | null;
  provider: string | null;
  sessionRef: string | null;
  /** The user's own prompt, shortened and per-row gated upstream. Null when there
   *  is nothing to show; see `ActivityEntry.title`. */
  conversationTitle: string | null;
}

interface RawToolEvents {
  generatedAt: string;
  window: { from: string; to: string };
  toolScope: { tool: string };
  installation?: { installId: string | null };
  events?: RawEvent[];
  nextCursor?: string | null;
}

/**
 * What the conversation cell says when the row carries no session reference.
 *
 * Not a withholding and not an error: a request that belonged to no session has no
 * conversation to name. The security cell draws its own dash for its own reason,
 * with its own tooltip - see `AppPane`.
 *
 * A plain hyphen, the same glyph every other "no reading" in the app draws. It
 * was an em dash, which CLAUDE.md forbids outright, and it sat two cells away
 * from the Type and Security dashes - three spellings of "nothing here" in one
 * table row.
 */
const NO_REFERENCE = "-";

/** What a row says when no model was attributed to the request. */
const NO_MODEL = "Unknown model";

/**
 * The vendor namespace of a canonical model id - the mark's fallback for a row
 * whose provider the gateway did not name, or named with no mark of its own.
 *
 * `provider` is null far more often than it looks: the column holds the
 * pipeline's `unknown` sentinel on a substantial share of rows - the gateway's
 * own comment counts 42 of 102 in a dev database - and the endpoint maps that
 * to null rather than let a client draw a mark for a provider nobody has. The
 * mark slot then rendered an empty spacer, which is what "the Model column has
 * no vendor icon" turned out to be: not a missing mark, a missing reading.
 *
 * But the vendor is right there in the id. Model ids are canonical
 * `provider/model` (`anthropic/claude-opus-5`), and the model card and the
 * Gate-model confirmation read their vendor through this too. Deriving it is not guessing:
 * `providerMarkFor` returns undefined for a namespace it has no mark for, so a
 * spelling this does not recognise falls back to the cube exactly as before.
 *
 * Only the namespace of a *namespaced* id. A bare `gpt-5` names no vendor, and
 * inventing one from the model family is the kind of guess principle 6 forbids.
 */
export function vendorFromModelId(model: string | null | undefined): string | null {
  if (!model) return null;
  const slash = model.indexOf("/");
  if (slash <= 0) return null;
  return model.slice(0, slash);
}

/**
 * The pipeline's sentinel for a column nobody filled. `gateway_requests` holds
 * the literal `unknown` for a model that could not be read off the request and
 * for a provider the request never reached, and both reach a client: the
 * tool-events endpoint maps them to null before they leave the gateway, the
 * security bus passes the columns through raw. Read on both feeds here, so
 * neither table can print "unknown" where a name goes or announce it as an
 * upstream in a tooltip, whichever side forgets.
 */
export const UNATTRIBUTED_SENTINEL = "unknown";

/** The value as a reading: null when absent, and null when it is the sentinel. */
export function attributed(value: string | null | undefined): string | null {
  return value === undefined || value === null || value === UNATTRIBUTED_SENTINEL ? null : value;
}


/**
 * The gateway's action verbs, in the pane's own vocabulary.
 *
 * The gateway records what a criterion *did* (`block`, `redact`) because that is
 * the policy's own wording; the design's pills read as what happened to the
 * request (`blocked`, `redacted`). Mapped here rather than renamed on either side:
 * the gateway's column is shared with the dashboard and the policy config, and the
 * pills are the Figma's copy.
 */
const SECURITY: Record<NonNullable<RawEvent["securityAction"]>, ActivityEntry["security"]> = {
  allow: "allow",
  flag: "flagged",
  redact: "redacted",
  block: "blocked",
};

/**
 * When a request happened, to the second (Figma 116:30951, "Jun 6, 00:50:51").
 *
 * Seconds because this is a feed of individual requests and an agent sends several
 * a minute: without them, four rows read as the same moment and the order looks
 * arbitrary. The date because the window is 24 hours and so straddles midnight -
 * a bare clock time makes yesterday evening look like this evening.
 *
 * A timestamp rather than an age, for the reason `ActivityView.takenAt` gives: an
 * age has to be recomputed to stay true, and a "2 minutes ago" written twenty
 * minutes ago is a worse lie than the age it was added to disclose.
 */
function eventTime(at: string): string {
  const d = new Date(at);
  const date = d.toLocaleDateString([], { month: "short", day: "numeric" });
  const time = d.toLocaleTimeString([], {
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
    hour12: false,
  });
  return `${date}, ${time}`;
}

/**
 * Glyphs for the guardrail categories, keyed by what the gateway sends.
 *
 * The same trio the frames draw - `Icon / ShieldAlert`, `Icon / UserRound`,
 * `Icon / KeyRound` - and the same glyphs `POLICY_ICONS` puts on the Overview
 * policy rows, because it is one taxonomy read on two surfaces. Both the
 * gateway's own spellings and the policy row ids are keyed, since the two
 * vocabularies have not been reconciled and only `pii` is evidenced in a
 * fixture.
 *
 * Anything unrecognised falls back rather than drawing nothing: the frame puts
 * a glyph in every Type cell, and a ragged column reads worse than a generic
 * shield. Same fallback `POLICY_ICONS` uses.
 */
const CATEGORY_ICONS: Record<string, IconName> = {
  pii: "userRound",
  phi: "userRound",
  "pii-phi": "userRound",
  injection: "shieldAlert",
  "prompt-injection": "shieldAlert",
  credential: "key",
  credentials: "key",
};

const CATEGORY_FALLBACK: IconName = "shieldCheck";

/**
 * Case-insensitive, own-property lookup into a category table: spellings drift
 * between producers (`PII` and `pii`), and a bare index would hand back
 * `Object.prototype`'s `constructor` for a gateway string that happens to match.
 */
function lookup<T>(table: Record<string, T>, key: string): T | undefined {
  const k = key.toLowerCase();
  return Object.hasOwn(table, k) ? table[k] : undefined;
}

/** What the Type column says for a request no guardrail matched.
 *
 * The explicit categories arrive from the gateway and render as the frame draws
 * them; this is the fourth case the frame does not draw, because it only ever
 * drew rows where something fired.
 *
 * Reserved for `allow`. See `toEntry`: it is a verdict, not a note that the
 * gateway answered, so an uncategorised `block` must not borrow it. */
const REGULAR = "Regular";
const REGULAR_ICON: IconName = "shieldCheck";
/** What "Regular" means, for the cell's hover. The dash cell has always
 *  explained itself the same way; a word Connect chose owes the same. */
const REGULAR_TITLE = "Gate examined this request and no guardrail matched";

/** One row, formatted. */
function toEntry(raw: RawEvent): ActivityEntry {
  const model = attributed(raw.model);
  const provider = attributed(raw.provider);
  return {
    id: raw.requestId,
    time: eventTime(raw.at),
    status: raw.status,
    // `allow` is the honest default *only* when the gateway answered. A null
    // action is unknown to us and renders as a dash, not as a verdict.
    security: raw.securityAction ? SECURITY[raw.securityAction] : null,
    // A guardrail category exists only where a guardrail fired, so ordinary
    // traffic carried none and the Type column was a dash on every row - which
    // is what AG-887 reports as "empty". A dash reads as missing; what actually
    // happened is that the request was examined and nothing matched, and that
    // is a reading worth naming.
    //
    // Keyed on `allow` specifically, not on "the gateway answered". "Regular"
    // is a verdict, and it is only true of the action that IS one: a `block`,
    // `flag` or `redact` that arrived without a category was examined and
    // something fired, so calling it Regular would print the opposite of the
    // pill in the Security cell two columns over. That row keeps the dash -
    // something happened and the gateway did not name what - which is the same
    // withholding the `security` line above makes, and CLAUDE.md principle 6.
    category: raw.securityCategory ?? (raw.securityAction === "allow" ? REGULAR : null),
    categoryIcon: raw.securityCategory
      ? (lookup(CATEGORY_ICONS, raw.securityCategory) ?? CATEGORY_FALLBACK)
      : raw.securityAction === "allow"
        ? REGULAR_ICON
        : null,
    categoryTitle:
      !raw.securityCategory && raw.securityAction === "allow" ? REGULAR_TITLE : null,
    // The gateway's label when it sends one, the id when it does not. Never a
    // label this side invented - see `RawEvent.modelName`.
    model: raw.modelName ?? model ?? NO_MODEL,
    modelId: model,
    provider,
    // The namespace alone; `VendorMark` tries `provider` first. See
    // `ActivityEntry.vendor` for why the two are carried separately.
    vendor: vendorFromModelId(model),
    title: raw.conversationTitle,
    reference: raw.sessionRef ?? NO_REFERENCE,
  };
}

export interface ToolEventsView {
  entries: ActivityEntry[];
  /** Pass to `loadMore`. Null when this is the last page. */
  nextCursor: string | null;
}

export function adaptEvents(raw: RawToolEvents): ToolEventsView {
  return {
    entries: (raw.events ?? []).map(toEntry),
    nextCursor: raw.nextCursor ?? null,
  };
}

/**
 * Name each row's model from the catalogue, where the gateway did not.
 *
 * Applied at render rather than in {@link adaptEvents}, because the two reads
 * race: the feed can land before the catalogue, and rows adapted once would
 * keep their ids until the next page. Mapping the held entries against whatever
 * the catalogue says now means the names appear the moment it lands.
 *
 * The gateway's own label still wins. A row whose `model` differs from its
 * `modelId` was labelled upstream (`RawEvent.modelName`), and this does not
 * second-guess it; a row still printing its id takes the catalogue's name for
 * that id, and keeps the id when the catalogue has no row for it - a retired
 * model, or one served before the catalogue knew it. "Unknown model" has no id
 * and is left alone. A row whose id named no vendor takes the catalogue's, so
 * the mark can be drawn for a bare native id too.
 *
 * The Overview's Security events table does the same lookup per row inside its
 * Model cell rather than through this: its rows are the wire type, unadapted,
 * and `modelLabelsFor` explains how the ids the two tables carry differ.
 */
export function labelEntries(entries: ActivityEntry[], labels: ModelLabels): ActivityEntry[] {
  return entries.map((e) => {
    if (e.modelId === null) return e;
    const label = labels(e.modelId);
    if (!label) return e;
    const model = e.model === e.modelId ? label.name : e.model;
    const vendor = e.vendor ?? label.vendor;
    return model === e.model && vendor === e.vendor ? e : { ...e, model, vendor };
  });
}

/**
 * Load one tool's feed, and pages of it on demand.
 *
 * Separate from `useActivity` rather than folded into it, which is the opposite
 * call to the one made for the tool *scope*: that shared a request shape and a
 * race, this does not. A feed pages and accumulates; an overview replaces itself
 * wholesale. Sharing the generation guard between them would mean a "load more"
 * and a scope change fighting over the same ref.
 *
 * No disk cache, deliberately: the held reading is one slot and belongs to the
 * Overview. See `activity_cache.rs`.
 */
export function useToolEvents(
  enabled: boolean,
  tool: string | null,
  installId: string | null = null,
  credential = "",
): {
  view: ToolEventsView | null;
  failure: ActivityFailure | null;
  loading: boolean;
  loadMore: () => void;
  reload: () => void;
  /** Whether `loadMore` has extended the list past page one. A `reload` puts
   *  page one back in its place, so a caller refreshing on its own initiative
   *  rather than the user's checks this first: pulling pages out from under
   *  someone reading them is worse than a stale first page. Set when the
   *  further page lands, not when it is asked for: a refused page extended
   *  nothing, and must not stop the list following traffic. */
  paged: boolean;
} {
  const [view, setView] = useState<ToolEventsView | null>(null);
  const [failure, setFailure] = useState<ActivityFailure | null>(null);
  const [loading, setLoading] = useState(false);
  const [paged, setPaged] = useState(false);
  /** Which scope is current, so a page that arrives after the user has moved on
   *  is dropped rather than appended to a different tool's feed. */
  const attempt = useRef(0);

  const fetchPage = useCallback(
    (cursor: string | null) => {
      if (!enabled || !tool) return;
      const mine = ++attempt.current;
      setLoading(true);
      setFailure(null);
      activityToolEvents(tool, installId ?? undefined, cursor ?? undefined)
        .then((text) => {
          if (mine !== attempt.current) return;
          const page = adaptEvents(JSON.parse(text) as RawToolEvents);
          // Append when paging, replace when starting over. `cursor` is what
          // distinguishes the two, so "load more" cannot silently reset the list.
          setView((prev) =>
            cursor && prev
              ? { entries: [...prev.entries, ...page.entries], nextCursor: page.nextCursor }
              : page,
          );
          if (cursor) setPaged(true);
        })
        .catch((e) => {
          if (mine !== attempt.current) return;
          // The pages already loaded stay: they are a real reading of this same
          // scope, and dropping them because the *next* page failed loses what
          // the user was reading.
          setFailure(toFailure(e));
        })
        .finally(() => {
          if (mine === attempt.current) setLoading(false);
        });
    },
    // `credential` is not read in the body and belongs here anyway: it is the
    // refetch trigger for an account switch, matching `useActivity`. An
    // `exhaustive-deps` autofix would drop it as unused and silently stop the feed
    // re-reading when the user changes account, leaving one account's requests on
    // screen under another's.
    [enabled, tool, installId, credential],
  );

  // A feed belongs to the scope it was read for, so a scope change drops it
  // rather than leaving one tool's requests under another tool's name.
  useEffect(() => {
    setView(null);
    setFailure(null);
    setPaged(false);
  }, [credential, installId, tool]);

  useEffect(() => {
    fetchPage(null);
  }, [fetchPage]);

  return {
    view,
    failure,
    loading,
    // Guarded here rather than at the call site. With no cursor `fetchPage`
    // re-reads page one and *replaces* the list, so a `loadMore` on the last page
    // would silently discard every page already loaded. The pane happens to hide
    // the control when `nextCursor` is null, but that is the pane being careful
    // about a hazard the hook should not have.
    loadMore: () => {
      if (view?.nextCursor) fetchPage(view.nextCursor);
    },
    reload: () => {
      setPaged(false);
      fetchPage(null);
    },
    paged,
  };
}
