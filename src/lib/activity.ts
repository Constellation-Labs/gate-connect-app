import { useCallback, useEffect, useRef, useState } from "react";
import { activityCachedOverview, activityInstallations, activityOverview } from "./api";
import type { MessagesBucket, UsageStats } from "../components/gc/metrics";
import type { Policy, Saving } from "../components/gc/Overview";
import type { IconName } from "../components/gc/Icon";

/**
 * Adapter between `GET /v1/me/activity` and the Overview pane's props.
 *
 * The endpoint speaks in facts (counts, fractions, an amount and a currency);
 * the pane speaks in rendered strings. Everything locale- or copy-shaped is
 * decided here rather than upstream, because a gateway has no business choosing
 * a desktop client's number formatting.
 *
 * The payload arrives as raw JSON text while the contract is still moving. This
 * module is the only place that knows its shape, so tightening it to a generated
 * type later is a change to one file.
 */

/** Per-section health, mirroring the endpoint. */
type SectionState = "ok" | "unavailable";

/**
 * Why a section is missing, from AG-576's taxonomy. Drives the copy and the
 * offered action, which is the whole reason the endpoint sends a cause rather
 * than a bare "unavailable".
 */
export type UnavailableReason =
  | "connectivity"
  | "access"
  | "attribution"
  | "not_configured"
  | "definition_pending";

const REASONS: UnavailableReason[] = [
  "connectivity",
  "access",
  "attribution",
  "not_configured",
  "definition_pending",
];

/**
 * Read a section's cause, narrowing anything unrecognised to `connectivity`.
 *
 * The taxonomy is versioned with the gateway, not with this build, so a newer
 * gateway may name a cause this app has never heard of. Passing it through would
 * reach `activityGaps`, which switches exhaustively over the union and has no
 * branch to fall into: the pane would then render a banner with no text rather
 * than say a section is missing. `connectivity` is the safe narrowing - it offers
 * a retry, which is the one action that cannot be wrong about a cause we cannot
 * name.
 */
function reasonOf(raw: unknown): UnavailableReason {
  return REASONS.includes(raw as UnavailableReason) ? (raw as UnavailableReason) : "connectivity";
}

interface Counter {
  state: SectionState;
  value?: number;
  reason?: UnavailableReason;
}

interface RawBucket {
  hour: string;
  requests: number;
  blocked: number | null;
  flagged: number | null;
  redacted: number | null;
}

interface RawOverview {
  generatedAt: string;
  window: { from: string; to: string };
  org: { orgId: string; name: string | null };
  counters: {
    blockedOrFlagged: Counter;
    needsReview: Counter;
    requestsRouted: Counter;
    tokensSaved: {
      state: SectionState;
      fraction?: number;
      amount?: number;
      currency?: string;
      reason?: UnavailableReason;
    };
  };
  requestsByHour: { state: SectionState; buckets?: RawBucket[]; reason?: UnavailableReason };
  policies: { state: SectionState; rows?: RawRow[]; reason?: UnavailableReason };
  tokenSavings: { state: SectionState; rows?: RawRow[]; reason?: UnavailableReason };
  /** The installation scope the gateway *applied*, echoed back. Optional
   *  because a gateway older than the attribution migration will not send it. */
  installation?: { installId: string | null };
}

interface RawRow {
  id: string;
  label: string;
  /** Absent when the policy states no single mode. See `PolicyAction`. */
  action?: "block" | "flag" | "redact" | "allow";
  enabled: boolean;
}

export interface ActivityView {
  /** The org the gateway resolved from the credential. The only way an API-key
   *  account learns its own org name: those accounts store no org locally. */
  orgName: string | null;
  stats: UsageStats;
  buckets: MessagesBucket[];
  policies: Policy[];
  savings: Saving[];
  /** Rendered as the pane's period label, e.g. "Last 24 hours · 14:03". */
  period: string;
  /** When the gateway computed this reading, as a local clock time - with the
   *  date in front of it when that is not today.
   *
   *  A clock time and not an age: an age has to be recomputed to stay true, and
   *  a "2 minutes ago" that was written twenty minutes ago is a worse lie than
   *  the age it was added to disclose. The date appears only when it has to
   *  because a held reading can outlive the day it was taken on, and "updated
   *  14:03" under yesterday's figures reads as this afternoon. */
  takenAt: string;
  /** Sections that could not be answered, for the pane's alert slot. */
  gaps: { section: string; reason: UnavailableReason }[];
  /** Which sections have no reading behind them, for the surfaces that draw
   *  them.
   *
   *  `gaps` says *why*, in copy, for the notice under the header. This says
   *  *whether*, as a flag, for the card itself - and the two cannot be the same
   *  field, because a card has to choose between "nothing was sent" and "we were
   *  not told", and those look identical in an empty array. Getting it wrong
   *  prints "No messages sent in the last 24hrs" over a section the gateway
   *  declined to answer. */
  missing: { chart: boolean; policies: boolean; savings: boolean };
  /** Which installation this reading covers, as the gateway echoed it, or
   *  `null` for the whole org.
   *
   *  Read from the response rather than from what we asked for: if the two ever
   *  disagree the numbers belong to the gateway's answer, and a label taken from
   *  the request would mislabel them. */
  installId: string | null;
}

/**
 * Why a whole fetch failed, mirroring `activity::FailureCode` in the core crate.
 *
 * Separate from `UnavailableReason`, which is the *gateway's* account of a
 * section it answered but could not fill. These are the client's account of
 * never having got an answer at all.
 */
export type FailureCode =
  | "offline"
  | "signed_out"
  | "no_org"
  | "rejected"
  | "gateway"
  | "unknown";

export interface ActivityFailure {
  code: FailureCode;
  /** The underlying detail, for the diagnostics report rather than the pane. */
  message: string;
}

const FAILURE_CODES: FailureCode[] = [
  "offline",
  "signed_out",
  "no_org",
  "rejected",
  "gateway",
  "unknown",
];

/**
 * Read the command's rejection.
 *
 * `activity_overview` rejects with a JSON envelope so the cause survives the IPC
 * boundary as a code rather than as prose. Anything that is not that envelope -
 * a Tauri plugin error, an unregistered command on an old binary - is genuinely
 * unknown, and its text is kept for diagnostics rather than guessed at.
 *
 * Exported for its tests: it is the seam between a Rust error and a UI decision,
 * and getting it wrong turns every failure into a generic one.
 */
export function toFailure(e: unknown): ActivityFailure {
  const text = typeof e === "string" ? e : String(e);
  try {
    const parsed = JSON.parse(text) as { code?: unknown; message?: unknown };
    if (FAILURE_CODES.includes(parsed.code as FailureCode)) {
      return {
        code: parsed.code as FailureCode,
        message: typeof parsed.message === "string" ? parsed.message : text,
      };
    }
  } catch {
    // Not the envelope. Fall through.
  }
  return { code: "unknown", message: text };
}

/** Icons the design puts on each policy row, keyed by the endpoint's row id. */
const POLICY_ICONS: Record<string, IconName> = {
  "prompt-injection": "shieldBan",
  "pii-phi": "idCard",
  credentials: "key",
};

const SAVINGS_ICONS: Record<string, IconName> = {
  compression: "layers",
  caching: "cube",
};

/**
 * Turn one hour of the endpoint's counts into a chart column.
 *
 * The design stacks four additive segments and labels the blue one "Total
 * messages", which cannot be the grand total or the stack double-counts - the
 * branch that introduced the chart flagged exactly this. So `total` here is
 * "everything not otherwise accounted for": requests minus the three security
 * series, floored at zero because the three count distinct requests per action
 * and one request can draw two actions.
 *
 * Null security counts mean the caller may not see them (see the endpoint's
 * `access` reason). They render as zero-height segments rather than being
 * invented, and the gap is reported separately so the UI can say why.
 */
function toBucket(b: RawBucket): MessagesBucket {
  const blocked = b.blocked ?? 0;
  const flagged = b.flagged ?? 0;
  const redacted = b.redacted ?? 0;
  return {
    // Hour-of-day tick, matching the design's "14". Local time, because the
    // user reads their own clock, not UTC.
    label: String(new Date(b.hour).getHours()),
    total: Math.max(0, b.requests - (blocked + flagged + redacted)),
    blocked,
    flagged,
    redacted,
  };
}

function toRows<T extends { id: string; name: string; icon: IconName; enabled: boolean }>(
  rows: RawRow[] | undefined,
  icons: Record<string, IconName>,
  fallback: IconName,
  extra: (r: RawRow) => Omit<T, "id" | "name" | "icon" | "enabled">,
): T[] {
  return (rows ?? []).map(
    (r) =>
      ({
        id: r.id,
        name: r.label,
        icon: icons[r.id] ?? fallback,
        enabled: r.enabled,
        ...extra(r),
      }) as T,
  );
}

export function adapt(raw: RawOverview): ActivityView {
  const c = raw.counters;
  const gaps: ActivityView["gaps"] = [];
  const note = (section: string, s: { state: SectionState; reason?: UnavailableReason }) => {
    if (s.state !== "ok") gaps.push({ section, reason: reasonOf(s.reason) });
  };
  note("Blocked and flagged", c.blockedOrFlagged);
  note("Tokens saved", c.tokensSaved);
  note("Messages", c.requestsRouted);
  note("Hourly chart", raw.requestsByHour);
  note("Policies", raw.policies);
  note("Savings", raw.tokenSavings);

  const saved = c.tokensSaved;
  const amount = saved.amount ?? 0;
  const takenAt = clockTime(new Date(raw.generatedAt));
  return {
    orgName: raw.org.name,
    stats: {
      // Null, not zero, for a counter the gateway declined: `UsageStats` says
      // why. A counter that answered `0` is a real reading and stays a zero.
      messages: c.requestsRouted.state === "ok" ? (c.requestsRouted.value ?? 0) : null,
      blockedFlagged: c.blockedOrFlagged.state === "ok" ? (c.blockedOrFlagged.value ?? 0) : null,
      // The endpoint sends a fraction; the tile wants whole percent.
      tokensSavedPercent: saved.state === "ok" ? Math.round((saved.fraction ?? 0) * 100) : null,
      // Formatted here, not upstream. `Intl` owns the currency symbol and
      // placement; the leading "+" is the design's own convention for a saving.
      tokensSavedAmount:
        saved.state === "ok"
          ? `+${new Intl.NumberFormat(undefined, {
              style: "currency",
              currency: saved.currency ?? "USD",
            }).format(amount)}`
          : null,
    },
    buckets: (raw.requestsByHour.buckets ?? []).map(toBucket),
    policies: toRows<Policy>(raw.policies.rows, POLICY_ICONS, "shieldCheck", (r) => ({
      // Never defaulted. The gateway omits this when the policy states no mode,
      // and the pipeline then acts per entity or per confidence tier - filling in
      // "flag" would print a verb for enforcement nobody configured.
      action: r.action ?? null,
    })),
    savings: toRows<Saving>(raw.tokenSavings.rows, SAVINGS_ICONS, "layers", () => ({})),
    period: `Last 24 hours · updated ${takenAt}`,
    takenAt,
    gaps,
    missing: {
      chart: raw.requestsByHour.state !== "ok",
      policies: raw.policies.state !== "ok",
      savings: raw.tokenSavings.state !== "ok",
    },
    installId: raw.installation?.installId ?? null,
  };
}

/**
 * A reading's timestamp as the user's own clock reads it, dated when it is not
 * from today. See `ActivityView.takenAt` for why the date is conditional.
 *
 * `now` is a parameter so the boundary is testable without freezing the clock.
 */
export function clockTime(taken: Date, now = new Date()): string {
  const time = taken.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
  if (taken.toDateString() === now.toDateString()) return time;
  return `${taken.toLocaleDateString([], { month: "short", day: "numeric" })}, ${time}`;
}

/**
 * Load the overview once, and on demand.
 *
 * `installId` scopes the reading to one installation; `null` is the whole org.
 * Changing it refetches, because the gateway narrows every section server-side -
 * there is no client-side slice of a payload that only covered one machine.
 *
 * `credential` identifies whose reading this is - the gateway, the org and the
 * credential type. It is not sent anywhere; changing it clears the view and
 * refetches. Numbers belong to the account that was asked for them, so switching
 * org or gateway has to re-read rather than leave the previous org's traffic on
 * screen under the new org's name.
 *
 * **Two reads, not one.** The held reading comes off disk while the network call
 * is in flight, and whichever the pane has is what it draws. AG-576's rule is
 * that a figure on screen is always a figure something actually measured: before
 * this, the first second of every open was zeros-or-dashes and a fetch, and an
 * open with no connectivity never got past it. Now the pane opens on the last
 * thing that really happened and swaps it for the current one when it arrives.
 * The cache only ever answers for its own scope - see `activity_cache.rs` - so
 * this cannot show one org's traffic under another's name.
 *
 * The cached read is ignored once the network answer has landed, in either
 * direction: a slow disk must not overwrite a fresh reading, and a *failed*
 * fetch must not be papered over with older numbers arriving late and looking
 * current.
 *
 * Deliberately not polling. The endpoint sits in the gateway's 100-requests-per-
 * minute throttle bucket, which is keyed on the source address rather than on
 * the credential: inference is exempt from it, but every other Gate Connect user
 * on the same office network or VPN egress is not, so a timer here spends a
 * budget shared with people this window cannot see.
 */
export function useActivity(
  enabled: boolean,
  installId: string | null = null,
  credential = "",
): {
  view: ActivityView | null;
  failure: ActivityFailure | null;
  loading: boolean;
  reload: () => void;
} {
  const [view, setView] = useState<ActivityView | null>(null);
  const [failure, setFailure] = useState<ActivityFailure | null>(null);
  const [loading, setLoading] = useState(false);
  /** Which attempt is current. A reply from an earlier one is dropped rather
   *  than applied, so a scope the user has already moved off cannot repaint the
   *  pane after the fact. */
  const attempt = useRef(0);

  const reload = useCallback(() => {
    if (!enabled) return;
    const mine = ++attempt.current;
    const current = () => mine === attempt.current;
    setLoading(true);
    setFailure(null);
    // Set by whichever way the network call resolves. The cache is a placeholder
    // for an answer that has not arrived; once one has, in either direction, it
    // has nothing left to say.
    let answered = false;

    void activityCachedOverview(installId ?? undefined)
      .then((text) => {
        if (!current() || answered || !text) return;
        setView(adapt(JSON.parse(text) as RawOverview));
      })
      .catch(() => {
        // No held reading, or an unreadable one. Both mean the pane waits for
        // the network, which is what it did before this existed.
      });

    activityOverview(installId ?? undefined)
      .then((text) => {
        answered = true;
        if (!current()) return;
        setView(adapt(JSON.parse(text) as RawOverview));
        setFailure(null);
      })
      .catch((e) => {
        answered = true;
        if (!current()) return;
        // The held view stays: it is a real reading of this same scope, and
        // replacing it with an empty screen loses the last thing the user knows
        // actually happened. Its own timestamp says when it was taken.
        setFailure(toFailure(e));
      })
      .finally(() => {
        if (current()) setLoading(false);
      });
  }, [enabled, installId, credential]);

  // A reading belongs to the scope it was taken for, so a scope change drops it
  // rather than leaving it under the new label until the replacement lands.
  // Separate from `reload` on purpose: the retry button re-reads without
  // blanking numbers that are still the best answer available.
  useEffect(() => {
    setView(null);
    setFailure(null);
  }, [credential, installId]);

  useEffect(reload, [reload]);

  return { view, failure, loading, reload };
}

/** One installation, as the discovery endpoint reports it. */
export interface Installation {
  installId: string;
  /** What to show. The gateway sends the raw id today: a hostname is often a
   *  person's real name, so naming installations is a privacy call nobody has
   *  taken yet. */
  label: string;
  /** Whether this is the machine the app is running on, decided by the gateway
   *  from the id we sent with the request - not by comparing ids here. */
  current: boolean;
  lastSeenAt: string;
  requests: number;
}

interface RawInstallations {
  installations?: Installation[];
  /** The caller's own id, or `null` when it did not identify itself. */
  current?: string | null;
}

/**
 * Load the installations this account has sent traffic from.
 *
 * A failure is not surfaced as a code the way the overview's is. This list only
 * populates a picker, and the pane it sits on has its own reading to show; an
 * empty picker degrades to the org-wide view, which is the default anyway.
 *
 * One failure is expected rather than exceptional: the gateway refuses this route
 * outright for a credential with no user on it, because the list names every
 * machine the org runs. That caller's overview is already declining its traffic
 * sections with a reason on screen, so a picker with nothing to scope would be
 * the second half of a message it has already been given.
 */
export function useInstallations(
  enabled: boolean,
  /** The account this list belongs to; see `useActivity`. */
  credential = "",
): {
  installations: Installation[];
  /** This machine's own id, from the gateway, or `null` if it is unattributed. */
  current: string | null;
} {
  const [installations, setInstallations] = useState<Installation[]>([]);
  const [current, setCurrent] = useState<string | null>(null);

  useEffect(() => {
    if (!enabled) return;
    let live = true;
    // Cleared before the new read, not after it lands: these are one org's
    // machines, and an installation from the org the user just left is a choice
    // the new reading cannot honour.
    setInstallations([]);
    setCurrent(null);
    activityInstallations()
      .then((text) => {
        if (!live) return;
        const raw = JSON.parse(text) as RawInstallations;
        setInstallations(raw.installations ?? []);
        setCurrent(raw.current ?? null);
      })
      .catch(() => {
        // Nothing to say: no list means no picker, and the org-wide reading the
        // pane already shows is still correct.
      });
    return () => {
      live = false;
    };
  }, [enabled, credential]);

  return { installations, current };
}
