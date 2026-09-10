import { useRef, useState } from "react";
import { disconnectToolsForQuit, quitApp } from "../lib/api";
import { track, trackError } from "../lib/analytics";
import { classifyError, type ClassifiedError } from "../lib/errors";
import { Takeover, TAKEOVER_Z } from "./Takeover";
import { Button, ErrorNote } from "./gc/ui";
import { Icon } from "./gc/Icon";

/** "Claude Code", "Claude Code and Codex", "Claude Code, Codex, and OpenCode". */
function joinNames(names: string[]): string {
  if (names.length <= 1) return names[0] ?? "";
  if (names.length === 2) return `${names[0]} and ${names[1]}`;
  return `${names.slice(0, -1).join(", ")}, and ${names[names.length - 1]}`;
}

/** Full-popover takeover shown when the user picks Quit from the tray while
 *  config-routed CLI tools are still connected. Their configs point at the
 *  loopback relay, which dies with the app, so those tools can't connect
 *  until Gate Connect runs again. Offer to disconnect the tools for the
 *  downtime (snapshot + disconnect, routing intent untouched, so the startup
 *  restore reapplies them) or quit with them in place. Sits above the other
 *  takeovers (z-30) - a pending quit decision should never be obscured by an
 *  update prompt or routing notice. */
export function QuitConfirm({ tools, onCancel }: { tools: string[]; onCancel: () => void }) {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<ClassifiedError | null>(null);
  /** Tools the teardown could not put back. Not an error - the sweep ran - but
   * the panel must not quit while claiming the cleanup finished. */
  const [leftBehind, setLeftBehind] = useState<string[] | null>(null);
  // The user asked to quit, but Enter on an unread panel should not decide
  // how. Cancel takes focus; both quit paths stay one Tab away.
  const safeRef = useRef<HTMLButtonElement>(null);

  async function turnOffAndQuit() {
    setBusy(true);
    setError(null);
    setLeftBehind(null);
    try {
      const failed = await disconnectToolsForQuit();
      if (failed.length > 0) {
        // The sweep ran but left a tool pointing at a relay that dies with this
        // process. Quitting here would strand it silently, so name it and stay
        // open - the user can retry, or quit anyway now knowing what it costs.
        setLeftBehind(failed);
        setBusy(false);
        return;
      }
      track("quit_confirmed", { integrations_disabled: true });
      await quitApp();
    } catch (e) {
      // A failed disconnect can leave tool configs half-reverted; surface it
      // and stay open rather than quitting with routing in an unknown state.
      trackError(e, "quit_disable");
      setError(classifyError(e, "quit_disable"));
      setBusy(false);
    }
  }

  async function quitAnyway() {
    setBusy(true);
    track("quit_confirmed", { integrations_disabled: false });
    await quitApp().catch(() => {});
  }

  const names = joinNames(tools);
  const plural = tools.length > 1;

  return (
    <Takeover
      z={TAKEOVER_Z.quit}
      labelledBy="quit-confirm-title"
      onEscape={onCancel}
      initialFocus={safeRef}
      // `busy` empties the panel of focusable controls.
      resetKey={busy}
    >
      <div className="flex h-14 w-14 items-center justify-center rounded-gc-lg bg-gc-sunken text-gc-ink-3">
        {/* Leaving, not protecting: the shield stays with routing states so
            its meaning doesn't dilute across every takeover. */}
        <Icon name="logOut" size={26} />
      </div>

      <div className="flex flex-col gap-1.5">
        <h1
          id="quit-confirm-title"
          className="text-gc-panel-title font-semibold tracking-[-0.01em] text-gc-ink"
        >
          Quit Gate Connect?
        </h1>
        <p className="text-gc-body-sm leading-snug text-gc-ink-3">
          {names} still {plural ? "route" : "routes"} through Gate. If you quit now,{" "}
          {plural ? "they" : "it"} can’t connect until Gate Connect runs again.
        </p>
        <p className="text-gc-caption leading-snug text-gc-ink-3">
          {/* "when Gate Connect starts again", not "at the next start": the next
              start of what was the open question, and the tool's own next launch
              is the wrong answer. Same phrasing as the notification this choice
              fires, so the two messages the user reads seconds apart agree. */}
          Disconnecting puts {plural ? "their" : "its"} own settings back while
          Gate Connect is closed, then reconnects {plural ? "them" : "it"} when
          Gate Connect starts again.
        </p>
        {leftBehind && (
          // Named, not counted, and the panel stays open: a tool left pointing
          // at Gate will not reach a model once this process exits, so the one
          // thing this must not do is quit quietly.
          <p className="text-gc-body-sm leading-snug text-gc-warning-deep" role="status">
            Couldn’t put {joinNames(leftBehind)} back on{" "}
            {leftBehind.length > 1 ? "their own settings" : "its own settings"}.{" "}
            {leftBehind.length > 1 ? "They still point" : "It still points"} at Gate and
            won’t reach a model until Gate Connect runs again. Try again, or quit knowing
            that.
          </p>
        )}
        {error && <ErrorNote error={error} />}
      </div>

      <div className="mt-1 flex w-full flex-col gap-2">
        <Button variant="accent" full disabled={busy} onClick={() => void turnOffAndQuit()}>
          {busy ? "Working…" : leftBehind ? "Try disconnecting again" : "Disconnect tools and quit"}
        </Button>
        {/* Names what it does instead of "Quit anyway", which said only that
            quitting would happen and left the difference between the two quit
            buttons to be inferred. */}
        <Button variant="secondary" full disabled={busy} onClick={() => void quitAnyway()}>
          Quit without disconnecting
        </Button>
        {/* A full secondary button, not a text link, so the safe option is the
            equal of the two that quit. This panel already focuses Cancel on
            mount, on the reasoning that Enter on an unread panel should not
            decide how to quit; at 12.5px text it measured 304x18.8 under two
            40px buttons, so the control the panel points focus at was also the
            faintest thing on it. RoutingChangeNotice states this rule for its
            own Cancel and this was the higher-stakes takeover breaking it. */}
        <Button
          ref={safeRef}
          variant="secondary"
          full
          disabled={busy}
          onClick={onCancel}
        >
          Cancel
        </Button>
      </div>
    </Takeover>
  );
}
