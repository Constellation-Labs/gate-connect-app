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
          className="text-[17px] font-semibold tracking-[-0.01em] text-gc-ink"
        >
          Quit Gate Connect?
        </h1>
        <p className="text-[12.5px] leading-snug text-gc-ink-3">
          {names} still {plural ? "route" : "routes"} through Gate. If you quit now,{" "}
          {plural ? "they" : "it"} can’t connect until Gate Connect runs again.
        </p>
        <p className="text-[11.5px] leading-snug text-gc-ink-3">
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
        {/* Names what it does instead of "Quit anyway", which said only that
            quitting would happen and left the difference between the two quit
            buttons to be inferred. */}
        <Button variant="secondary" full disabled={busy} onClick={() => void quitAnyway()}>
          Quit without disconnecting
        </Button>
        <button
          ref={safeRef}
          type="button"
          disabled={busy}
          onClick={onCancel}
          className="text-[12.5px] font-medium text-gc-ink-3 transition hover:text-gc-ink disabled:opacity-45"
        >
          Cancel
        </button>
      </div>
    </Takeover>
  );
}
