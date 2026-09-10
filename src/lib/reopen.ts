import type { RunningAgent, Verdict } from "./api";

/**
 * The vocabulary of the reopen flow: what a tool is doing right now, what the
 * result was, and the one thing left to do about it.
 *
 * Why a module rather than JSX: the same readings appear in four places - the
 * confirmation's subject list, the progress dialog, the shell banner and the
 * tray card - and AG-566 requires them to agree. A row assembled twice is a row
 * that says "Verifying" on one surface and "Reopen required" on another for the
 * same tool, which is the class of bug `lib/groups.ts` documents one level up.
 * `lib/recovery.ts` exists for the same reason, and this file follows its shape.
 *
 * Everything here is a pure function of the DTOs. No fetching and no state: the
 * driver is `useRunningApps`, and a formatter that could change what it formats
 * would be a second place for the flow to advance from.
 */

/**
 * Where one tool is in the sequence.
 *
 * "Stage" rather than "status", the same distinction `lib/recovery.ts` draws:
 * this is a step in an operation the user is watching, and calling it a status
 * would invite comparison with the routing status on the rail, which answers a
 * different question about a different moment.
 */
export type ReopenStage =
  /** Its configuration is being written - the first step, and where a
   *  `retry_application` puts a row back. */
  | "applying"
  /** Its configuration is written and the process it was running when that
   *  happened is still up, so its traffic is still on the old route. The state
   *  the flow opens in, and what the rail calls "Reopen required". */
  | "reopen_required"
  /** Gate is signalling the process. */
  | "closing"
  /** Closed, and now waiting for the person to start it again. The stage every
   *  tool in the registry ends its close at, because none can be relaunched. */
  | "awaiting_reopen"
  /** Gate is launching it. Reachable only for a tool whose `can_reopen` is
   *  true, which no tool is today - see `RunningAgent.can_reopen`. */
  | "reopening"
  /** It is running again and the sweep has not yet said where its traffic
   *  goes. */
  | "verifying"
  /** It is running again, and no sweep is ever going to answer for it: its slug
   *  is a proxy-domain key rather than a registry tool, so `routing_verdicts`
   *  has no row for it. Terminal, and deliberately not filed with the verified
   *  ones - Gate did what it could and says so, rather than claiming a reading
   *  nobody took. See `RunningAgent.verifiable`. */
  | "reopened"
  /** Verified: routing through Gate. */
  | "routing"
  /** Verified: on the tool's own settings, which is the answer when the change
   *  being applied was routing *off*. */
  | "not_routed"
  /** Gate could not close it, so it is still running with its old route. */
  | "close_failed"
  /** The configuration write failed. Nothing was applied. */
  | "config_failed"
  /** It came back, and the check could not confirm where its traffic goes. */
  | "verify_failed";

/** What the stage is, in the user's words. */
export const REOPEN_STAGE_LABEL: Record<ReopenStage, string> = {
  applying: "Applying",
  reopen_required: "Reopen required",
  closing: "Closing",
  awaiting_reopen: "Reopen required",
  reopening: "Reopening",
  verifying: "Verifying",
  reopened: "Reopened",
  routing: "Routing through Gate",
  not_routed: "On its own settings",
  close_failed: "Could not close",
  config_failed: "Configuration failed",
  verify_failed: "Verification failed",
};

/**
 * The line under the stage: what it means for this tool.
 *
 * The label alone leaves the two waiting stages ambiguous. "Reopen required"
 * could be read as Gate being about to do something, and this is the sentence
 * that says the next move is the user's.
 */
export const REOPEN_STAGE_DETAIL: Record<ReopenStage, string> = {
  applying: "Writing this tool's configuration.",
  reopen_required:
    "Running, and still using the route it started with.",
  closing: "Asking this tool to close so it can pick up its new route.",
  awaiting_reopen:
    "Closed. Open it again and Gate will check its route.",
  reopening: "Gate is starting this tool again.",
  verifying: "It is running again. Gate is checking where its traffic goes.",
  reopened:
    "Open again, on the new route. Gate routes this one through the system proxy, so there is no per-tool check to run.",
  routing: "Open, and its traffic is going through Gate.",
  not_routed: "Open, and its traffic is going to its own upstream.",
  close_failed:
    "Gate could not close it, so it is still using the route it started with.",
  config_failed:
    "Gate could not write this tool's configuration, so nothing changed for it.",
  verify_failed:
    "Gate could not confirm where its traffic goes, so it is not claiming either answer.",
};

/**
 * How often to look while the next move is the user's, in milliseconds.
 *
 * One cadence for one wait, shared by the three places that do the looking:
 * `useRunningApps`' watch while the progress dialog is open, and each shell's
 * standing sweep for a `reopen_required` verdict once it has been dismissed.
 * Three copies of this number is how one surface comes to notice a reopen a
 * minute after another already has.
 *
 * Deliberately slower than the cadence for work Gate is doing itself: nothing
 * is in flight here, and walking the process table and probing the relay twenty
 * times a minute for an answer that arrives when someone opens a terminal is
 * cost with no reading behind it.
 */
export const REOPEN_IDLE_WATCH_MS = 10_000;

/** Why any of this is necessary, in one sentence. Shared by every surface that
 *  raises the flow, so the reason cannot be phrased two ways. */
export const WHY_REOPEN =
  "A tool reads its configuration when it starts, so one that was already running keeps the route it launched with until it is opened again.";

/**
 * Waiting on the person, not on Gate.
 *
 * The distinction the progress dialog turns on: a tool sitting here is not
 * mid-operation, so the account of what happened can be drawn around it even
 * though the watch keeps looking - and it keeps looking precisely because this
 * is the stage a reopen resolves from.
 */
export function isWaitingOnUser(stage: ReopenStage): boolean {
  return stage === "reopen_required" || stage === "awaiting_reopen";
}

/** Nothing is in flight for this tool: it has either finished or is waiting for
 *  the user to act. */
export function isResting(stage: ReopenStage): boolean {
  return isTerminal(stage) || isWaitingOnUser(stage);
}

/** Nothing more will happen to this tool on its own. */
export function isTerminal(stage: ReopenStage): boolean {
  return (
    stage === "reopened" ||
    stage === "routing" ||
    stage === "not_routed" ||
    stage === "close_failed" ||
    stage === "config_failed" ||
    stage === "verify_failed"
  );
}

/**
 * How the result is separated (AG-566 AC 9).
 *
 * `awaiting_reopen` is a bucket of its own rather than a failure: nothing went
 * wrong, the tool is simply not open yet, and filing it with the failures would
 * make the ordinary outcome of this flow look like a fault.
 */
export type ReopenBucket =
  | "verified"
  | "reopened"
  | "manual_reopen"
  | "close_failed"
  | "config_failed"
  | "verify_failed";

export const REOPEN_BUCKET_TITLE: Record<ReopenBucket, string> = {
  verified: "Applied and verified",
  reopened: "Reopened, not checked",
  manual_reopen: "Waiting for you to reopen",
  close_failed: "Could not be closed or reopened",
  config_failed: "Configuration failed",
  verify_failed: "Verification failed",
};

export const REOPEN_BUCKET_BLURB: Record<ReopenBucket, string> = {
  verified: "Gate checked these after they came back.",
  reopened:
    "Gate closed and reopened these. They ride the system proxy rather than a config file, so there is no per-tool reading to take.",
  manual_reopen:
    "Their configuration is saved. Open each one and Gate finishes the check.",
  close_failed: "These are still running on the route they started with.",
  config_failed: "Nothing was written for these, so they are unchanged.",
  verify_failed: "These are open, and Gate could not confirm their route.",
};

/** Which bucket a stage lands in, or `null` while the tool is still in flight. */
export function bucketOf(stage: ReopenStage): ReopenBucket | null {
  switch (stage) {
    case "routing":
    case "not_routed":
      return "verified";
    case "reopened":
      return "reopened";
    case "reopen_required":
    case "awaiting_reopen":
      return "manual_reopen";
    case "close_failed":
      return "close_failed";
    case "config_failed":
      return "config_failed";
    case "verify_failed":
      return "verify_failed";
    default:
      return null;
  }
}

/**
 * One action, on one tool.
 *
 * Deliberately not {@link import("./api").VerdictNextAction}: that set answers
 * "this tool is not routing, what now" for a row on the rail, and this one
 * answers "this step of the operation did not land". They overlap on Reopen
 * tool and separate everywhere else.
 */
export type ReopenAction =
  | "reopen_tool"
  | "retry_application"
  | "retry_verification"
  | "use_tool_defaults"
  | "view_diagnostics"
  | "contact_support";

/** AG-566's own words for each, so the control and the ticket agree. */
export const REOPEN_ACTION_LABEL: Record<ReopenAction, string> = {
  reopen_tool: "Reopen tool",
  retry_application: "Retry application",
  retry_verification: "Retry verification",
  use_tool_defaults: "Use tool defaults",
  view_diagnostics: "View diagnostics",
  contact_support: "Contact support",
};

/**
 * What a row offers, in the order it offers it: the thing most likely to fix
 * this stage first, the escape hatch after it, and the two that only ever
 * report - diagnostics and support - last.
 *
 * A resolved row offers nothing. Its stage says the tool is where the user
 * asked it to be, and a button beside that would invite them to redo work that
 * already landed.
 */
export function actionsFor(stage: ReopenStage): ReopenAction[] {
  switch (stage) {
    case "reopen_required":
      // Running, on the old route: the process is the thing in the way, and
      // offering to deal with it is something Gate can actually do.
      return ["reopen_tool", "view_diagnostics"];
    case "awaiting_reopen":
      // Closed, and only the user can start it again. A "Reopen tool" button
      // here would be an instruction dressed up as a control - and so, it turns
      // out, was the "Retry verification" that used to sit here: the watch
      // already re-reads both probes on a tick, and the shells now keep
      // sweeping after this dialog is dismissed, so the reopen moves the row
      // whether or not anybody presses anything. A refresh button beside a
      // reading that refreshes itself teaches the user it does not.
      return ["view_diagnostics"];
    case "close_failed":
      return ["reopen_tool", "view_diagnostics", "contact_support"];
    case "config_failed":
      return ["retry_application", "use_tool_defaults", "view_diagnostics"];
    case "verify_failed":
      return [
        "retry_verification",
        "use_tool_defaults",
        "view_diagnostics",
        "contact_support",
      ];
    default:
      return [];
  }
}

/** One tool as every surface of this flow draws it. */
export interface ReopenTool {
  slug: string;
  name: string;
  /** Can Gate launch it again itself? Straight from the backend, never assumed
   *  - the copy that says who reopens what is built from this. */
  canReopen: boolean;
  /** Is a process for it up right now? */
  running: boolean;
  /** Can the sweep ever answer for it? Straight from the backend, never
   *  assumed. False for the desktop apps, whose slugs are proxy-domain keys the
   *  registry has no row for - and the reason `nextStage` stops those rows at
   *  `reopened` instead of waiting out a verification budget nothing was going
   *  to answer. */
  verifiable: boolean;
  /** Where its traffic goes now, and where its saved configuration asks it to
   *  go. Both null when the sweep could not establish them, and the surfaces
   *  omit the pair rather than inventing half of it: a guessed endpoint here is
   *  a claim about the user's traffic. */
  routeInUse: string | null;
  requestedRoute: string | null;
  stage: ReopenStage;
  /** The backend's own words for the last failure, for a Details disclosure.
   *  Machine output, so it is drawn in mono. */
  error?: string;
}

/**
 * The tools this flow is about, from the process scan and the last sweep.
 *
 * Built from the scan rather than from the list of slugs that were written,
 * because the flow is about *running* tools: one whose config changed while it
 * was closed has nothing to reopen and never appears here. Two processes of one
 * tool collapse to one row - the person reading has one Codex, and a pid is not
 * something they can act on.
 */
export function reopenTools(
  agents: RunningAgent[],
  /** Product name per slug, from `list_tools`. It cannot answer for the two
   *  desktop-app rows - their slugs are proxy-domain keys, so the registry has
   *  no entry - and `RunningAgent.product_name` is what covers those. */
  names: Map<string, string>,
  verdicts: Map<string, Verdict>,
  stage: ReopenStage = "reopen_required",
): ReopenTool[] {
  const seen = new Set<string>();
  const tools: ReopenTool[] = [];
  for (const agent of agents) {
    const slug = agent.slug;
    if (slug === "" || seen.has(slug)) continue;
    seen.add(slug);
    const verdict = verdicts.get(slug);
    tools.push({
      slug,
      // The backend's product name before the OS's: `list_tools` cannot name a
      // proxy-domain slug, and falling through to the raw process name drew a
      // row titled `Claude` for what the user calls Claude Desktop.
      name: names.get(slug) ?? agent.product_name,
      canReopen: agent.can_reopen,
      running: true,
      verifiable: agent.verifiable,
      routeInUse: verdict?.route_in_use ?? null,
      requestedRoute: verdict?.requested_route ?? null,
      stage,
    });
  }
  return tools;
}

/**
 * Whether a process for the tool is up, and whether it is the one Gate asked to
 * close.
 *
 * `stale` is the process that was running when the configuration changed - the
 * backend's own `predates_routing`, so this cannot disagree with the verdict
 * beside it. `fresh` is one started since, which is what a reopen looks like
 * from outside.
 */
export type ReopenPresence = "gone" | "stale" | "fresh";

/** How long a stage may sit before the flow stops calling it progress, counted
 *  in watch ticks. Two for a close, because SIGTERM is asynchronous and a tool
 *  flushing state is not a tool refusing to die; longer for a check, which
 *  waits on a probe of the relay and the session. */
const CLOSE_TICKS = 2;
const VERIFY_TICKS = 10;
/** How long a tool Gate relaunched may stay gone before the row stops claiming
 *  Gate is handling it. Generous next to `CLOSE_TICKS`, because a desktop app's
 *  cold start is seconds and the wrong answer here is telling someone to open an
 *  app that is already opening. */
const REOPEN_TICKS = 8;

/**
 * The stage one tool moves to on a watch tick.
 *
 * Every transition here is driven by evidence, and the two waiting cases are
 * where that matters most:
 *
 * - **A tool that is not running stays waiting, whatever the verdict says.**
 *   The sweep will happily call a closed tool `on`: its config carries Gate's
 *   values, the relay answers and the session is valid, which is everything
 *   `verdict_for` needs. But nothing has read that file, so AG-566 AC 8 is
 *   explicit that it is the reopen that gets verified, not the config. Reading
 *   the verdict here would report a tool as applied and verified while it sits
 *   closed on the user's machine.
 * - **A check that never resolves fails rather than spinning.** `verifying`
 *   with no answer is a state the user can watch forever, and a flow that
 *   cannot say "I could not confirm this" has to pretend it did.
 */
export function nextStage(
  tool: ReopenTool,
  verdict: Verdict | undefined,
  presence: ReopenPresence,
  waited: number,
): ReopenStage {
  if (isTerminal(tool.stage)) return tool.stage;
  if (presence === "stale") {
    // Still the process we asked to close. Give it a moment, then say so.
    return waited >= CLOSE_TICKS ? "close_failed" : "closing";
  }
  if (presence === "gone") {
    // Split on who is putting it back. `awaiting_reopen` reads "Closed. Open it
    // again" - true for a terminal tool, and a lie for an app Gate has already
    // relaunched, which would have the user starting a second copy of something
    // that is mid-launch.
    if (!tool.canReopen) return "awaiting_reopen";
    // Still gone long after Gate launched it: the relaunch did not take, so the
    // row stops claiming Gate has it and hands the move back.
    return waited >= REOPEN_TICKS ? "awaiting_reopen" : "reopening";
  }
  // Back, and nothing is coming: the sweep walks the registry and this slug is
  // not in it. Waiting here is what produced "Verification failed" on macOS
  // every single time, for the one row Gate closes and reopens itself - a
  // failure reported against a check that was never going to run.
  if (!tool.verifiable) return "reopened";
  if (!verdict) {
    return waited >= VERIFY_TICKS ? "verify_failed" : "verifying";
  }
  switch (verdict.state) {
    case "on":
      return "routing";
    case "off":
      return "not_routed";
    case "needs_attention":
      if (verdict.reason === "configuration_changed") return "config_failed";
      // `reopen_required` against a process that started after the change is
      // the sweep and the scan disagreeing, which resolves itself on the next
      // sweep. Kept as verifying until the budget runs out rather than reported
      // as a fault the user cannot act on.
      if (verdict.reason === "reopen_required") {
        return waited >= VERIFY_TICKS ? "verify_failed" : "verifying";
      }
      // A dead relay or a refused session is not this flow's failure: the
      // configuration is applied and the tool is open, and the rail carries
      // that reason where it can also offer the reconnect. From inside this
      // operation it is a check that did not confirm the route.
      return "verify_failed";
    case "not_installed":
      return "verify_failed";
  }
}

/** The rows the result groups, in a fixed order: what worked, what the user
 *  still has to do, then the three failures. Empty buckets are dropped rather
 *  than drawn as a heading with nothing under it. */
export function reopenBuckets(
  tools: ReopenTool[],
): { key: ReopenBucket; title: string; blurb: string; tools: ReopenTool[] }[] {
  const order: ReopenBucket[] = [
    "verified",
    "reopened",
    "manual_reopen",
    "close_failed",
    "config_failed",
    "verify_failed",
  ];
  return order
    .map((key) => ({
      key,
      title: REOPEN_BUCKET_TITLE[key],
      blurb: REOPEN_BUCKET_BLURB[key],
      tools: tools.filter((t) => bucketOf(t.stage) === key),
    }))
    .filter((bucket) => bucket.tools.length > 0);
}

/** Every tool has settled, so the flow can stop watching. */
export function allSettled(tools: ReopenTool[]): boolean {
  return tools.every((t) => isTerminal(t.stage));
}

/** Everything landed and was checked, which is the one outcome that needs no
 *  account of itself - the design draws "Change is ready" for it. */
export function allVerified(tools: ReopenTool[]): boolean {
  return tools.length > 0 && tools.every((t) => bucketOf(t.stage) === "verified");
}
