import { useCallback, useEffect, useState } from "react";

/**
 * Text scaling for the popover.
 *
 * WCAG 2.1 SC 1.4.4 (Resize Text, AA) requires text to reach 200% without loss
 * of content or function, and PRODUCT.md commits to AA. The app had no mechanism
 * at all: every size was an absolute `text-[Npx]` literal (138 of them, zero
 * rem), the window is `resizable: false`, and nothing called a zoom API. Raising
 * the OS or browser text size changed nothing on screen, which is measurable:
 * root 16px -> 32px left a 13.5px heading at 13.5px.
 *
 * Why not webview zoom, which would have been one `set_zoom` call: the app root
 * is `h-full w-full`, so the CSS layout width *is* the window width. Zooming a
 * 380px window to 200% lays the page out at 190 CSS px, and "Waiting on routing"
 * is a ~130px pill. Zoom shrinks the room while it grows the type. Scaling the
 * rem root instead grows type inside a 360px-wide composition that stays the
 * shape it was designed as, and lets the window grow in the one axis a menubar
 * popover can afford to grow in.
 */

/** The steps. Five, not a continuous slider: this is a menubar popover, and a
 *  slider implies a precision the vertical budget cannot honour. 200% is the top
 *  because that is what 1.4.4 asks for. */
export const TEXT_SCALES = [1, 1.25, 1.5, 1.75, 2] as const;

export type TextScale = (typeof TEXT_SCALES)[number];

/** Root size the rem ramp is authored against. `tailwind.config.ts` divides by
 *  this, so 0.84375rem reads back as 13.5px at scale 1. */
const BASE_ROOT_PX = 16;

/** The window's configured logical size (`tauri.conf.json`). Height grows with
 *  the scale; width never does, because the 360px room is the design. */
const BASE_WINDOW = { width: 380, height: 620 };

const KEY = "gc.text-scale.v1";

function clampToStep(raw: number): TextScale {
  let best: TextScale = 1;
  for (const s of TEXT_SCALES) {
    if (Math.abs(s - raw) < Math.abs(best - raw)) best = s;
  }
  return best;
}

export function readStoredScale(): TextScale {
  try {
    const raw = localStorage.getItem(KEY);
    if (!raw) return 1;
    const n = Number.parseFloat(raw);
    return Number.isFinite(n) ? clampToStep(n) : 1;
  } catch {
    // Storage failures degrade to 100%, never to an unreadable screen.
    return 1;
  }
}

function store(scale: TextScale): void {
  try {
    localStorage.setItem(KEY, String(scale));
  } catch {
    /* best effort; the scale still applies for this session */
  }
}

/** Apply the scale to the document. Exported so `main.tsx` can set it before
 *  first paint and avoid a visible reflow from 100% to the stored value. */
export function applyTextScale(scale: TextScale): void {
  document.documentElement.style.fontSize = `${BASE_ROOT_PX * scale}px`;
}

/**
 * Grow the window's height with the scale, capped to what the display can
 * actually show, then let the body scroll for the remainder (`gc-scroll-more`
 * already draws the fold).
 *
 * Isolated and failure-tolerant on purpose. This is the one part of text scaling
 * that depends on the Tauri window layer rather than on CSS, it cannot be
 * exercised from a browser harness, and `resizable: false` may refuse a
 * programmatic resize on some platforms. If it fails, the type has still scaled
 * and the content still scrolls, so the accessibility requirement is met either
 * way and the failure costs height, not function.
 */
async function growWindow(scale: TextScale): Promise<void> {
  try {
    const [{ getCurrentWindow, currentMonitor, LogicalSize }] = await Promise.all([
      import("@tauri-apps/api/window"),
    ]);
    const monitor = await currentMonitor().catch(() => null);
    // Leave the menu bar and a margin; a popover taller than its screen is
    // worse than one that scrolls.
    const ceiling = monitor
      ? Math.floor((monitor.size.height / (monitor.scaleFactor || 1)) * 0.85)
      : BASE_WINDOW.height;
    const height = Math.min(Math.round(BASE_WINDOW.height * scale), ceiling);
    await getCurrentWindow().setSize(new LogicalSize(BASE_WINDOW.width, height));
  } catch {
    /* Not in Tauri, or the platform refused. CSS has already done the work. */
  }
}

/**
 * The scale, its setter, and the keyboard accelerators.
 *
 * Cmd/Ctrl `+` / `-` / `0` because that is the gesture every user already has
 * for this, and it is the only mechanism: the popover carries no visible
 * control, so the shortcut has to work on every surface.
 */
export function useTextScale(): {
  scale: TextScale;
  setScale: (next: TextScale) => void;
  increase: () => void;
  decrease: () => void;
  reset: () => void;
} {
  const [scale, setScaleState] = useState<TextScale>(readStoredScale);

  const setScale = useCallback((next: TextScale) => {
    setScaleState(next);
    store(next);
    applyTextScale(next);
    void growWindow(next);
  }, []);

  const step = useCallback(
    (direction: 1 | -1) => {
      const i = TEXT_SCALES.indexOf(scale);
      const next = TEXT_SCALES[Math.min(TEXT_SCALES.length - 1, Math.max(0, i + direction))];
      if (next !== scale) setScale(next);
    },
    [scale, setScale],
  );

  const increase = useCallback(() => step(1), [step]);
  const decrease = useCallback(() => step(-1), [step]);
  const reset = useCallback(() => setScale(1), [setScale]);

  // Apply on mount so a stored scale survives a relaunch even if `main.tsx`
  // did not pre-apply it, and re-assert the window height for the same reason.
  useEffect(() => {
    applyTextScale(scale);
    if (scale !== 1) void growWindow(scale);
    // Mount only: later changes go through `setScale`, which applies them.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  useEffect(() => {
    const onKeyDown = (e: KeyboardEvent) => {
      if (!e.metaKey && !e.ctrlKey) return;
      // `=` is the unshifted key that carries `+` on most layouts, and both
      // arrive here; Numpad has its own codes.
      if (e.key === "+" || e.key === "=" || e.code === "NumpadAdd") {
        e.preventDefault();
        increase();
      } else if (e.key === "-" || e.key === "_" || e.code === "NumpadSubtract") {
        e.preventDefault();
        decrease();
      } else if (e.key === "0" || e.code === "Numpad0") {
        e.preventDefault();
        reset();
      }
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [increase, decrease, reset]);

  return { scale, setScale, increase, decrease, reset };
}
