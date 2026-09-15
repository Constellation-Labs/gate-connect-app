import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { ModelPickerDialog } from "./dialogs";

/**
 * `ModelPickerDialog`'s two selection modes (design, 2026-09-04).
 *
 * The 25 cases in `e2e/new-ui-model-picker.spec.ts` already walk the multiple
 * mode through the real shell, so what is worth pinning here is what a browser
 * run cannot reach or would pay a lot to reach: the `Apply selections` gate,
 * which is a comparison rather than a click path; the single mode, which has no
 * call site yet and so appears in no e2e flow at all; and the handful of
 * behaviours that are *different* between the modes rather than merely present
 * in one - the lock on the last model, the footer note, the button row.
 *
 * Copy is asserted because it is the file's, not ours.
 */

const noop = () => {};

// `tags` carries what the gateway advertises; `modelCompatibility` reads it.
// Empty here on purpose: these cases are about the selection modes, and the
// compatibility filter has its own coverage in `modelCompatibility.test.ts`.
const CATALOGUE = [
  { id: "anthropic/claude-opus-5", vendor: "anthropic", tags: [] },
  { id: "openai/gpt-5", vendor: "openai", tags: [] },
  { id: "moonshot/kimi-k3", vendor: "moonshot", tags: [] },
];

function renderPicker(
  overrides: Partial<Parameters<typeof ModelPickerDialog>[0]> = {},
) {
  return render(
    <ModelPickerDialog
      appName="OpenCode"
      appSlug="opencode"
      models={CATALOGUE}
      selectedIds={[CATALOGUE[0].id]}
      onSave={noop}
      onDismiss={noop}
      {...overrides}
    />,
  );
}

/** The row for one model, and the primary, named by the mode drawing them. */
const apply = () => screen.getByRole("button", { name: "Apply selections" });
const box = (id: string) => screen.getByRole("checkbox", { name: id });
const radio = (id: string) => screen.getByRole("radio", { name: id });

afterEach(cleanup);

describe("the model picker, choosing several", () => {
  it("draws the file's copy, and names the app in the subtitle", () => {
    renderPicker();
    expect(
      screen.getByRole("heading", { name: "Choose Gate models" }),
    ).toBeTruthy();
    expect(screen.getByRole("dialog").textContent).toContain(
      "OpenCode will be able to use these models",
    );
    expect(screen.getByRole("button", { name: "Cancel" })).toBeTruthy();
    expect(apply()).toBeTruthy();
  });

  it("draws a checkbox per model, marking the ones already applied", () => {
    renderPicker();
    expect(screen.getAllByRole("checkbox")).toHaveLength(3);
    expect(box(CATALOGUE[0].id).getAttribute("aria-checked")).toBe("true");
    expect(box(CATALOGUE[1].id).getAttribute("aria-checked")).toBe("false");
  });

  it("refuses Apply until something actually changes", () => {
    const onSave = vi.fn();
    renderPicker({ onSave });

    // Opened on an already-applied set: there is nothing to apply, which is the
    // state design asked for explicitly ("if they open the modal after
    // selections are applied, then the apply button is disabled"). Asserted on
    // `aria-disabled` and on the click being inert, because `Modal` refuses by
    // dropping the handler rather than by setting `disabled`.
    expect(apply().getAttribute("aria-disabled")).toBe("true");
    fireEvent.click(apply());
    expect(onSave).not.toHaveBeenCalled();

    fireEvent.click(box(CATALOGUE[1].id));
    expect(apply().getAttribute("aria-disabled")).toBeNull();
  });

  it("compares the draft as a set, so reordering the same models is not a change", () => {
    // Opens on two, then clears one and re-adds it. The draft now holds the
    // same two models in the *other* order, which is the only state that tells
    // a set comparison apart from a sequence one: `join()` would call this a
    // change and enable Apply over a write that would change nothing. Order in
    // the stored list is the user's, not the selection's.
    renderPicker({ selectedIds: [CATALOGUE[0].id, CATALOGUE[1].id] });

    fireEvent.click(box(CATALOGUE[0].id));
    expect(apply().getAttribute("aria-disabled")).toBeNull();

    fireEvent.click(box(CATALOGUE[0].id));
    expect(apply().getAttribute("aria-disabled")).toBe("true");
  });

  it("applies the whole draft at once, not one click at a time", () => {
    const onSave = vi.fn();
    renderPicker({ onSave });

    fireEvent.click(box(CATALOGUE[1].id));
    fireEvent.click(box(CATALOGUE[2].id));
    expect(onSave).not.toHaveBeenCalled();

    fireEvent.click(apply());
    expect(onSave).toHaveBeenCalledTimes(1);
    expect([...onSave.mock.calls[0][0]].sort()).toEqual(
      [CATALOGUE[0].id, CATALOGUE[1].id, CATALOGUE[2].id].sort(),
    );
  });

  it("states the cost consequence, and says what is needed when the draft empties", () => {
    renderPicker();
    expect(screen.getByRole("dialog").textContent).toContain(
      "consume Gate credits",
    );

    // The empty draft is a dead end without a sentence beside the refused
    // button, so the note swaps rather than going quiet.
    fireEvent.click(box(CATALOGUE[0].id));
    const dialog = screen.getByRole("dialog").textContent ?? "";
    expect(dialog).toContain("No models enabled");
    expect(dialog).toContain("needs at least one model");
  });

  it("holds AG-590 on the primary, not on the row", () => {
    // The row clears freely; what may not be *written* is an empty set. Pinned
    // because the rule used to live on the row and moving it is easy to undo by
    // accident, which would make the last model unclearable again.
    renderPicker();
    const only = box(CATALOGUE[0].id);
    expect(only.getAttribute("aria-disabled")).toBeNull();

    fireEvent.click(only);
    expect(box(CATALOGUE[0].id).getAttribute("aria-checked")).toBe("false");
    expect(apply().getAttribute("aria-disabled")).toBe("true");

    // And a different model makes it saveable again.
    fireEvent.click(box(CATALOGUE[1].id));
    expect(apply().getAttribute("aria-disabled")).toBeNull();
  });
});

describe("the model picker, choosing one", () => {
  it("draws the singular title and no button row at all", () => {
    renderPicker({ multiple: false });
    expect(
      screen.getByRole("heading", { name: "Choose a Gate model" }),
    ).toBeTruthy();
    // The click is the confirmation, so there is nothing for a footer to do.
    expect(screen.queryByRole("button", { name: "Cancel" })).toBeNull();
    expect(
      screen.queryByRole("button", { name: "Apply selections" }),
    ).toBeNull();
  });

  it("draws radios rather than checkboxes", () => {
    renderPicker({ multiple: false });
    expect(screen.queryAllByRole("checkbox")).toHaveLength(0);
    expect(screen.getAllByRole("radio")).toHaveLength(3);
    expect(radio(CATALOGUE[0].id).getAttribute("aria-checked")).toBe("true");
  });

  it("applies on the click, replacing the selection rather than adding to it", () => {
    const onSave = vi.fn();
    renderPicker({ multiple: false, onSave });

    fireEvent.click(radio(CATALOGUE[1].id));
    expect(onSave).toHaveBeenCalledTimes(1);
    expect(onSave.mock.calls[0][0]).toEqual([CATALOGUE[1].id]);
  });

  it("cannot reach an empty set, so re-picking the current model just applies it", () => {
    // The multiple mode can empty its draft and is refused on the primary.
    // Single-select has no such state to reach: every click writes exactly one
    // model, including a click on the one already highlighted.
    const onSave = vi.fn();
    renderPicker({ multiple: false, onSave, selectedIds: [CATALOGUE[0].id] });

    fireEvent.click(radio(CATALOGUE[0].id));
    expect(onSave).toHaveBeenCalledWith([CATALOGUE[0].id]);
  });

  it("drops the footer note, which is about a set being assembled", () => {
    renderPicker({ multiple: false });
    const dialog = screen.getByRole("dialog").textContent ?? "";
    expect(dialog).not.toContain("consume Gate credits");
    expect(dialog).not.toContain("No models enabled");
  });

  it("reports an unavailable model without offering a control for it", () => {
    // AG-592's row. In the multiple mode it is a checkbox the user clears; here
    // picking any model below replaces the whole set and takes it with them, so
    // a control would do the same job twice.
    renderPicker({ multiple: false, selectedIds: ["retired/model-v1"] });

    const dialog = screen.getByRole("dialog");
    expect(dialog.textContent).toContain("retired/model-v1");
    expect(dialog.textContent).toContain("Unavailable");

    // Every role the row could arrive as, not just the one this mode's live
    // rows use: the multiple mode's version of this row is a hardcoded
    // `checkbox`, so asking only about `radio` would pass on the very
    // regression this pins - the multiple branch rendering in single mode.
    const named = { name: /retired\/model-v1/ };
    expect(screen.queryAllByRole("checkbox", named)).toHaveLength(0);
    expect(screen.queryAllByRole("radio", named)).toHaveLength(0);
    expect(screen.queryAllByRole("button", named)).toHaveLength(0);
  });
});
