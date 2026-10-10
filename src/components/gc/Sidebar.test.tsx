import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import { Sidebar } from "./Sidebar";
import type { SidebarGroup, SidebarView } from "./Sidebar";

afterEach(cleanup);

const groups: SidebarGroup[] = [
  {
    id: "band:other",
    label: "Other apps",
    apps: [{ slug: "openclaw", name: "OpenClaw", status: { kind: "protected" }, on: true }],
  },
  {
    id: "not-installed",
    label: "Not installed",
    notInstalled: true,
    apps: [{ slug: "hermes", name: "Hermes", status: { kind: "not-installed" }, on: false }],
  },
];

function rail(view: SidebarView = { kind: "overview" }, onSelectApp = vi.fn()) {
  render(
    <Sidebar
      orgName="Acme"
      view={view}
      onNavigate={() => {}}
      groups={groups}
      onSelectApp={onSelectApp}
    />,
  );
  return onSelectApp;
}

/** The eyebrow row and the list under it, for one group. */
function group(label: string) {
  const heading = screen.getByRole("heading", { name: label });
  const eyebrow = heading.parentElement as HTMLElement;
  return { eyebrow, list: eyebrow.nextElementSibling as HTMLElement };
}

describe("Sidebar: the Not installed group", () => {
  it("draws its rows with the status and no counter", () => {
    rail();
    const { eyebrow, list } = group("Not installed");
    expect(within(list).getByText("Hermes")).toBeTruthy();
    expect(within(list).getByText("Not installed")).toBeTruthy();
    // An ordinary group keeps its counter; this one has none.
    expect(within(group("Other apps").eyebrow).getByText(/1 of\s+1/)).toBeTruthy();
    expect(eyebrow.textContent).toBe("Not installed");
  });

  it("makes its rows display-only", () => {
    const onSelectApp = rail();
    const { list } = group("Not installed");
    expect(within(list).queryByRole("button")).toBeNull();
    fireEvent.click(within(list).getByText("Hermes"));
    expect(onSelectApp).not.toHaveBeenCalled();

    // The installed app is still a row that opens its pane.
    fireEvent.click(screen.getByRole("button", { name: /OpenClaw/ }));
    expect(onSelectApp).toHaveBeenCalledWith("openclaw");
  });

  it("does not mark a row selected when the open pane is an app uninstalled since", () => {
    rail({ kind: "app", slug: "hermes" });
    const row = within(group("Not installed").list).getByRole("listitem");
    expect(row.className).not.toContain("shadow-base-xs");
  });
});
