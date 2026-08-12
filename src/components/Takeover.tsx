import { useRef } from "react";
import type { ReactNode, RefObject } from "react";
import { useFocusTrap } from "../lib/useFocusTrap";

/** Stacking order for the full-popover takeovers. The popover never stacks
 *  dialogs, but more than one can be *pending* at once, and the priority is
 *  fixed: a quit decision the user just asked for outranks a routing notice,
 *  which outranks an update prompt, which outranks an offer nobody requested.
 *
 *  `trust` is the one that must never be buried: an operation is suspended on
 *  its answer (App's `ensureCaTrusted` awaits the click), so a takeover hiding
 *  it would leave the user with a busy popover and nothing to press. It can
 *  only open from an interaction with the room, which every other takeover
 *  covers, so in practice it never shares the screen with one. */
export const TAKEOVER_Z = {
  offer: "z-10",
  routing: "z-10",
  update: "z-20",
  trust: "z-20",
  quit: "z-30",
} as const;

/** The full-popover takeover shell: the panel slides over the room, traps
 *  focus, and closes on Escape.
 *
 *  Extracted because all four takeovers carried a byte-identical 90-character
 *  layout string and their own copy of the ref + trap wiring, which is four
 *  places for the next one to drift from. Only the stacking level, the label
 *  and the trap's arguments differ, so those are the props. */
export function Takeover({
  z,
  labelledBy,
  onEscape,
  /** Where focus lands on mount: the safe choice, never the destructive one. */
  initialFocus,
  /** Changes when the panel swaps its contents for a new step, so the trap
   * re-places focus instead of dropping it on `document.body`. */
  resetKey,
  children,
}: {
  z: (typeof TAKEOVER_Z)[keyof typeof TAKEOVER_Z];
  labelledBy: string;
  onEscape?: () => void;
  initialFocus?: RefObject<HTMLElement>;
  resetKey?: unknown;
  children: ReactNode;
}) {
  const panelRef = useRef<HTMLDivElement>(null);
  useFocusTrap(panelRef, onEscape, initialFocus, resetKey);
  return (
    <div
      ref={panelRef}
      role="dialog"
      aria-modal="true"
      aria-labelledby={labelledBy}
      className={`gc-panel-in absolute inset-0 ${z} flex flex-col items-center justify-center gap-5 bg-gc-surface px-7 text-center`}
    >
      {children}
    </div>
  );
}
