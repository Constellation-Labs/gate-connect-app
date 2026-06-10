import { useEffect, useState } from "react";
import { check, type Update } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";
import { IconButton } from "./gc/ui";

type Phase = "idle" | "installing" | "error";

/** Checks for an app update on mount; if one exists, renders a slim banner
 *  asking the user to confirm before downloading, installing, and relaunching.
 *  The check is silent — offline or unreachable endpoint shows nothing. */
export function UpdateBanner() {
  const [update, setUpdate] = useState<Update | null>(null);
  const [dismissed, setDismissed] = useState(false);
  const [phase, setPhase] = useState<Phase>("idle");

  useEffect(() => {
    let alive = true;
    check()
      .then((u) => {
        if (alive && u) setUpdate(u);
      })
      .catch(() => undefined);
    return () => {
      alive = false;
    };
  }, []);

  if (!update || dismissed) return null;

  const install = async () => {
    setPhase("installing");
    try {
      await update.downloadAndInstall();
      await relaunch();
    } catch {
      setPhase("error");
    }
  };

  return (
    <div className="flex shrink-0 items-center gap-2 border-b border-gc-line bg-gc-sunken py-1.5 pl-3.5 pr-1.5">
      <div className="min-w-0 flex-1 text-[11.5px] text-gc-ink-2">
        {phase === "error" ? (
          "Update failed — try again later"
        ) : (
          <>
            Update available · <span className="font-mono">v{update.version}</span>
          </>
        )}
      </div>
      {phase !== "error" && (
        <button
          type="button"
          onClick={install}
          disabled={phase === "installing"}
          className="shrink-0 text-[11.5px] font-semibold text-gc-ink underline decoration-gc-line-strong underline-offset-2 transition hover:decoration-gc-ink-3 disabled:opacity-50"
        >
          {phase === "installing" ? "Installing…" : "Update"}
        </button>
      )}
      <IconButton
        icon="x"
        size={13}
        onClick={() => setDismissed(true)}
        aria-label="Dismiss update"
      />
    </div>
  );
}
