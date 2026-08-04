import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { useRef, useState } from "react";
import { useFocusTrap } from "./useFocusTrap";

/** A panel that swaps its buttons on "step", the shape every takeover has. */
function Panel({ onEscape }: { onEscape?: () => void }) {
  const panelRef = useRef<HTMLDivElement>(null);
  const safeRef = useRef<HTMLButtonElement>(null);
  const [step, setStep] = useState<"info" | "confirm" | "busy">("info");
  useFocusTrap(panelRef, onEscape, safeRef, step);
  return (
    <div ref={panelRef} role="dialog">
      {step === "info" && (
        <>
          <button ref={safeRef} onClick={() => setStep("confirm")}>
            Advance
          </button>
          <button>Other</button>
        </>
      )}
      {step === "confirm" && (
        <>
          <button onClick={() => setStep("busy")}>Destroy</button>
          <button ref={safeRef}>Cancel</button>
        </>
      )}
      {/* Every control disabled: the state that left focus on body, because
          `.focus()` on a disabled element silently does nothing. */}
      {step === "busy" && (
        <>
          <button disabled>Destroy</button>
          <button ref={safeRef} disabled>
            Cancel
          </button>
        </>
      )}
    </div>
  );
}

afterEach(cleanup);

describe("useFocusTrap across step changes", () => {
  it("re-places focus on the safe control when the step changes", () => {
    render(<Panel />);
    expect(document.activeElement).toBe(screen.getByText("Advance"));
    fireEvent.click(screen.getByText("Advance"));
    // Previously focus fell to <body> here, and the next Tab reached the
    // destructive button.
    expect(document.activeElement).toBe(screen.getByText("Cancel"));
  });

  it("falls back to the panel when every control is disabled", () => {
    render(<Panel />);
    fireEvent.click(screen.getByText("Advance"));
    fireEvent.click(screen.getByText("Destroy"));
    expect(document.activeElement).toBe(screen.getByRole("dialog"));
    expect(document.activeElement).not.toBe(document.body);
  });

  it("handles Escape after focus has left the panel", () => {
    const onEscape = vi.fn();
    render(<Panel onEscape={onEscape} />);
    // The listener is on document, so this works even from outside the panel.
    (document.activeElement as HTMLElement).blur();
    expect(document.activeElement).toBe(document.body);
    fireEvent.keyDown(document, { key: "Escape" });
    expect(onEscape).toHaveBeenCalledTimes(1);
  });

  it("keeps Tab inside the panel when focus has escaped it", () => {
    render(<Panel />);
    (document.activeElement as HTMLElement).blur();
    fireEvent.keyDown(document, { key: "Tab" });
    expect(screen.getByRole("dialog").contains(document.activeElement)).toBe(true);
  });
});
