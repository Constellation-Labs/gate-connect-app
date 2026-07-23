import { useState } from "react";
import { closeRunningAgents } from "../lib/api";
import { track, trackError } from "../lib/analytics";
import { Button } from "./gc/ui";
import { Icon } from "./gc/Icon";

/** Full-popover takeover shown when routing flips in a way the user must act
 *  on: routing was already on as the app launched (restart persistence
 *  brought it back, or it never went down), or the proxy was toggled from the
 *  home screen. Agents keep the connection they resolved at their own launch,
 *  so offer to close them; the user starts them again when ready. Sits under
 *  the UpdatePanel takeover (z-20) so an update prompt still wins. */
export function StartupRoutingNotice({
  routingOn,
  onDismiss,
}: {
  routingOn: boolean;
  onDismiss: () => void;
}) {
  const [closing, setClosing] = useState(false);
  // Clicking "Close running agents" arms this inline confirm step first; the
  // popover never stacks dialogs, so the panel itself swaps its copy/buttons.
  const [confirming, setConfirming] = useState(false);
  // Signalled-process count once the close ran; null until then.
  const [closed, setClosed] = useState<number | null>(null);
  const [error, setError] = useState<string | null>(null);

  async function closeAgents() {
    setClosing(true);
    setError(null);
    try {
      const count = await closeRunningAgents();
      setClosed(count);
      track("agents_closed", { count });
    } catch (e) {
      trackError(e, "close_agents");
      setError(typeof e === "string" ? e : String(e));
    } finally {
      setClosing(false);
    }
  }

  return (
    <div className="gc-panel-in absolute inset-0 z-10 flex flex-col items-center justify-center gap-5 bg-gc-surface px-7 text-center">
      <div
        className={`flex h-14 w-14 items-center justify-center rounded-gc-lg ${
          routingOn ? "bg-gc-accent-wash text-gc-accent" : "bg-gc-sunken text-gc-ink-3"
        }`}
      >
        <Icon name="shieldCheck" size={26} />
      </div>

      <div className="flex flex-col gap-1.5">
        <h1 className="text-[17px] font-semibold tracking-[-0.01em] text-gc-ink">
          {routingOn
            ? "Gate Connect is redirecting traffic"
            : "Gate Connect stopped redirecting traffic"}
        </h1>
        {closed === null ? (
          <p className="text-[12.5px] leading-snug text-gc-error">
            {confirming
              ? "Close all running agents? Anything they're working on will be interrupted."
              : routingOn
                ? "Agents already running won't route through Gate. Close them and start them again."
                : "Agents already running still point at Gate. Close them and start them again."}
          </p>
        ) : (
          <p className="text-[12.5px] leading-snug text-gc-ink-3">
            {closed > 0
              ? `Closed ${closed} agent${closed === 1 ? "" : "s"}. Start them again when you're ready.`
              : "No running agents found."}
          </p>
        )}
        {error && <p className="text-[12.5px] leading-snug text-gc-error">{error}</p>}
      </div>

      <div className="mt-1 flex w-full flex-col gap-2">
        {closed === null && !confirming && (
          <>
            <Button variant="accent" full onClick={onDismiss}>
              Got it
            </Button>
            <Button variant="secondary" full onClick={() => setConfirming(true)}>
              Close running agents
            </Button>
          </>
        )}
        {closed === null && confirming && (
          <>
            <Button variant="accent" full disabled={closing} onClick={() => void closeAgents()}>
              {closing ? "Closing…" : "Close agents"}
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
