import { useEffect, useRef, useState } from "react";
import { check, type Update } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";
import { getVersion } from "@tauri-apps/api/app";
import { setUpdaterRelaunching } from "../lib/api";
import { useWindowReopen } from "../lib/useWindowReopen";
import { useFocusTrap } from "../lib/useFocusTrap";
import { track, trackError } from "../lib/analytics";
import { Button, IconButton } from "./gc/ui";
import { Icon } from "./gc/Icon";

/** Checks for an app update. The first check at startup takes over the popover
 *  with a full-panel prompt. Because the webview stays mounted for the app's
 *  whole life, later checks run each time the tray reopens the popover and
 *  surface as a slim top banner instead - a mid-task reopen shouldn't be
 *  blocked by a takeover. Both offer the same one-click "install & relaunch".
 *  The check is silent - offline or an unreachable endpoint shows nothing. */
export function UpdatePanel() {
  const [update, setUpdate] = useState<Update | null>(null);
  const [current, setCurrent] = useState("");
  const [installing, setInstalling] = useState(false);
  const [failed, setFailed] = useState(false);
  // The startup takeover, dismissed with "Later".
  const [panelDismissed, setPanelDismissed] = useState(false);
  // The reopen banner, dismissed with its close button.
  const [bannerDismissed, setBannerDismissed] = useState(false);
  // Flipped once the window has been reopened (blurred, then refocused); from
  // then on an available update surfaces as the banner, not the startup panel.
  const [reopened, setReopened] = useState(false);

  // Exposure event, once per discovered version - a re-check that returns the
  // same version isn't a new surface.
  const shownVersion = useRef<string | null>(null);
  useEffect(() => {
    if (!update || shownVersion.current === update.version) return;
    shownVersion.current = update.version;
    track("update_shown", { source: reopened ? "banner" : "panel" });
  }, [update, reopened]);

  const runCheck = () =>
    check()
      .then((u) => {
        if (u) setUpdate(u);
      })
      .catch(() => undefined);

  useEffect(() => {
    void runCheck();
    getVersion()
      .then(setCurrent)
      .catch(() => undefined);
  }, []);

  // Re-check on each tray reopen, so an update released while the app sat in
  // the tray still surfaces. From then on an available update surfaces as the
  // banner, not the startup panel - a mid-task reopen shouldn't be blocked by
  // a takeover.
  useWindowReopen(() => {
    setReopened(true);
    void runCheck();
  });

  async function install() {
    if (!update) return;
    setFailed(false);
    setInstalling(true);
    // Download and install as separate phases so the updater-relaunch mark
    // brackets only the install: quitting the app while the (long) download
    // is still running is a genuine user exit, and a set mark there would
    // make the exit handler skip its routing-intent clear and deferred
    // launch-at-login opt-out completion with no relaunch coming.
    try {
      await update.download();
    } catch (err) {
      trackError(err, "update");
      setInstalling(false);
      setFailed(true);
      return;
    }
    try {
      // Mark the coming exit as an updater relaunch so the backend keeps the
      // routing intent and restores routing after the restart. Before the
      // install, not after: on Windows the installer exits the app from
      // inside install().
      await setUpdaterRelaunching(true);
      await update.install();
      // Best-effort: on Windows the installer exits the app from inside
      // install(), so this event may never send there.
      track("update_installed");
      await relaunch();
    } catch (err) {
      trackError(err, "update");
      await setUpdaterRelaunching(false).catch(() => undefined);
      setInstalling(false);
      setFailed(true);
    }
  }

  if (!update) return null;

  // After a reopen, surface the quiet top banner instead of the takeover.
  if (reopened) {
    if (bannerDismissed) return null;
    return (
      <div className="flex shrink-0 items-center gap-2 border-b border-gc-line bg-gc-accent-wash-2 py-1.5 pl-3 pr-1.5">
        <Icon name="refresh" size={13} className="shrink-0 text-gc-accent" />
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
            onClick={() => {
              setBannerDismissed(true);
              track("update_dismissed", { source: "banner" });
            }}
            aria-label="Dismiss update"
          />
        )}
      </div>
    );
  }

  // Startup: full-panel takeover.
  if (panelDismissed) return null;
  return (
    <UpdateTakeover
      version={update.version}
      current={current}
      installing={installing}
      failed={failed}
      onInstall={() => void install()}
      onLater={() => {
        setPanelDismissed(true);
        track("update_dismissed", { source: "panel" });
      }}
    />
  );
}

/** The startup takeover lives in its own component so its focus trap mounts
 *  exactly when the panel does (the parent renders long before, as null). */
function UpdateTakeover({
  version,
  current,
  installing,
  failed,
  onInstall,
  onLater,
}: {
  version: string;
  current: string;
  installing: boolean;
  failed: boolean;
  onInstall: () => void;
  onLater: () => void;
}) {
  const panelRef = useRef<HTMLDivElement>(null);
  // Escape defers the update, matching "Later" - but not mid-install, when
  // there is no safe dismissal.
  useFocusTrap(panelRef, installing ? undefined : onLater);
  return (
    <div
      ref={panelRef}
      role="dialog"
      aria-modal="true"
      aria-labelledby="update-panel-title"
      className="gc-panel-in absolute inset-0 z-20 flex flex-col items-center justify-center gap-5 bg-gc-surface px-7 text-center"
    >
      <div className="flex h-14 w-14 items-center justify-center rounded-gc-lg bg-gc-accent-wash text-gc-accent">
        <Icon name="refresh" size={26} />
      </div>

      <div className="flex flex-col gap-1.5">
        <h1
          id="update-panel-title"
          className="text-[17px] font-semibold tracking-[-0.01em] text-gc-ink"
        >
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
          v{version}
        </span>
      </div>

      <div className="mt-1 flex w-full flex-col gap-2">
        <Button variant="accent" full disabled={installing} onClick={onInstall}>
          {installing ? "Installing…" : failed ? "Retry update" : "Install & relaunch"}
        </Button>
        {!installing && (
          <button
            type="button"
            onClick={onLater}
            className="text-[12.5px] font-medium text-gc-ink-3 transition hover:text-gc-ink"
          >
            Later
          </button>
        )}
      </div>
    </div>
  );
}
