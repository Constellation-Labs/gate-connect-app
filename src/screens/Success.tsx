import { PopHeader } from "../components/gc/PopHeader";
import { ConstellationHexMark } from "../components/gc/ConstellationHexMark";
import { Button } from "../components/gc/ui";
import { Icon } from "../components/gc/Icon";

/** Connected confirmation shown right after first-run. When routing is off,
 * the primary action finishes the job the copy promises (turn routing on)
 * instead of dead-ending in a "Done". */
export function Success({
  workspace,
  proxyOn,
  showProxy,
  busy,
  onTurnOnRouting,
  onDone,
  onOpenSettings,
}: {
  workspace: string;
  proxyOn: boolean;
  showProxy: boolean;
  busy: boolean;
  onTurnOnRouting: () => void;
  onDone: () => void;
  onOpenSettings: () => void;
}) {
  const offerRouting = showProxy && !proxyOn;
  return (
    <div className="flex flex-col">
      <PopHeader
        workspace={workspace}
        pill={proxyOn ? "connected" : "idle"}
        onGear={onOpenSettings}
      />
      <div className="flex flex-col items-center px-5 pb-5 pt-4 text-center">
        <div className="relative mb-3">
          <ConstellationHexMark size={46} />
          <span className="absolute -bottom-1 -right-1 flex h-[18px] w-[18px] items-center justify-center rounded-full bg-gc-success text-white ring-2 ring-white">
            <Icon name="check" size={11} stroke={3} />
          </span>
        </div>
        <h1
          tabIndex={-1}
          data-screen-focus
          className="text-gc-panel-title font-semibold tracking-[-0.02em] text-gc-ink outline-none"
        >
          {/* The heading claims only what finished. It used to read "You're
              connected" under a header pill reading "Routing off", which is the
              same contradiction Home's pill logic was rewritten to avoid: the
              account is linked, routing is not on, and the two facts were
              landing 40px apart in opposite directions. */}
          {proxyOn ? "You’re connected" : "Account connected"}
        </h1>
        <p className="mt-1.5 max-w-[280px] text-gc-body-sm leading-[1.45] text-gc-ink-3">
          {/* Three states, not two. The middle one is the reason this split
              matters: with no proxy subsystem to report on, the old copy still
              claimed to be "routing your agents through Gate". */}
          {offerRouting
            ? "One step left: turn on routing to send your agents through Gate."
            : proxyOn
              ? "Gate Connect is linked to your workspace and routing your agents through Gate."
              : "Gate Connect is linked to your workspace."}
        </p>
        {offerRouting ? (
          <>
            <Button
              variant="accent"
              full
              className="mt-4"
              disabled={busy}
              onClick={onTurnOnRouting}
            >
              {busy ? "Turning on…" : "Turn on routing"}
            </Button>
            <button
              type="button"
              onClick={onDone}
              className="mt-2.5 text-gc-body-sm font-medium text-gc-ink-3 transition hover:text-gc-ink"
            >
              Not now
            </button>
          </>
        ) : (
          <Button variant="accent" full className="mt-4" onClick={onDone}>
            Done
          </Button>
        )}
      </div>
    </div>
  );
}
