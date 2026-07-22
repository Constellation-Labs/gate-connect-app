import { Button } from "./gc/ui";
import { Icon } from "./gc/Icon";

/** Full-popover takeover shown when routing flips in a way the user must act
 *  on: routing was already on as the app launched (restart persistence
 *  brought it back, or it never went down), or the proxy was toggled from the
 *  home screen. Agents keep the connection they resolved at their own launch,
 *  so tell the user to restart them either way. Sits under the UpdatePanel
 *  takeover (z-20) so an update prompt still wins. */
export function StartupRoutingNotice({
  routingOn,
  onDismiss,
}: {
  routingOn: boolean;
  onDismiss: () => void;
}) {
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
        <p className="text-[12.5px] leading-snug text-gc-error">
          {routingOn
            ? "Restart your AI agents so their traffic routes through Gate."
            : "Restart your AI agents so they connect directly again."}
        </p>
      </div>

      <div className="mt-1 flex w-full flex-col gap-2">
        <Button variant="accent" full onClick={onDismiss}>
          Got it
        </Button>
      </div>
    </div>
  );
}
