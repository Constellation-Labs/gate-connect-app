import { useEffect, useRef, useState } from "react";
import { useUpdate } from "../lib/useUpdate";
import { useWindowReopen } from "../lib/useWindowReopen";
import { Takeover, TAKEOVER_Z } from "./Takeover";
import { track } from "../lib/analytics";
import { Button, IconButton } from "./gc/ui";
import { Icon } from "./gc/Icon";

/** Checks for an app update. The first check at startup takes over the popover
 *  with a full-panel prompt. Because the webview stays mounted for the app's
 *  whole life, later checks run each time the tray reopens the popover and
 *  surface as a slim top banner instead - a mid-task reopen shouldn't be
 *  blocked by a takeover. Both offer the same one-click "install & relaunch".
 *  The check is silent - offline or an unreachable endpoint shows nothing. */
export function UpdatePanel({
  suppressTakeover = false,
  onTakeoverVisibleChange,
}: {
  /** True while another (higher z) takeover owns the popover; the startup
   * update takeover defers until it clears rather than mounting beneath it
   * and stealing focus into a hidden panel. */
  suppressTakeover?: boolean;
  /** Lets the shell mark the background aria-hidden while the startup
   * takeover is mounted. */
  onTakeoverVisibleChange?: (visible: boolean) => void;
}) {
  // The check/download/install sequence is shared with the window UI; this
  // component owns only how it is surfaced. See `lib/useUpdate`.
  const { available: update, current, installing, failed, checkNow, install, loadCurrentVersion } =
    useUpdate();
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

  useEffect(() => {
    void checkNow();
    void loadCurrentVersion();
  }, [checkNow, loadCurrentVersion]);

  // Re-check on each tray reopen, so an update released while the app sat in
  // the tray still surfaces. From then on an available update surfaces as the
  // banner, not the startup panel - a mid-task reopen shouldn't be blocked by
  // a takeover.
  useWindowReopen(() => {
    setReopened(true);
    void checkNow();
  });

  // The startup takeover is visible when an update exists, no reopen has
  // demoted it to the banner, it hasn't been dismissed, and no higher
  // takeover suppresses it. Reported on the edge so the shell can hide the
  // background from assistive tech.
  const takeoverVisible = !!update && !reopened && !panelDismissed && !suppressTakeover;
  useEffect(() => {
    onTakeoverVisibleChange?.(takeoverVisible);
  }, [takeoverVisible, onTakeoverVisibleChange]);

  if (!update) return null;

  // After a reopen, surface the quiet top banner instead of the takeover.
  if (reopened) {
    if (bannerDismissed) return null;
    return (
      <div
        role="status"
        className="flex shrink-0 items-center gap-2 border-b border-gc-line bg-gc-accent-wash-2 py-1.5 pl-3 pr-1.5"
      >
        <Icon name="refresh" size={13} className="shrink-0 text-gc-accent" />
        <div className="min-w-0 flex-1 text-gc-caption text-gc-ink-2">
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
          className="shrink-0 text-gc-caption font-semibold text-gc-ink underline decoration-gc-line-strong underline-offset-2 transition hover:decoration-gc-ink-3 disabled:no-underline disabled:opacity-45"
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

  // Startup: full-panel takeover. Deferred (not dismissed) while a higher
  // takeover is up; it mounts once that clears.
  if (panelDismissed || suppressTakeover) return null;
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
  // Escape defers the update, matching "Later" - but not mid-install, when
  // there is no safe dismissal.
  // "Install & relaunch" restarts the app under the user; Later takes focus.
  const safeRef = useRef<HTMLButtonElement>(null);
  return (
    <Takeover
      z={TAKEOVER_Z.update}
      labelledBy="update-panel-title"
      onEscape={installing ? undefined : onLater}
      initialFocus={safeRef}
      // `installing` unmounts Later and disables Install, leaving nothing
      // focusable in the panel.
      resetKey={installing}
    >
      <div className="flex h-14 w-14 items-center justify-center rounded-gc-lg bg-gc-accent-wash text-gc-accent">
        <Icon name="refresh" size={26} />
      </div>

      <div className="flex flex-col gap-1.5">
        <h1
          id="update-panel-title"
          className="text-gc-panel-title font-semibold tracking-[-0.01em] text-gc-ink"
        >
          Update ready
        </h1>
        <p className="text-gc-body-sm leading-snug text-gc-ink-3">
          {failed
            ? "The update couldn’t install. Check your connection and try again."
            : "A new version of Gate Connect is ready to install."}
        </p>
        {/* The button says "Install & relaunch" and the panel said nothing about
            what a relaunch costs. This is the one takeover a user meets
            mid-task, and the question it left unanswered is whether their
            traffic stops: it does not, because the exit is marked as an updater
            relaunch and the backend restores the routing intent afterwards.
            Phrased as the setup rather than as "routing comes back on", which
            would imply routing is currently on; this panel does not know. */}
        {!failed && (
          <p className="text-gc-caption leading-snug text-gc-ink-3">
            Installing restarts Gate Connect; your routing setup comes back with
            it.
          </p>
        )}
      </div>

      <div className="flex items-center gap-2 font-mono text-gc-caption">
        {current && <span className="text-gc-ink-3">v{current}</span>}
        {/* ink-4, the weight every other chevron in the app carries. ink-5 was
            the only use of that step in the product and it measures 2.42:1 on
            white, under the 3:1 bar for a graphic that turns two version numbers
            into a direction. */}
        {current && <Icon name="chevronRight" size={13} className="text-gc-ink-4" />}
        {/* Sunken, not highlight. `gc-highlight` is the hint-banner surface in
            DESIGN.md, spent on the one thing this app has to say when routing
            state and running apps disagree; a version number is not that, and
            borrowing the colour dilutes it. Sunken is the neutral chip surface
            the mechanism chips and the idle pill already use, and ink at medium
            still ranks this above the outgoing version beside it. */}
        <span className="rounded-gc-pill bg-gc-sunken px-2 py-0.5 font-medium text-gc-ink">
          v{version}
        </span>
      </div>

      <div className="mt-1 flex w-full flex-col gap-2">
        <Button variant="accent" full disabled={installing} onClick={onInstall}>
          {installing ? "Installing…" : failed ? "Retry update" : "Install & relaunch"}
        </Button>
        {!installing && (
          <button
            ref={safeRef}
            type="button"
            onClick={onLater}
            className="text-gc-body-sm font-medium text-gc-ink-3 transition hover:text-gc-ink"
          >
            Later
          </button>
        )}
      </div>
    </Takeover>
  );
}
