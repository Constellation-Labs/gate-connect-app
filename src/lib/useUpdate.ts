import { useCallback, useRef, useState } from "react";
import { check, type Update } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";
import { getVersion } from "@tauri-apps/api/app";
import { setUpdaterRelaunching } from "./api";
import { track, trackError } from "./analytics";

/**
 * Finding and installing an app update.
 *
 * Extracted from `components/UpdatePanel.tsx` rather than reimplemented for the
 * new shell, which is the opposite call from `useRouting` and `useSettingsActions`.
 * Those two were tangled with the popover's shape; this is not. What it holds is
 * a *sequence* whose ordering is load-bearing in a way no reader would guess
 * (see `install`), and a second copy of that could drift into a botched update
 * or a machine left routing through a stopped engine.
 *
 * The two surfaces differ only in presentation: the popover takes over on the
 * first check and drops to a slim banner after a reopen, the window always uses
 * the banner. That decision stays with each of them.
 */

export type CheckOutcome = "idle" | "up-to-date" | "found" | "failed";

export interface UpdateState {
  /** The pending update, or null. Only the version is needed to render. */
  available: { version: string } | null;
  /** The running app's version, for surfaces that show both. */
  current: string;
  checking: boolean;
  installing: boolean;
  /** A download or install that failed, so the surface can offer a retry. */
  failed: boolean;
  /**
   * The result of the most recent *explicit* check. A background check leaves
   * this alone: "up to date" is only worth saying to someone who just asked.
   */
  outcome: CheckOutcome;
  checkNow: (explicit?: boolean) => Promise<void>;
  install: () => Promise<void>;
  loadCurrentVersion: () => Promise<void>;
}

export function useUpdate(): UpdateState {
  // The `Update` handle carries the download/install methods, so it is kept as
  // a ref rather than state: it is not renderable and replacing it should not
  // repaint anything on its own.
  const handle = useRef<Update | null>(null);
  const [available, setAvailable] = useState<{ version: string } | null>(null);
  const [current, setCurrent] = useState("");
  const [checking, setChecking] = useState(false);
  const [installing, setInstalling] = useState(false);
  const [failed, setFailed] = useState(false);
  const [outcome, setOutcome] = useState<CheckOutcome>("idle");

  const loadCurrentVersion = useCallback(async () => {
    try {
      setCurrent(await getVersion());
    } catch {
      // Cosmetic. A surface that cannot name the current version still works.
    }
  }, []);

  /**
   * Ask the update endpoint. Silent by default: offline, or an unreachable
   * endpoint, is not something to interrupt anyone about.
   *
   * `explicit` is the Settings row, where silence would read as a broken button,
   * so the outcome is recorded for it to report.
   */
  const checkNow = useCallback(async (explicit = false) => {
    setChecking(true);
    if (explicit) setOutcome("idle");
    try {
      const found = await check();
      handle.current = found ?? null;
      setAvailable(found ? { version: found.version } : null);
      if (explicit) setOutcome(found ? "found" : "up-to-date");
    } catch (err) {
      if (explicit) {
        setOutcome("failed");
        trackError(err, "update");
      }
      // A silent check that fails stays silent.
    } finally {
      setChecking(false);
    }
  }, []);

  /**
   * Download, then install and relaunch.
   *
   * The two phases are separate so the updater-relaunch mark brackets only the
   * install. Quitting during the (long) download is a genuine user exit, and a
   * mark set there would make the exit handler skip clearing the routing intent
   * and completing a deferred launch-at-login opt-out, with no relaunch coming
   * to redo them.
   *
   * The mark goes on *before* `install()`, not after: on Windows the installer
   * exits the app from inside that call, so marking afterwards would never run.
   */
  const install = useCallback(async () => {
    const update = handle.current;
    if (!update || installing) return;
    setFailed(false);
    setInstalling(true);

    try {
      await update.download();
    } catch (err) {
      trackError(err, "update");
      setInstalling(false);
      setFailed(true);
      return;
    }

    try {
      await setUpdaterRelaunching(true);
      await update.install();
      // Best-effort: on Windows the installer exits from inside install(), so
      // this may never send there.
      track("update_installed");
      await relaunch();
    } catch (err) {
      trackError(err, "update");
      await setUpdaterRelaunching(false).catch(() => undefined);
      setInstalling(false);
      setFailed(true);
    }
  }, [installing]);

  return {
    available,
    current,
    checking,
    installing,
    failed,
    outcome,
    checkNow,
    install,
    loadCurrentVersion,
  };
}
