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
  unknown: "Not recognized",
  deferred_signed_out: "Waiting for sign-in",
  deferred_engine_down: "Waiting for routing",
};

/** The one-line detail under a stage. Says what the stage *means* for this
 *  entry, because the label alone leaves "Not started" ambiguous between
 *  "nothing happened to it" and "it is next".
 *
 *  **Two sets, because a summary row is not always a tool.** The restore walks
 *  providers as well as tools, and every row was getting the tool sentence: a
 *  proxy-only provider - OpenRouter has no `tool_ids` at all - was told "Gate
 *  could not write this tool's config" about a config file it does not have.
 *  `RecoveryTool.kind` has been on the DTO the whole time and nothing read it.
 *
 *  A provider's routing is not one file, so its sentences talk about routing.
 *  That stays true whether the provider configures member tools, rides a proxy
 *  domain, or both, which is why this splits on kind and not on some further
 *  fact the frontend would have to be told. */
const STAGE_DETAIL_TOOL: Record<RestoreOutcome, string> = {
  pending: "The operation stopped before reaching this one. Its settings are untouched.",
  restored: "Gate's routing values are back in this tool's config.",
  write_failed:
    "Gate could not write this tool's config. It is still recorded, so a retry picks it up.",
  not_installed: "Not on this machine any more, so there is nothing to restore.",
  unknown: "Recorded by an older version, or a tool since removed. Dropped.",
  deferred_signed_out: "Nothing was attempted: there is no account to point this tool at.",
  deferred_engine_down:
    "Nothing was attempted: this tool points at the Gate proxy, which was not running yet. It is still recorded, so a resume picks it up.",
};

const STAGE_DETAIL_PROVIDER: Record<RestoreOutcome, string> = {
  pending: "The operation stopped before reaching this one. Nothing about it was changed.",
  restored: "This provider's routing is back on.",
  write_failed:
    "Gate could not restore this provider's routing. It is still recorded, so a retry picks it up.",
  not_installed: "Nothing this provider routes is on this machine any more.",
  unknown: "Recorded by an older version, or a provider since removed. Dropped.",
  deferred_signed_out: "Nothing was attempted: there is no account to route this provider through.",
  deferred_engine_down:
    "Nothing was attempted: this provider routes through the Gate proxy, which was not running yet. It is still recorded, so a resume picks it up.",
};

/** The detail for one row, by what the row is. */
export function stageDetail(stage: RestoreOutcome, kind: RecoveryTool["kind"]): string {
  return kind === "provider" ? STAGE_DETAIL_PROVIDER[stage] : STAGE_DETAIL_TOOL[stage];
}

/** Grouped failure kinds, for a review that says what *class* of thing went
 *  wrong rather than printing one sentence per entry. `none` never renders. */
export const ERROR_CATEGORY_LABEL: Record<RecoveryTool["error_category"], string> = {
  none: "",
  write: "Configuration write",
  not_installed: "Tool missing",
  unknown: "Unrecognized entry",
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
  configuration_overridden: "another config outranks Gate's",
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
  /** The raw outcome, beside its label - the same pairing this row already
   *  makes for `nextStep` and `action`. A surface that needs to group rows by
   *  what happened can then test the outcome rather than match on the sentence
   *  drawn for it. */
  outcome: RestoreOutcome;
  /** What the write reached, and what that means. */
  stage: string;
  stageDetail: string;
  stageComplete: boolean;
  /** Present only for a stage that failed. */
  errorCategory: string;
  /** The backend's own words for the failure, for a Details disclosure. Machine
   *  output, so the surface draws it in mono. Null when the stage did not fail,
   *  and when a build older than the journal field recorded it. */
  error: string | null;
  /** "Write failed 4m ago", or just the label when the clock said nothing. */
  stageLine: string;
  /** The last route a check actually established, and when. Null when none ever
   *  has - which is a real answer, not a gap: it means nothing has verified this
   *  tool since Gate started recording. */
  lastVerified: string | null;
  /** The most recent check, whatever it concluded. */
  checkResult: string;
  /** Whether a process is running, and whether it predates the change - or that
   *  Gate has no way to look, which is a third answer and not the second. */
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
    outcome: tool.stage,
    stage,
    stageDetail: stageDetail(tool.stage, tool.kind),
    stageComplete: tool.stage_complete,
    errorCategory: ERROR_CATEGORY_LABEL[tool.error_category],
    error: tool.error ?? null,
    // "Not started" has no useful timestamp: the entry was seeded when the
    // operation began, and dating it invites the reader to think something
    // happened to that tool then.
    stageLine: tool.stage === "pending" || at === UNKNOWN ? stage : `${stage} ${at}`,
    lastVerified: tool.last_verified_state
      ? `${CHECK_LABEL[tool.last_verified_state]}, ${ago(tool.last_verified_unix, now)}`
      : null,
    // "Never checked" is a fact about a tool and a category error about a
    // provider. The sweep walks the registry, so a provider slug can never have
    // a verdict logged against it - it is checked through its members, and
    // saying nothing has ever checked it invites the reader to go and check it.
    checkResult: check
      ? `${check} (${ago(tool.check_at_unix, now)})`
      : tool.kind === "provider"
        ? "Checked per tool, not per provider"
        : "Never checked",
    // Three answers, not two. `null` is "Gate has no process name for this
    // one", which is the case for a provider slug, for OpenClaw and Hermes, and
    // for the environment channel - and printing "Not running" for those
    // asserted a walk of the process table that never happened.
    runningState: tool.reopen_pending
      ? "Running, using the settings it started with"
      : tool.running === null
        ? "Gate has no process to look for"
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

/**
 * The plain-English half of a row (AG-886).
 *
 * `stageDetail` and the four readings beside it are written for someone holding
 * the codebase: "The operation stopped before reaching this one", "Checked per
 * tool, not per provider", "Gate has no process to look for". AG-570 asked for
 * them and they stay - handing this summary to someone else is still one of the
 * things the review is for - but they belong behind a disclosure rather than in
 * front of a user who wants to know whether their editor is routed.
 *
 * These carry the same facts in the words the reader would use. One set rather
 * than the tool/provider split `stageDetail` makes, because the split exists to
 * avoid promising a config file to a provider that has none, and none of these
 * sentences mentions a config file.
 */
const PLAIN_OUTCOME: Record<RestoreOutcome, string> = {
  pending: "Gate did not reach this one, so nothing about it changed.",
  restored: "This one is set up and routing again.",
  write_failed: "Gate could not save its settings, so it is still on the route it had.",
  not_installed: "This is not on your machine any more, so there was nothing to do.",
  unknown: "Gate no longer recognizes this entry, so it was dropped.",
  deferred_signed_out: "Gate has no account to point this at yet.",
  deferred_engine_down: "Gate's proxy was not running yet, so this one is still waiting.",
};

/** Why this entry is where it is, for the user-facing half of the row. */
export function plainOutcome(stage: RestoreOutcome): string {
  return PLAIN_OUTCOME[stage];
}

/**
 * The one thing to do about a row, as an instruction rather than a control.
 *
 * {@link NEXT_STEP_LABEL} names a button; this says what pressing it is for,
 * which is what AG-886 asks the dialog to answer. The review is read-only by
 * AG-570, so every instruction points at a surface that can actually act - the
 * routing notice's own "Resume now", or the tool itself.
 */
const NEXT_STEP_INSTRUCTION: Record<RecoveryNextStep, string> = {
  none: "",
  retry: "Choose Resume now on the routing notice to finish this.",
  sign_in: "Sign in to your Gate account and this finishes on its own.",
  reopen_tool: "Close it and open it again, and Gate will check its route.",
};

/** What the user should do about a row, or "" when nothing is owed. */
export function plainNextStep(step: RecoveryNextStep): string {
  return NEXT_STEP_INSTRUCTION[step];
}

/**
 * The review's subtitle, in the user's terms (AG-886).
 *
 * {@link operationLine} says what the journal was doing ("It was trying to
 * leave routing on for every tool it had recorded"), which answers a question
 * about Gate's bookkeeping rather than about the reader's machine.
 */
export function plainOperationLine(summary: RecoverySummary, now: Date): string {
  const when = ago(summary.updated_unix, now);
  const what = summary.requested_routing_on ? "turning routing on" : "turning routing off";
  // The notice that opens this dialog is raised by `unresolved`, but the dialog
  // outlives the notice: resuming from underneath it settles every row while it
  // is still on screen, and "did not finish" would then be false.
  const tail =
    unresolved(summary).length === 0 ? "and everything it recorded is done" : "and did not finish";
  return when === UNKNOWN ? `Gate was ${what} ${tail}.` : `Gate was ${what} ${when} ${tail}.`;
}
