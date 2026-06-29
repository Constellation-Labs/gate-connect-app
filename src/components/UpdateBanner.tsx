import { useEffect, useState } from "react";
import { check, type Update } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";
import { IconButton } from "./gc/ui";

/** Checks for an app update on mount; if one exists, renders a slim banner
 *  offering a one-click in-app update. "Update" downloads and installs the
 *  signed bundle, then relaunches into the new version. The check is silent -
 *  offline or an unreachable endpoint shows nothing. */
export function UpdateBanner() {
  const [update, setUpdate] = useState<Update | null>(null);
  const [dismissed, setDismissed] = useState(false);
  const [installing, setInstalling] = useState(false);
  const [failed, setFailed] = useState(false);

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

  async function install() {
    if (!update) return;
    setFailed(false);
    setInstalling(true);
    try {
      await update.downloadAndInstall();
      await relaunch();
    } catch {
      setInstalling(false);
      setFailed(true);
    }
  }

  return (
    <div className="flex shrink-0 items-center gap-2 border-b border-gc-line bg-gc-sunken py-1.5 pl-3.5 pr-1.5">
      <div className="min-w-0 flex-1 text-[11.5px] text-gc-ink-2">
        {failed ? (
          "Update failed · try again"
        ) : (
          <>
            Update available · <span className="font-mono">v{update.version}</span>
          </>
        )}
      </div>
      <button
        type="button"
        disabled={installing}
        onClick={() => {
          void install();
        }}
        className="shrink-0 text-[11.5px] font-semibold text-gc-ink underline decoration-gc-line-strong underline-offset-2 transition hover:decoration-gc-ink-3 disabled:no-underline disabled:opacity-60"
      >
        {installing ? "Installing…" : failed ? "Retry" : "Update"}
      </button>
      {!installing && (
        <IconButton
          icon="x"
          size={13}
          onClick={() => setDismissed(true)}
          aria-label="Dismiss update"
        />
      )}
    </div>
  );
}
