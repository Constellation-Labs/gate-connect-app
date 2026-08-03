import { ConstellationHexMark } from "./ConstellationHexMark";
import { ConnPill, IconButton } from "./ui";

/** Popover header - hex mark + "Gate Connect" wordmark, workspace sub-label,
 *  connection pill, and the settings gear. Port of the prototype's PopHeader. */
export function PopHeader({
  workspace,
  pill = "connected",
  onGear,
}: {
  workspace: string;
  pill?: "connected" | "partial" | "idle" | "signedout";
  onGear?: () => void;
}) {
  return (
    <div className="sticky top-0 z-[5] flex items-center gap-2 bg-gc-surface px-3.5 pb-2 pt-3.5">
      <div className="flex min-w-0 flex-1 flex-col gap-[3px] leading-none">
        <h1
          tabIndex={-1}
          data-screen-focus
          className="inline-flex items-center gap-2 outline-none"
        >
          <ConstellationHexMark size={17} fill="#002a5f" />
          <span className="whitespace-nowrap text-[14.5px] font-semibold tracking-[-0.02em] text-gc-navy">
            Gate <span className="text-gc-accent">Connect</span>
          </span>
        </h1>
        <span className="truncate pl-[25px] font-mono text-[10.5px] text-gc-ink-3">
          {workspace}
        </span>
      </div>
      <div className="ml-auto flex shrink-0 items-center gap-1.5">
        {/* The header pill answers exactly one question: is traffic routing
            through Gate right now? (Signed-in state lives in Settings.)
            "Partly routed" is the honest answer while the CA is untrusted:
            config tools route, proxy-routed apps don't yet. */}
        <ConnPill
          state={pill}
          label={
            pill === "connected"
              ? "Routing on"
              : pill === "partial"
                ? "Needs trust"
                : "Routing off"
          }
        />
        {onGear && (
          <IconButton icon="settings" size={15} onClick={onGear} aria-label="Settings" />
        )}
      </div>
    </div>
  );
}
