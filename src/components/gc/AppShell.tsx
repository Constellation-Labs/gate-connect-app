import type { ReactNode } from "react";
import { RoutingBanner, UpdateBanner } from "./banners";
import { Sidebar } from "./Sidebar";
import type { SidebarApp, SidebarView } from "./Sidebar";
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
  apps,
  onSelectApp,
  onToggleApp,
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
  apps: SidebarApp[];
  onSelectApp: (slug: string) => void;
  onToggleApp: (slug: string, next: boolean) => void;
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

      <div className="flex min-h-0 flex-1">
        <Sidebar
          orgName={orgName}
          onSwitchOrg={onSwitchOrg}
          view={view}
          onNavigate={onNavigate}
          apps={apps}
          onSelectApp={onSelectApp}
          onToggleApp={onToggleApp}
        />
        {children}
      </div>

      {dialog}
    </div>
  );
}
