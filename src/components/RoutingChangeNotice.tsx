import { useRef, useState } from "react";
import { closeRunningAgents, type ClosedAgents } from "../lib/api";
import { track, trackError } from "../lib/analytics";
import { classifyError, type ClassifiedError } from "../lib/errors";
import { Takeover, TAKEOVER_Z } from "./Takeover";
import { Button, ErrorNote } from "./gc/ui";
import { Icon } from "./gc/Icon";

/** Full-popover takeover shown when the user flips routing from the home
 *  screen while tools and apps are running. They keep the connection they
 *  resolved at their own launch, so offer to restart them: desktop apps quit
 *  and come back, and terminal tools close for the user to start again.
 *  Routing that comes back on its own at startup is NOT this surface -
 *  that's the calm inline hint on Home. Sits under the
 *  UpdatePanel takeover (z-20) so an update prompt still wins. */
export function RoutingChangeNotice({
  routingOn,
  startConfirming = false,
  onDismiss,
  onAgentsClosed,
}: {
  routingOn: boolean;
  /** Open directly on the close-agents confirm step - used when the entry
   * point (the Home banner's "Restart them…") already declared the intent,
   * so the informational step would just be a third click. */
  startConfirming?: boolean;
  onDismiss: () => void;
  /** Fired after a successful restart, so a surface that opened this takeover
   * (the Home startup banner) can retire advice the user just acted on. */
  onAgentsClosed?: () => void;
}) {
  const [closing, setClosing] = useState(false);
  // Clicking "Restart them…" arms this inline confirm step first; the
  // popover never stacks dialogs, so the panel itself swaps its copy/buttons.
  const [confirming, setConfirming] = useState(startConfirming);
  // What the restart did once it ran; null until then.
  const [closed, setClosed] = useState<ClosedAgents | null>(null);
  const [error, setError] = useState<ClassifiedError | null>(null);
  // Focus the way out, not the way through: this panel can be reached by
  // pressing Enter on the Home banner, and its primary is "Restart them".
  const safeRef = useRef<HTMLButtonElement>(null);
  // `confirming`/`closed` are the step: each swaps the buttons out.

  async function closeAgents() {
    setClosing(true);
    setError(null);
    try {
      // `closed === null` is the not-yet-run sentinel, so a nullish resolve
      // would leave the confirm step up with no feedback.
      const result = (await closeRunningAgents()) ?? NOTHING_CLOSED;
      setClosed(result);
      track("agents_closed", { count: result.closed, restarted: result.restarted.length });
      onAgentsClosed?.();
    } catch (e) {
      trackError(e, "close_agents");
      setError(classifyError(e, "close_agents"));
    } finally {
      setClosing(false);
    }
  }

  return (
    <Takeover
      z={TAKEOVER_Z.routing}
      labelledBy="routing-notice-title"
      onEscape={onDismiss}
      initialFocus={safeRef}
      resetKey={`${confirming}:${closed !== null}`}
    >
      {/* The tile follows the step, not just the routing direction. On the
          confirm step the panel is asking to close the user's running apps and
          pairs that with a red `danger` button, so a shieldCheck in indigo wash
          put "protected" and "destroy" 40px apart in the same panel. `info` in
          warning wash is the glyph this system already uses to ask before an
          irreversible step (Home's certificate card picks it for the same
          reason: not repeating a shield that means something else). */}
      <div
        className={`flex h-14 w-14 items-center justify-center rounded-gc-lg ${
          confirming && closed === null
            ? "bg-gc-warning-wash text-gc-warning"
            : routingOn
              ? "bg-gc-accent-wash text-gc-accent"
              : "bg-gc-sunken text-gc-ink-3"
        }`}
      >
        <Icon name={confirming && closed === null ? "info" : "shieldCheck"} size={26} />
      </div>

      <div className="flex flex-col gap-1.5">
        <h1
          id="routing-notice-title"
          className="text-gc-panel-title font-semibold tracking-[-0.01em] text-gc-ink"
        >
          {/* The heading is the `aria-labelledby` target, so it has to move
              when the step does. It used to read "Routing is off" through all
              three steps, which meant a screen reader entering the confirm
              heard no change at all. */}
          {confirming && closed === null
            ? "Restart the tools and apps that are running?"
            : routingOn
              ? "Routing is on"
              : "Routing is off"}
        </h1>
        {/* Informational state, so ink - error red stays reserved for
            failures (the ErrorNote below). */}
        {closed === null ? (
          <p className="text-gc-body-sm leading-snug text-gc-ink-3">
            {confirming
              ? // Not "Restart everything still running": this restarts the
                // agent process set, so a routed app outside it (ChatGPT
                // desktop) is untouched and that wording would tell the user it
                // had been handled. Says which half comes back on its own,
                // because a terminal tool cannot: it belongs to its terminal.
                "Desktop apps like Claude quit and open again. Tools running in a terminal close, and you start them again yourself. Anything they’re working on will be interrupted."
              : routingOn
                ? "Tools and apps that were already open aren’t routing through Gate yet. Restart them and they pick Gate up."
                : "Tools and apps that were already open still point at Gate. Restart them and they go back to their own settings."}
          </p>
        ) : (
          // The one line that reports the result of a destructive action, and
          // it arrives by swapping a <p> inside an already-open dialog. Without
          // a live region nothing announces it, so a screen-reader user closes
          // every running agent and hears nothing back.
          <p role="status" aria-live="polite" className="text-gc-body-sm leading-snug text-gc-ink-3">
            {closedSummary(closed)}
          </p>
        )}
        {error && <ErrorNote error={error} />}
      </div>

      <div className="mt-1 flex w-full flex-col gap-2">
        {closed === null && !confirming && (
          <>
            <Button variant="accent" full onClick={onDismiss}>
              Got it
            </Button>
            {/* Ellipsis, matching Home's banner: same action, different entry
                point, and both land on the confirm step rather than acting. */}
            <Button variant="secondary" full onClick={() => setConfirming(true)}>
              Restart them…
            </Button>
          </>
        )}
        {closed === null && confirming && (
          <>
            {/* Not accent: this interrupts the user's in-flight work, and
                step 1 already trained the reflex to hit the accent button.
                Same grammar as Settings' Reset. Cancel is a full secondary
                button, not a text link, so the safe option is its equal. */}
            <Button variant="danger" full disabled={closing} onClick={() => void closeAgents()}>
              {closing ? "Restarting…" : "Restart them"}
            </Button>
            <Button
              ref={safeRef}
              variant="secondary"
              full
              disabled={closing}
              onClick={() => setConfirming(false)}
            >
              Cancel
            </Button>
          </>
        )}
        {closed !== null && (
          <Button variant="accent" full onClick={onDismiss}>
            Done
          </Button>
        )}
      </div>
    </Takeover>
  );
}

const NOTHING_CLOSED: ClosedAgents = {
  closed: 0,
  restarted: [],
  reopen_yourself: [],
  still_running: [],
};

const LIST = new Intl.ListFormat("en", { style: "long", type: "conjunction" });

/** The one line reporting what the restart did: what came back on its own,
 *  then what the user has to start again, by name, so "start them again"
 *  never leaves them guessing which ones, then what would not quit at all -
 *  still open, so neither "closed" nor "nothing was running" is true of it. */
export function closedSummary({
  closed,
  restarted,
  reopen_yourself,
  still_running,
}: ClosedAgents): string {
  if (closed === 0 && still_running.length === 0) return "Nothing was running.";
  const pronoun = reopen_yourself.length === 1 ? "it" : "them";
  const stuck = still_running.length === 1 ? "it" : "them";
  const parts = [
    restarted.length > 0 && `Restarted ${LIST.format(restarted)}.`,
    reopen_yourself.length > 0 &&
      (restarted.length > 0
        ? `Start ${LIST.format(reopen_yourself)} again when you need ${pronoun}.`
        : `Closed ${LIST.format(reopen_yourself)}. Start ${pronoun} again when you need ${pronoun}.`),
    still_running.length > 0 &&
      `${LIST.format(still_running)} didn’t quit. Quit ${stuck} and open ${stuck} again.`,
  ];
  return parts.filter(Boolean).join(" ") || `Closed ${closed} ${closed === 1 ? "app" : "apps"}.`;
}
