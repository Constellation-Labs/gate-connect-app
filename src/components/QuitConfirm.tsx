import { useRef, useState } from "react";
import { disconnectToolsForQuit, quitApp, type PendingQuit } from "../lib/api";
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
 *  tools are still routed through Gate. Two things can happen to a tool when
 *  the app closes, and which one depends on the address its config names, so
 *  the backend tells us per tool (`pending.reverting`): a config naming the
 *  relay or the engine's own port names something that dies with the app, so
 *  a plain quit puts that tool back on its own settings on the way out and
 *  the startup restore reconnects it; a config naming the forwarder keeps
 *  working without Gate, because that process outlives the app. The other
 *  button disconnects everything for the downtime. Sits above the other
 *  takeovers (z-30) - a pending quit decision should never be obscured by an
 *  update prompt or routing notice. */
export function QuitConfirm({
  pending,
  onCancel,
}: {
  pending: PendingQuit;
  onCancel: () => void;
}) {
  const tools = pending.tools;
  const reverting = pending.reverting;
  const keeping = tools.filter((t) => !reverting.includes(t));
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<ClassifiedError | null>(null);
  // The user asked to quit, but Enter on an unread panel should not decide
  // how. Cancel takes focus; both quit paths stay one Tab away.
  const safeRef = useRef<HTMLButtonElement>(null);

  async function turnOffAndQuit() {
    setBusy(true);
    setError(null);
    try {
      await disconnectToolsForQuit();
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
          {/* Says what the plain quit will do to each tool, by name, because
              it does two different things. A config naming the relay or the
              engine's own port dies with the app, so those tools go back to
              their own settings on the way out (and reconnect at the next
              start); a config naming the forwarder keeps working without Gate.
              The backend decides which is which off the configured address,
              and this is the same list `quit_app` will act on. See the
              plain-quit column in docs/routing-architecture.md, "Who does
              what, per event". */}
          {names} still {plural ? "route" : "routes"} through Gate.{" "}
          {reverting.length === 0 ? (
            <>
              If you quit now, {plural ? "they keep" : "it keeps"} working without Gate until
              Gate Connect runs again.
            </>
          ) : (
            <>
              If you quit now, {joinNames(reverting)}{" "}
              {reverting.length > 1 ? "go" : "goes"} back to{" "}
              {reverting.length > 1 ? "their" : "its"} own settings until Gate Connect runs
              again
              {keeping.length > 0 ? (
                <>
                  ; {joinNames(keeping)} {keeping.length > 1 ? "keep" : "keeps"} working without
                  Gate
                </>
              ) : null}
              .
            </>
          )}
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
        {error && <ErrorNote error={error} />}
      </div>

      <div className="mt-1 flex w-full flex-col gap-2">
        <Button variant="accent" full disabled={busy} onClick={() => void turnOffAndQuit()}>
          {busy ? "Working…" : "Disconnect tools and quit"}
        </Button>
        {/* Plain "Quit": this button used to say "without disconnecting", which
            stopped being true when a plain quit began putting relay-named tools
            back on their own settings. The sentence above says exactly what it
            does per tool; the label should not make a second claim. */}
        <Button variant="secondary" full disabled={busy} onClick={() => void quitAnyway()}>
          Quit
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
