import type {
  RecoveryNextStep,
  RecoverySummary,
  RecoveryTool,
  RestoreOutcome,
  TeardownTool,
  VerdictReason,
} from "./api";

/**
 * Turning the backend's recovery summary into the rows the notice and the review
 * draw.
 *
 * Why this is a module and not inline JSX: the same four readings appear in three
 * places - the notice's per-tool progress, the read-only review, and the tray's
 * one-line version - and AG-570 requires them to agree. A row assembled twice is
 * a row that can say "Waiting" in one surface and "Failed" in another for the
 * same tool, which is the class of bug `lib/groups.ts` documents one level up.
 *
 * Everything here is a pure function of the DTO. No fetching, no state: the
 * review must not change what it is reviewing, and the cheapest way to guarantee
 * that is for the layer that formats it to have nothing to change.
 */

/** A value the backend could not resolve. Named rather than blank, on the same
 *  reasoning `diagnosticsReport` gives: a hole that reads as a fact about the
 *  machine beats one that reads as a formatting bug. */
const UNKNOWN = "unknown";

/** What the *write* reached, in the user's words.
 *
 *  "Stage" rather than "status" throughout: this is a step in an operation, and
 *  calling it a status would invite the reader to compare it with the routing
 *  status beside it, which answers a different question. */
export const STAGE_LABEL: Record<RestoreOutcome, string> = {
  pending: "Not started",
  restored: "Configuration written",
  write_failed: "Write failed",
  not_installed: "No longer installed",
  unknown: "Not recognised",
  deferred_signed_out: "Waiting for sign-in",
};

/** The one-line detail under a stage. Says what the stage *means* for this tool,
 *  because the label alone leaves "Not started" ambiguous between "nothing
 *  happened to it" and "it is next". */
export const STAGE_DETAIL: Record<RestoreOutcome, string> = {
  pending: "The operation stopped before reaching this one. Its settings are untouched.",
  restored: "Gate's routing values are back in this tool's config.",
  write_failed: "Gate could not write this tool's config. It is still recorded, so a retry picks it up.",
  not_installed: "Not on this machine any more, so there is nothing to restore.",
  unknown: "Recorded by an older version, or a tool since removed. Dropped.",
  deferred_signed_out: "Nothing was attempted: there is no account to point this tool at.",
};

/** Grouped failure kinds, for a review that says what *class* of thing went
 *  wrong rather than printing one sentence per entry. `none` never renders. */
export const ERROR_CATEGORY_LABEL: Record<RecoveryTool["error_category"], string> = {
  none: "",
  write: "Configuration write",
  not_installed: "Tool missing",
  unknown: "Unrecognised entry",
  account: "Account",
};

/** The button on a row. Straight from `recovery::NextStep`, so the control and
 *  the backend's own vocabulary say the same thing. */
export const NEXT_STEP_LABEL: Record<RecoveryNextStep, string> = {
  none: "",
  retry: "Retry",
  sign_in: "Sign in",
  reopen_tool: "Reopen tool",
};

/** The check half of a row: what the last sweep concluded about this tool.
 *
 *  Separate vocabulary from `verdict.ts`'s status line on purpose. That one is
 *  written for a row the user is reading *now* ("Protected", "Not protected -
 *  Reopen required"); this one is written for a reading taken at some point in
 *  the past, so it is phrased as a result rather than as a state. */
const CHECK_LABEL: Record<NonNullable<RecoveryTool["check_state"]>, string> = {
  on: "Routing through Gate",
  off: "Not routed",
  needs_attention: "Needs attention",
  not_installed: "Not installed",
};

const CHECK_REASON_LABEL: Record<VerdictReason, string> = {
  configuration_changed: "config changed outside Gate",
  reopen_required: "reopen required",
  connection_problem: "connection problem",
  access_problem: "access problem",
  verification_failed: "could not be verified",
};

/** A duration in whole units, largest two first. Borrowed shape from
 *  `diagnosticsReport.span` rather than the shared one, because that module's is
 *  private and copying six lines beats exporting a helper that then has two
 *  callers with different rounding needs. */
function span(totalSec: number): string {
  const days = Math.floor(totalSec / 86400);
  const hours = Math.floor((totalSec % 86400) / 3600);
  const minutes = Math.floor((totalSec % 3600) / 60);
  if (days > 0) return `${days}d ${hours}h`;
  if (hours > 0) return `${hours}h ${minutes}m`;
  return minutes > 0 ? `${minutes}m` : "just now";
}

/**
 * How long ago a reading was taken.
 *
 * `0` is the backend's "the clock would not answer", and it renders as unknown
 * rather than as 1970 - the same rule the DTO's own doc comments state. A future
 * timestamp (a clock that moved backwards between the write and now) also reads
 * as `just now` rather than as a negative age.
 */
export function ago(atUnix: number, now: Date): string {
  if (atUnix <= 0) return UNKNOWN;
  const deltaSec = Math.floor(now.getTime() / 1000) - atUnix;
  return deltaSec <= 0 ? "just now" : `${span(deltaSec)} ago`;
}

/** What one row says, once. */
export interface RecoveryRow {
  slug: string;
  name: string;
  kind: "provider" | "tool";
  /** What the write reached, and what that means. */
  stage: string;
  stageDetail: string;
  stageComplete: boolean;
  /** Present only for a stage that failed. */
  errorCategory: string;
  /** "Write failed 4m ago", or just the label when the clock said nothing. */
  stageLine: string;
  /** The last route a check actually established, and when. Null when none ever
   *  has - which is a real answer, not a gap: it means nothing has verified this
   *  tool since Gate started recording. */
  lastVerified: string | null;
  /** The most recent check, whatever it concluded. */
  checkResult: string;
  /** Whether a process is running, and whether it predates the change. */
  runningState: string;
  /** The label of the one action offered, or null when nothing is owed. */
  action: string | null;
  nextStep: RecoveryNextStep;
}

/** One tool's row. `now` is passed in rather than read so the whole summary
 *  renders against a single clock and two rows cannot disagree about "now". */
export function recoveryRow(tool: RecoveryTool, now: Date): RecoveryRow {
  const stage = STAGE_LABEL[tool.stage];
  const at = ago(tool.stage_at_unix, now);
  const check = tool.check_state
    ? tool.check_state === "needs_attention" && tool.check_reason
      ? `${CHECK_LABEL.needs_attention} - ${CHECK_REASON_LABEL[tool.check_reason]}`
      : CHECK_LABEL[tool.check_state]
    : null;
  return {
    slug: tool.slug,
    name: tool.name,
    kind: tool.kind,
    stage,
    stageDetail: STAGE_DETAIL[tool.stage],
    stageComplete: tool.stage_complete,
    errorCategory: ERROR_CATEGORY_LABEL[tool.error_category],
    // "Not started" has no useful timestamp: the entry was seeded when the
    // operation began, and dating it invites the reader to think something
    // happened to that tool then.
    stageLine: tool.stage === "pending" || at === UNKNOWN ? stage : `${stage} ${at}`,
    lastVerified: tool.last_verified_state
      ? `${CHECK_LABEL[tool.last_verified_state]}, ${ago(tool.last_verified_unix, now)}`
      : null,
    checkResult: check ? `${check} (${ago(tool.check_at_unix, now)})` : "Never checked",
    runningState: tool.reopen_pending
      ? "Running, using the settings it started with"
      : tool.running
        ? "Running, with current settings"
        : "Not running",
    action: tool.next_step === "none" ? null : NEXT_STEP_LABEL[tool.next_step],
    nextStep: tool.next_step,
  };
}

/** Every row of a summary, in the order the backend gave them - journal order,
 *  which is the order the operation attempted. */
export function recoveryRows(summary: RecoverySummary, now: Date): RecoveryRow[] {
  return summary.tools.map((t) => recoveryRow(t, now));
}

/** What the operation was trying to do, named so the notice does not have to
 *  say "an operation". */
export function operationLabel(summary: RecoverySummary): string {
  return summary.requested_routing_on
    ? "Turning routing back on"
    : "Turning routing off";
}

/** The header line of the summary: the operation, when it was last touched, and
 *  what it was trying to achieve. AG-570 names all three. */
export function operationLine(summary: RecoverySummary, now: Date): string {
  const when = ago(summary.updated_unix, now);
  const outcome = summary.requested_routing_on
    ? "routing on for every tool it had recorded"
    : "routing off for every tool it had recorded";
  return when === UNKNOWN
    ? `${operationLabel(summary)}. It was trying to leave ${outcome}.`
    : `${operationLabel(summary)}, last updated ${when}. It was trying to leave ${outcome}.`;
}

/** Stage counts for the review's own header. Completed rather than "restored":
 *  a dropped entry is finished too, and counting it as outstanding would ask for
 *  action nobody can take. */
export function stageCounts(summary: RecoverySummary): {
  complete: number;
  pending: number;
  total: number;
} {
  const complete = summary.tools.filter((t) => t.stage_complete).length;
  return {
    complete,
    pending: summary.tools.length - complete,
    total: summary.tools.length,
  };
}

/** Rows that still owe the user something, which is what keeps the notice on
 *  screen. Derived from the offered step rather than from the stage, so a tool
 *  whose write finished but whose process is stale still counts - it is not
 *  routing, and the notice is the only thing saying so. */
export function unresolved(summary: RecoverySummary): RecoveryTool[] {
  return summary.tools.filter((t) => t.next_step !== "none");
}

/** The button on a teardown row. Its own set, not {@link NEXT_STEP_LABEL}: a
 *  teardown's unfinished business is a disconnect that did not land, and
 *  "Retry" alone would not say which direction it retries in. */
export const TEARDOWN_ACTION_LABEL: Record<TeardownTool["next_action"], string> = {
  none: "",
  retry_disconnect: "Retry disconnect",
  reopen_tool: "Reopen tool",
  retry_check: "Retry check",
};
