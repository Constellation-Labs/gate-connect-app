import type { ReactNode } from "react";
import { RoutingBanner, UpdateBanner } from "./banners";
import { Sidebar } from "./Sidebar";
import type { InventoryState, SidebarGroup, SidebarView } from "./Sidebar";
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
  onSelectApp,
  onToggleApp,
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
  /** Omitted when the account cannot switch organizations. Passed straight
   *  through to the sidebar, which then draws the org line as a label. */
  onSwitchOrg?: () => void;
  view: SidebarView;
  onNavigate: (view: SidebarView) => void;
  /** The rail's app rows, grouped under their family eyebrows. */
  appGroups: SidebarGroup[];
  onSelectApp: (slug: string) => void;
  onToggleApp: (slug: string, next: boolean) => void;
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
    // `tabular-nums` on the root, not per figure. "Always use tabular nums on
    // numbers" is design's standing rule (2026-09-04): the point is that a
    // column of counts, percentages and currency lines up, and Geist's
    // proportional digits do not. Set once here so no figure added later can
    // miss it - the same argument the `label/copy` tracking tokens make.
    <div className="relative flex h-full w-full flex-col overflow-hidden bg-base-card tabular-nums">
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

      {/* Under the modal scrim, like every other piece of chrome.
        *
        * The error banner used to be lifted above it (#244 narrowed the lift to
        * this one notice, on the grounds that a failed action is reported here
        * and nowhere else, so its dismiss button would otherwise be readable
        * and unclickable).
        *
        * Driving the app against staging showed what that costs: a connect
        * failure and the certificate dialog arrive from the same click, and the
        * banner then floats lit over a dimmed dialog it has nothing to do with.
        * It reads as a layering bug, and it makes a modal not modal.
        *
        * Readable is the part that matters - the banner is a report, not a
        * control - and the dismiss comes back the moment the dialog closes. An
        * error raised BY a dialog's own action belongs inside that dialog,
        * which is the better fix and a larger one. */}
      {notice && <div>{notice}</div>}

      <div className="flex min-h-0 flex-1">
        <Sidebar
          orgName={orgName}
          onSwitchOrg={onSwitchOrg}
          view={view}
          onNavigate={onNavigate}
          groups={appGroups}
          onSelectApp={onSelectApp}
          onToggleApp={onToggleApp}
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
