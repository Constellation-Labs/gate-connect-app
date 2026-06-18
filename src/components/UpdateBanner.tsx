import { useEffect, useState } from "react";
import { check, type Update } from "@tauri-apps/plugin-updater";
import { openUrl } from "@tauri-apps/plugin-opener";
import { IconButton } from "./gc/ui";

const RELEASES_URL =
  "https://github.com/Constellation-Labs/gate-connect-app/releases/latest";

/** Checks for an app update on mount; if one exists, renders a slim banner
 *  telling the user a newer version is available and linking to the releases
 *  page so they can download and install it themselves. The check is silent —
 *  offline or unreachable endpoint shows nothing. */
export function UpdateBanner() {
  const [update, setUpdate] = useState<Update | null>(null);
  const [dismissed, setDismissed] = useState(false);

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

  return (
    <div className="flex shrink-0 items-center gap-2 border-b border-gc-line bg-gc-sunken py-1.5 pl-3.5 pr-1.5">
      <div className="min-w-0 flex-1 text-[11.5px] text-gc-ink-2">
        Update available · <span className="font-mono">v{update.version}</span>
      </div>
      <button
        type="button"
        onClick={() => {
          void openUrl(RELEASES_URL);
        }}
        className="shrink-0 text-[11.5px] font-semibold text-gc-ink underline decoration-gc-line-strong underline-offset-2 transition hover:decoration-gc-ink-3"
      >
        Download
      </button>
      <IconButton
        icon="x"
        size={13}
        onClick={() => setDismissed(true)}
        aria-label="Dismiss update"
      />
    </div>
  );
}
