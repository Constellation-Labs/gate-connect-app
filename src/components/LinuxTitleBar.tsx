import { getCurrentWindow } from "@tauri-apps/api/window";

/**
 * Custom window chrome for Linux only. The window is borderless
 * (`decorations: false`), and on Wayland/GNOME the native CSD title-bar
 * buttons don't reliably bind - so we draw our own strip and call the Tauri
 * window APIs directly, which do work here. macOS and Windows never render
 * this: they're non-activating popovers managed by the tray, not draggable
 * windows.
 *
 * The strip is the drag region (begin-move via `startDragging`, which the
 * compositor honors even where programmatic `set_position` is ignored). The
 * buttons sit outside the drag region so a click lands on the control rather
 * than starting a drag. Close hides the window (re-summon from the tray);
 * minimize drops it to the taskbar.
 */
export function LinuxTitleBar() {
  const win = getCurrentWindow();

  return (
    <div className="sticky top-0 z-50 flex h-8 shrink-0 items-stretch border-b border-gc-line bg-gc-surface">
      {/* Drag region: primary-button press starts an interactive move. */}
      <div
        className="flex-1"
        onMouseDown={(e) => {
          if (e.button === 0) void win.startDragging();
        }}
      />
      <button
        type="button"
        aria-label="Minimize"
        className="flex w-11 items-center justify-center text-gc-ink-4 transition-colors hover:bg-gc-sunken hover:text-gc-ink-2"
        onClick={() => void win.minimize()}
      >
        <svg width="10" height="10" viewBox="0 0 10 10" aria-hidden="true">
          <line x1="1" y1="5" x2="9" y2="5" stroke="currentColor" strokeWidth="1.2" />
        </svg>
      </button>
      <button
        type="button"
        aria-label="Close"
        className="flex w-11 items-center justify-center text-gc-ink-4 transition-colors hover:bg-gc-error hover:text-white"
        onClick={() => void win.hide()}
      >
        <svg width="10" height="10" viewBox="0 0 10 10" aria-hidden="true">
          <line x1="1" y1="1" x2="9" y2="9" stroke="currentColor" strokeWidth="1.2" />
          <line x1="9" y1="1" x2="1" y2="9" stroke="currentColor" strokeWidth="1.2" />
        </svg>
      </button>
    </div>
  );
}
