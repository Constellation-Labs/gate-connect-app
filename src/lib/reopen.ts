import type { RunningAgent, Verdict } from "./api";

/**
 * The vocabulary of the reopen flow: what a tool is doing right now, what the
 * result was, and the one thing left to do about it.
 *
 * Why a module rather than JSX: the same readings appear in more than one place -
 * the apply-changes dialog's rows and the pane's reopen card - and AG-566
 * requires them to agree. A row assembled twice is a row
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

/**
 * The line under the stage: what it means for this tool.
 *
 * The label alone leaves the two waiting stages ambiguous. "Reopen required"
 * could be read as Gate being about to do something, and this is the sentence
 * that says the next move is the user's.
 */
export const REOPEN_STAGE_DETAIL: Record<ReopenStage, string> = {
  applying: "Writing this tool's configuration.",
  reopen_required: "Running now. It will keep its current route until closed.",
  closing: "Asking this tool to close so it can pick up its new configuration.",
  awaiting_reopen:
    "Closed. Open it again and Gate will check its route.",
  reopening: "Gate is starting this tool again.",
  verifying: "It is running again. Gate is checking where its traffic goes.",
  reopened:
    "Open again, on the new route. Gate routes this one through the system proxy, so there is no per-tool check to run.",
  routing: "Open, and its traffic is going through Gate.",
  not_routed: "Open, and its traffic is going to its own upstream.",
  close_failed:
    "Gate could not close it, so it is still using the settings it started with.",
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

/** One tool as every surface of this flow draws it. */
export interface ReopenTool {
  /** The row's identity, from [`reopenKey`]. Not the slug: a Code-tab session
   *  inside Claude Desktop and a terminal `claude` are one slug and two rows. */
  key: string;
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
 * Which row a scanned process belongs to: its slug and the product name the
 * backend resolved it to. Two processes of one tool share a key; a Code-tab
 * session and a terminal CLI, both `claude-code`, do not.
 */
export function reopenKey(agent: RunningAgent): string {
  return `${agent.slug}:${agent.product_name}`;
}

/**
 * The tools this flow is about, from the process scan and the last sweep.
 *
 * Built from the scan rather than from the list of slugs that were written,
 * because the flow is about *running* tools: one whose config changed while it
 * was closed has nothing to reopen and never appears here. Two processes of one
 * tool collapse to one row - the person reading has one Codex, and a pid is not
 * something they can act on. Two products behind one slug stay two rows
 * ([`reopenKey`]); what a button does is still per slug.
 */
export function reopenTools(
  agents: RunningAgent[],
  /** Product name per slug, from `list_tools`. Only the fallback now: the
   *  backend's `RunningAgent.product_name` is read first. */
  names: Map<string, string>,
  verdicts: Map<string, Verdict>,
  stage: ReopenStage = "reopen_required",
): ReopenTool[] {
  const seen = new Set<string>();
  const tools: ReopenTool[] = [];
  for (const agent of agents) {
    const slug = agent.slug;
    const key = reopenKey(agent);
    if (slug === "" || seen.has(key)) continue;
    seen.add(key);
    const verdict = verdicts.get(slug);
    tools.push({
      key,
      slug,
      // The backend's product name first. It names the process, not just the
      // slug: `list_tools` cannot name the proxy-domain slugs of the two
      // desktop apps, and it calls a Code-tab session inside Claude Desktop
      // "Claude Code", the same as a terminal CLI. The registry name is only
      // for a scan that did not supply one.
      name: agent.product_name || (names.get(slug) ?? ""),
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
 * backend's own `needs_reopen`, so this cannot disagree with the verdict
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

/** Every tool has settled, so the flow can stop watching. */
export function allSettled(tools: ReopenTool[]): boolean {
  return tools.every((t) => isTerminal(t.stage));
}
