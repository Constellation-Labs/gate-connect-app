import { useRef, useState } from "react";
import { closeRunningAgents } from "../lib/api";
import { track, trackError } from "../lib/analytics";
import { classifyError, type ClassifiedError } from "../lib/errors";
import { useFocusTrap } from "../lib/useFocusTrap";
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
  const panelRef = useRef<HTMLDivElement>(null);
  useFocusTrap(panelRef, onDismiss);

  async function closeAgents() {
    setClosing(true);
    setError(null);
    try {
      const count = await closeRunningAgents();
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
    <div
      ref={panelRef}
      role="dialog"
      aria-modal="true"
      aria-labelledby="routing-notice-title"
      className="gc-panel-in absolute inset-0 z-10 flex flex-col items-center justify-center gap-5 bg-gc-surface px-7 text-center"
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
          {routingOn ? "Routing is on" : "Routing is off"}
        </h1>
        {/* Informational state, so ink - error red stays reserved for
            failures (the ErrorNote below). */}
        {closed === null ? (
          <p className="text-[12.5px] leading-snug text-gc-ink-3">
            {confirming
              ? "Close everything still running, including desktop apps like Claude? Anything they're working on will be interrupted."
              : routingOn
                ? "Anything already open isn't routing through Gate yet. Close it and it picks Gate up the next time you open it."
                : "Anything already open still points at Gate. Close it and it goes back to normal the next time you open it."}
          </p>
        ) : (
          <p className="text-[12.5px] leading-snug text-gc-ink-3">
            {closed > 0
              ? `Closed ${closed}. Open them again whenever you need them.`
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
            <Button variant="secondary" full onClick={() => setConfirming(true)}>
              Close them now
            </Button>
          </>
        )}
        {closed === null && confirming && (
          <>
            <Button variant="accent" full disabled={closing} onClick={() => void closeAgents()}>
              {closing ? "Closing…" : "Close everything"}
            </Button>
            <button
              type="button"
              disabled={closing}
              onClick={() => setConfirming(false)}
              className="text-[12.5px] font-medium text-gc-ink-3 transition hover:text-gc-ink disabled:opacity-60"
            >
              Cancel
            </button>
          </>
        )}
        {closed !== null && (
          <Button variant="accent" full onClick={onDismiss}>
            Done
          </Button>
        )}
      </div>
    </div>
  );
}
