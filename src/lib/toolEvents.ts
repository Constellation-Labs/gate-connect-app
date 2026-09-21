import { useCallback, useEffect, useRef, useState } from "react";
import { activityToolEvents } from "./api";
import { toFailure, type ActivityFailure } from "./activity";
import type { ActivityEntry } from "./toolEventRow";
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
 * The vendor namespace of a canonical model id, for a row the gateway named no
 * provider on.
 *
 * `provider` is null far more often than it looks: the column holds the
 * pipeline's `unknown` sentinel on a substantial share of rows - the gateway's
 * own comment counts 42 of 102 in a dev database - and the endpoint maps that
 * to null rather than let a client draw a mark for a provider nobody has. The
 * mark slot then rendered an empty spacer, which is what "the Model column has
 * no vendor icon" turned out to be: not a missing mark, a missing reading.
 *
 * But the vendor is right there in the id. Model ids are canonical
 * `provider/model` (`anthropic/claude-opus-5`), and `NewUiApp` already splits
 * them this way in two places for the same reason. Deriving it is not guessing:
 * `providerMarkFor` returns undefined for a namespace it has no mark for, so a
 * spelling this does not recognise falls back to the cube exactly as before.
 *
 * Only the namespace of a *namespaced* id. A bare `gpt-5` names no vendor, and
 * inventing one from the model family is the kind of guess principle 6 forbids.
 */
function vendorFromModelId(model: string | null | undefined): string | null {
  if (!model) return null;
  const slash = model.indexOf("/");
  if (slash <= 0) return null;
  return model.slice(0, slash);
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
 * The glyph's ink, per category, as `card/recent-activity` draws it
 * (`661:16450`).
 *
 * Measured off that frame and matched against the variables it resolves:
 * Injection `tailwind colors/red/600` #dc2626, PII `green/600` #16a34a,
 * Credential `purple/600` #9333ea. None of those ramps is redefined in
 * `tailwind.config.ts` - only `blue` is - so the classes render the values the
 * variables name.
 *
 * The column drew every glyph at `base.foreground` before this, and the reason
 * is worth recording: the ink was taken from the *exported asset's* own stroke,
 * which is #030712, rather than from the glyph as the frame renders it. An
 * export is not the frame.
 *
 * `phi` shares PII's ink as well as its glyph: the gateway scans them from one
 * criterion pair and the frame draws no PHI row, so splitting them here would
 * invent a distinction the taxonomy does not make. Recorded as inferred rather
 * than drawn. Anything else - `other`, an unrecognised spelling, and our own
 * "Regular" - keeps `base.foreground`: a colour is what a guardrail firing
 * looks like, and those are the cases where none did.
 */
const CATEGORY_TONES: Record<string, string> = {
  pii: "text-green-600",
  phi: "text-green-600",
  "pii-phi": "text-green-600",
  injection: "text-red-600",
  "prompt-injection": "text-red-600",
  credential: "text-purple-600",
  credentials: "text-purple-600",
};

/**
 * The ink for a category, or `base.foreground` where the frame draws none.
 *
 * `Object.hasOwn` and not a bare index, for the reason `providerNameFor` gives
 * at length: the key is a gateway string, and `constructor` and `__proto__`
 * both survive `toLowerCase()` and come back truthy off `Object.prototype`, so
 * `??` never fires and the class attribute is handed a function. This shipped
 * guarded in `ProviderMark` and unguarded here, in the same change.
 */
export function categoryTone(category: string | null): string {
  if (category === null) return "text-base-foreground";
  return lookup(CATEGORY_TONES, category) ?? "text-base-foreground";
}

/**
 * One normalisation for both category tables.
 *
 * The glyph and its ink have to agree, and they did not: this function's
 * callers used to disagree about case, so a row the gateway spelled `PII` took
 * the fallback *shield* in PII *green* - a combination neither table intends.
 * A PR whose premise is that spellings drift between producers is the wrong
 * place to normalise one lookup and not its neighbour.
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
    model: raw.model ?? NO_MODEL,
    provider: raw.provider,
    vendor: raw.provider ?? vendorFromModelId(raw.model),
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
