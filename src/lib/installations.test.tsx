import { beforeEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, renderHook, screen, waitFor } from "@testing-library/react";

import { InstallationPicker } from "../components/gc/InstallationPicker";
import { useInstallations, type Installation } from "./activity";
import { activityInstallations as activityInstallationsRaw } from "./api";

vi.mock("./api", () => ({
  activityInstallations: vi.fn(),
  activityOverview: vi.fn(),
}));

const activityInstallations = vi.mocked(activityInstallationsRaw);

/**
 * The installation dimension on the client (AG-572 AC 1).
 *
 * Two things are worth holding still here. The picker's default is org-wide,
 * because traffic sent before attribution existed - and anything from curl or
 * CI - carries no installation, and scoping by default would drop it out of a
 * total the user could already see. And the label always follows the *echoed*
 * scope, never the requested one, so a reading can never be captioned with a
 * filter it does not actually cover.
 */
const INSTALL = "3f2b9c4e-7a1d-4f88-9d1e-0c5a6b7e8f90";
const OTHER = "9a8b7c6d-5e4f-4321-8765-0fedcba98765";

/** The one `<select>` on screen, and the text of its options. */
const select = () => screen.getByRole("combobox") as HTMLSelectElement;
const options = () =>
  Array.from(select().options).map((o) => o.textContent);

function installation(overrides: Partial<Installation>): Installation {
  return {
    installId: INSTALL,
    label: INSTALL,
    current: false,
    lastSeenAt: "2026-08-17T12:00:00.000Z",
    requests: 42,
    ...overrides,
  };
}

describe("InstallationPicker", () => {
  beforeEach(() => {
    cleanup();
    vi.clearAllMocks();
  });

  it("stays hidden until there is a choice to make", () => {
    // One installation, or none, means the control could only ever say "All
    // installations": furniture that implies a filter is doing something.
    const { container, rerender } = render(
      <InstallationPicker installations={[]} value={null} onChange={vi.fn()} />,
    );
    expect(container.innerHTML).toBe("");

    rerender(
      <InstallationPicker
        installations={[installation({ current: true })]}
        value={null}
        onChange={vi.fn()}
      />,
    );
    expect(container.innerHTML).toBe("");
  });

  it("defaults to every installation, not to this machine", () => {
    render(
      <InstallationPicker
        installations={[
          installation({ current: true }),
          installation({ installId: OTHER, label: OTHER }),
        ]}
        value={null}
        onChange={vi.fn()}
      />,
    );
    expect(select().value).toBe("");
    expect(options()).toContain("All installations");
  });

  it("says which entry is this machine, because a raw id does not", () => {
    render(
      <InstallationPicker
        installations={[
          installation({ current: true }),
          installation({ installId: OTHER, label: OTHER }),
        ]}
        value={null}
        onChange={vi.fn()}
      />,
    );
    expect(options()).toContain(`${INSTALL} (this machine)`);
    expect(options()).toContain(OTHER);
  });

  it("reports a selection as an id, and clearing it as null", () => {
    const onChange = vi.fn();
    render(
      <InstallationPicker
        installations={[
          installation({ current: true }),
          installation({ installId: OTHER, label: OTHER }),
        ]}
        value={null}
        onChange={onChange}
      />,
    );
    fireEvent.change(select(), { target: { value: OTHER } });
    expect(onChange).toHaveBeenCalledWith(OTHER);

    // Back to org-wide has to be `null` rather than the empty string: the empty
    // string is a value the gateway would reject as a malformed installId.
    onChange.mockClear();
    cleanup();
    render(
      <InstallationPicker
        installations={[
          installation({ current: true }),
          installation({ installId: OTHER, label: OTHER }),
        ]}
        value={OTHER}
        onChange={onChange}
      />,
    );
    fireEvent.change(select(), { target: { value: "" } });
    expect(onChange).toHaveBeenCalledWith(null);
  });

  it("shows the scope it was given, so it can lag a reading rather than lead it", () => {
    // The caller passes the *echoed* scope. While a refetch is in flight the
    // numbers on screen still belong to the previous scope, and a picker that
    // jumped ahead would caption them with a filter they do not cover.
    render(
      <InstallationPicker
        installations={[
          installation({ current: true }),
          installation({ installId: OTHER, label: OTHER }),
        ]}
        value={INSTALL}
        onChange={vi.fn()}
      />,
    );
    expect(select().value).toBe(INSTALL);
  });
});

/**
 * The hook behind the picker.
 *
 * Two properties matter and neither is about rendering. The list is one org's
 * machines, so it may not outlive the credential that produced it; and nothing
 * may be fetched before there is a credential to fetch with, or the pane opens
 * on a failure banner that is about to be wrong.
 */
describe("useInstallations", () => {
  beforeEach(() => {
    cleanup();
    vi.clearAllMocks();
  });

  it("does not read until it is enabled", () => {
    renderHook(() => useInstallations(false, "oauth|https://gw|org-1"));

    expect(activityInstallations).not.toHaveBeenCalled();
  });

  it("re-reads and drops the old list when the credential changes", async () => {
    activityInstallations.mockResolvedValue(
      JSON.stringify({ installations: [installation({}), installation({ installId: OTHER })] }),
    );
    const { result, rerender } = renderHook(
      ({ credential }: { credential: string }) => useInstallations(true, credential),
      { initialProps: { credential: "oauth|https://gw|org-1" } },
    );
    await waitFor(() => expect(result.current.installations).toHaveLength(2));

    // The second org's read is still in flight here. The first org's machines
    // are already gone, because offering them would be offering a scope the new
    // reading cannot honour.
    activityInstallations.mockReturnValue(new Promise(() => {}));
    rerender({ credential: "oauth|https://gw|org-2" });

    expect(result.current.installations).toEqual([]);
    expect(activityInstallations).toHaveBeenCalledTimes(2);
  });
});
