import { useRef, useState } from "react";
import { closeRunningAgents } from "../lib/api";
import { track, trackError } from "../lib/analytics";
import { classifyError, type ClassifiedError } from "../lib/errors";
import { Takeover, TAKEOVER_Z } from "./Takeover";
import { Button, ErrorNote } from "./gc/ui";
import { Icon } from "./gc/Icon";

/** Full-popover takeover shown when the user flips routing from the home
 *  screen while tools and apps are running. They keep the connection they
 *  resolved at their own launch, so offer to close them; the user starts them
 *  again when ready. Routing that comes back on its own at startup is NOT
 *  this surface - that's the calm inline hint on Home. Sits under the
 *  UpdatePanel takeover (z-20) so an update prompt still wins. */
export function RoutingChangeNotice({
  routingOn,
  startConfirming = false,
  onDismiss,
  onAgentsClosed,
}: {
  routingOn: boolean;
  /** Open directly on the close-agents confirm step - used when the entry
   * point (the Home banner's "Close agents…") already declared the intent,
   * so the informational step would just be a third click. */
  startConfirming?: boolean;
  onDismiss: () => void;
  /** Fired after a successful close, so a surface that opened this takeover
   * (the Home startup banner) can retire advice the user just acted on. */
  onAgentsClosed?: () => void;
}) {
  const [closing, setClosing] = useState(false);
  // Clicking "Close running agents" arms this inline confirm step first; the
  // popover never stacks dialogs, so the panel itself swaps its copy/buttons.
  const [confirming, setConfirming] = useState(startConfirming);
  // Signalled-process count once the close ran; null until then.
  const [closed, setClosed] = useState<number | null>(null);
  const [error, setError] = useState<ClassifiedError | null>(null);
  // Focus the way out, not the way through: this panel can be reached by
  // pressing Enter on the Home banner, and its primary is "Close everything".
  const safeRef = useRef<HTMLButtonElement>(null);
  // `confirming`/`closed` are the step: each swaps the buttons out.

  async function closeAgents() {
    setClosing(true);
    setError(null);
    try {
      // `closed === null` is the not-yet-run sentinel, so a nullish resolve
      // would leave the confirm step up with no feedback.
      const count = (await closeRunningAgents()) ?? 0;
      setClosed(count);
      track("agents_closed", { count });
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
      resetKey={`${confirming}:${closed}`}
    >
      <div
        className={`flex h-14 w-14 items-center justify-center rounded-gc-lg ${
          routingOn ? "bg-gc-accent-wash text-gc-accent" : "bg-gc-sunken text-gc-ink-3"
        }`}
      >
        <Icon name="shieldCheck" size={26} />
      </div>

      <div className="flex flex-col gap-1.5">
        <h1
          id="routing-notice-title"
          className="text-[17px] font-semibold tracking-[-0.01em] text-gc-ink"
        >
          {/* The heading is the `aria-labelledby` target, so it has to move
              when the step does. It used to read "Routing is off" through all
              three steps, which meant a screen reader entering the confirm
              heard no change at all. */}
          {confirming && closed === null
            ? "Close the tools and apps that are running?"
            : routingOn
              ? "Routing is on"
              : "Routing is off"}
        </h1>
        {/* Informational state, so ink - error red stays reserved for
            failures (the ErrorNote below). */}
        {closed === null ? (
          <p className="text-[12.5px] leading-snug text-gc-ink-3">
            {confirming
              ? // Not "Close everything still running": this closes the agent
                // process set, so a routed app outside it (ChatGPT desktop,
                // Cowork) survives and the old wording told the user it had
                // been handled. The desktop-app warning stays because it is
                // true and it is the surprising half - the match lowercases the
                // process name, so macOS Claude Desktop is in scope.
                "Desktop apps like Claude close too, and anything they’re working on will be interrupted."
              : routingOn
                ? "Tools and apps that were already open aren’t routing through Gate yet. Close them and they pick Gate up when you open them again."
                : "Tools and apps that were already open still point at Gate. Close them and they go back to their own settings when you open them again."}
          </p>
        ) : (
          // The one line that reports the result of a destructive action, and
          // it arrives by swapping a <p> inside an already-open dialog. Without
          // a live region nothing announces it, so a screen-reader user closes
          // every running agent and hears nothing back.
          <p role="status" aria-live="polite" className="text-[12.5px] leading-snug text-gc-ink-3">
            {closed > 0
              ? `Closed ${closed} ${closed === 1 ? "app" : "apps"}. Open them again whenever you need them.`
              : "Nothing was running."}
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
              Close them…
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
              {closing ? "Closing…" : "Close them"}
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
