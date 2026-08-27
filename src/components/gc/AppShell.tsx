import type { ReactNode } from "react";
import { RoutingBanner, UpdateBanner } from "./banners";
import { Sidebar } from "./Sidebar";
import type {
  InventoryState,
  MasterRouting,
  SidebarGroup,
  SidebarView,
} from "./Sidebar";
import { Topbar } from "./Topbar";
import type { TopnavAction } from "./Topbar";

/**
 * The 1024x720 window: chrome, the banner stack, the navigation rail, and
 * whichever pane is open.
 *
 * The pane arrives as `children` rather than being selected here. Routing is a
 * one-line switch on `view.kind` at the call site, and pushing it in would mean
 * this component had to accept every pane's data as props.
 *
 * Dialogs are a slot for the same reason: the shell owns *that* a dialog is
 * covering the window, the caller owns which one.
 */
export function AppShell({
  menuOpen,
  onMenuToggle,
  onMenuSelect,
  update,
  routing,
  orgName,
  onSwitchOrg,
  view,
  onNavigate,
  appGroups,
  master,
  onSelectApp,
  onToggleApp,
  onToggleGroup,
  onRefreshApps,
  refreshingApps,
  inventory,
  notice,
  dialog,
  children,
}: {
  menuOpen: boolean;
  onMenuToggle: () => void;
  onMenuSelect: (action: TopnavAction) => void;
  /** Omit when no update is pending; the banner is not rendered at all. */
  update?: { version: string; onUpdate: () => void; onDismiss: () => void };
  routing: { protectedCount: number; totalCount: number };
  orgName: string;
  onSwitchOrg: () => void;
  view: SidebarView;
  onNavigate: (view: SidebarView) => void;
  /** The rail's app rows, grouped under their family eyebrows. */
  appGroups: SidebarGroup[];
  /** The engine's switch, drawn above the app groups. Passed straight through
   * to the sidebar, which owns routing's controls. */
  master?: MasterRouting;
  onSelectApp: (slug: string) => void;
  onToggleApp: (slug: string, next: boolean) => void;
  onToggleGroup?: (id: string, next: boolean) => void;
  /** Re-run tool detection. Passed straight through to the sidebar, which owns
   * the control. */
  onRefreshApps?: () => void;
  refreshingApps?: boolean;
  /** What the last detection scan established. Passed through to the sidebar,
   * which owns the inventory. */
  inventory?: InventoryState;
  /**
   * A failure worth interrupting for, shown under the banners. Sits here rather
   * than inside a pane so it survives navigation and reads the same whichever
   * pane is open - a failed toggle is about the window, not about one view.
   */
  notice?: ReactNode;
  /** A dialog covering the window, or nothing. */
  dialog?: ReactNode;
  /** The open pane. */
  children: ReactNode;
}) {
  return (
    // `relative` anchors the dialog scrim, which covers the window including
    // its chrome - the design dims the topbar and banners along with the pane.
    <div className="relative flex h-full w-full flex-col overflow-hidden bg-base-card">
      <Topbar
        menuOpen={menuOpen}
        onMenuToggle={onMenuToggle}
        onMenuSelect={onMenuSelect}
      />

      {update && (
        <UpdateBanner
          version={update.version}
          onUpdate={update.onUpdate}
          onDismiss={update.onDismiss}
        />
      )}
      <RoutingBanner
        protectedCount={routing.protectedCount}
        totalCount={routing.totalCount}
      />

      {notice}

      <div className="flex min-h-0 flex-1">
        <Sidebar
          orgName={orgName}
          onSwitchOrg={onSwitchOrg}
          view={view}
          onNavigate={onNavigate}
          groups={appGroups}
          master={master}
          onSelectApp={onSelectApp}
          onToggleApp={onToggleApp}
          onToggleGroup={onToggleGroup}
          onRefresh={onRefreshApps}
          refreshing={refreshingApps}
          inventory={inventory}
        />
        {children}
      </div>

      {dialog}
    </div>
  );
}
