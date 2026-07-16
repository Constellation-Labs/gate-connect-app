import { useEffect, useState } from "react";
import { check, type Update } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";
import { getVersion } from "@tauri-apps/api/app";
import { Button } from "./gc/ui";
import { Icon } from "./gc/Icon";

/** Checks for an app update on mount; if one exists, takes over the popover
 *  with a full-panel prompt offering a one-click in-app update. "Install &
 *  relaunch" downloads and installs the signed bundle, then relaunches into
 *  the new version. The check is silent - offline or an unreachable endpoint
 *  shows nothing, and the panel stays up until the user installs or picks
 *  "Later", so it can't be scrolled past. */
export function UpdatePanel() {
  const [update, setUpdate] = useState<Update | null>(null);
  const [current, setCurrent] = useState("");
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
    getVersion()
      .then((v) => {
        if (alive) setCurrent(v);
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
    <div className="gc-panel-in absolute inset-0 z-20 flex flex-col items-center justify-center gap-5 bg-gc-surface px-7 text-center">
      <div className="flex h-14 w-14 items-center justify-center rounded-gc-lg bg-gc-accent-wash text-gc-accent">
        <Icon name="refresh" size={26} />
      </div>

      <div className="flex flex-col gap-1.5">
        <h1 className="text-[17px] font-semibold tracking-[-0.01em] text-gc-ink">
          Update ready
        </h1>
        <p className="text-[12.5px] leading-snug text-gc-ink-3">
          {failed
            ? "The update couldn’t install. Check your connection and try again."
            : "A new version of Gate Connect is ready to install."}
        </p>
      </div>

      <div className="flex items-center gap-2 font-mono text-[11.5px]">
        {current && <span className="text-gc-ink-4">v{current}</span>}
        {current && <Icon name="chevronRight" size={13} className="text-gc-ink-5" />}
        <span className="rounded-gc-pill bg-gc-highlight px-2 py-0.5 font-medium text-gc-ink">
          v{update.version}
        </span>
      </div>

      <div className="mt-1 flex w-full flex-col gap-2">
        <Button
          variant="accent"
          full
          disabled={installing}
          onClick={() => {
            void install();
          }}
        >
          {installing ? "Installing…" : failed ? "Retry update" : "Install & relaunch"}
        </Button>
        {!installing && (
          <button
            type="button"
            onClick={() => setDismissed(true)}
            className="text-[12.5px] font-medium text-gc-ink-3 transition hover:text-gc-ink"
          >
            Later
          </button>
        )}
      </div>
    </div>
  );
}
