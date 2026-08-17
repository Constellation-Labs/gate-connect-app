import { BaseSwitch, Card } from "./base";
import { Icon } from "./Icon";
import type { IconName } from "./Icon";

/**
 * The Settings pane (Figma `Flows / Settings`). Named `SettingsPane` rather
 * than `Settings` so it does not read as a swap for `screens/Settings.tsx`,
 * which is the popover's version and stays until the shell swap.
 *
 * Rows are uniform enough across all six sections that the pane takes a
 * declarative model rather than a prop per field: label, an optional value, and
 * either an action button or a switch. Presentational, like the rest.
 */

export interface SettingsAction {
  label: string;
  onClick: () => void;
  /** Filled red rather than outline: Disconnect Gate, Review reset. */
  destructive?: boolean;
  /** Opens the web dashboard, so the label carries the external-link glyph. */
  external?: boolean;
}

export interface SettingsRow {
  id: string;
  icon: IconName;
  label: string;
  /** Second line under the label - the Startup rows and Danger zone. */
  description?: string;
  /** Middle column, e.g. "MacBook Pro". */
  value?: string;
  /** Set the value in Geist Mono: install IDs, API keys, versions. */
  mono?: boolean;
  action?: SettingsAction;
  toggle?: { on: boolean; onToggle: () => void };
}

export interface SettingsSection {
  id: string;
  title: string;
  /** Red heading and a red-tinted card. */
  danger?: boolean;
  rows: SettingsRow[];
}

/**
 * The design's own section list, as data. Kept here rather than in the shell so
 * the copy stays next to the screen it describes, and so "which rows exist" is
 * testable without mounting the app.
 *
 * Diagnostics is the one row the Figma does not draw. `screens/Diagnostics.tsx`
 * exists and has nowhere else to live in the new IA, so it sits under About and
 * opens the report dialog. Expect it to be redrawn.
 */
export function buildSettingsSections({
  deviceName,
  installId,
  loginId,
  plan,
  gateway,
  apiKeyMasked,
  launchAtLogin,
  notifications,
  version,
  onRenameDevice,
  onCopyInstallId,
  onUpgradePlan,
  onReplaceKey,
  onDisconnect,
  onToggleLaunchAtLogin,
  onToggleNotifications,
  onReplayTutorial,
  onCheckForUpdates,
  onViewDiagnostics,
  onReviewReset,
}: {
  deviceName: string;
  installId: string;
  loginId: string;
  plan: string;
  gateway: string;
  /** Already masked upstream - this pane never sees the key. */
  apiKeyMasked: string;
  launchAtLogin: boolean;
  notifications: boolean;
  version: string;
  onRenameDevice: () => void;
  onCopyInstallId: () => void;
  onUpgradePlan: () => void;
  onReplaceKey: () => void;
  onDisconnect: () => void;
  onToggleLaunchAtLogin: () => void;
  onToggleNotifications: () => void;
  onReplayTutorial: () => void;
  onCheckForUpdates: () => void;
  onViewDiagnostics: () => void;
  onReviewReset: () => void;
}): SettingsSection[] {
  return [
    {
      id: "device",
      title: "Device",
      rows: [
        {
          id: "device",
          icon: "monitorSmartphone",
          label: "Device",
          value: deviceName,
          action: { label: "Rename device", onClick: onRenameDevice },
        },
        {
          id: "install-id",
          icon: "idCard",
          label: "Install ID",
          value: installId,
          mono: true,
          action: { label: "Copy ID", onClick: onCopyInstallId },
        },
      ],
    },
    {
      id: "account",
      title: "Account",
      rows: [
        { id: "login", icon: "user", label: "Login ID", value: loginId },
        {
          id: "plan",
          icon: "receipt",
          label: "Gate plan",
          value: plan,
          action: { label: "Upgrade plan", onClick: onUpgradePlan, external: true },
        },
      ],
    },
    {
      id: "connection",
      title: "Connection",
      rows: [
        { id: "gateway", icon: "globe", label: "Gateway", value: gateway },
        {
          id: "api-key",
          icon: "key",
          label: "API key",
          value: apiKeyMasked,
          mono: true,
          action: { label: "Replace key", onClick: onReplaceKey },
        },
        {
          id: "session",
          icon: "link",
          label: "Active session",
          action: { label: "Disconnect Gate", onClick: onDisconnect, destructive: true },
        },
      ],
    },
    {
      id: "startup",
      title: "Startup",
      rows: [
        {
          id: "launch",
          icon: "power",
          label: "Launch at login",
          description: "Keeps routing on after restart",
          toggle: { on: launchAtLogin, onToggle: onToggleLaunchAtLogin },
        },
        {
          id: "notifications",
          icon: "bell",
          label: "Notifications",
          description: "Alert me when a request is blocked or flagged",
          toggle: { on: notifications, onToggle: onToggleNotifications },
        },
      ],
    },
    {
      id: "about",
      title: "About",
      rows: [
        {
          id: "tutorial",
          icon: "bookOpenText",
          label: "Tutorial",
          action: { label: "Replay tutorial", onClick: onReplayTutorial },
        },
        {
          id: "version",
          icon: "codeXml",
          label: "Version",
          value: version,
          mono: true,
          action: { label: "Check for updates", onClick: onCheckForUpdates },
        },
        {
          id: "diagnostics",
          icon: "info",
          label: "Diagnostics",
          description: "Everything Gate knows about this install, as shareable text",
          action: { label: "View report", onClick: onViewDiagnostics },
        },
      ],
    },
    {
      id: "danger",
      title: "Danger zone",
      danger: true,
      rows: [
        {
          id: "reset",
          icon: "refresh",
          label: "Reset Gate Connect",
          description:
            "Turn routing off, disconnect tools, remove this account or key, and start setup again.",
          action: { label: "Review reset", onClick: onReviewReset, destructive: true },
        },
      ],
    },
  ];
}

export function SettingsPane({ sections }: { sections: SettingsSection[] }) {
  return (
    <div className="flex flex-1 flex-col gap-4 overflow-auto bg-gray-100 p-6">
      <h1 className="text-xl font-medium leading-6 tracking-heading text-neutral-900">
        Settings
      </h1>

      {sections.map((section) => (
        <section key={section.id} className="flex flex-col gap-2">
          <h2
            className={`text-sm font-medium leading-5 ${
              section.danger ? "text-red-600" : "text-neutral-900"
            }`}
          >
            {section.title}
          </h2>

          {/* The danger card tints rather than sitting on plain white. Its
           * red-50 / red-200 pairing is inferred from the heading colour, not
           * sampled from Figma. */}
          <Card className={section.danger ? "border-red-200 bg-red-50" : ""}>
            {section.rows.map((row, i) => (
              <Row key={row.id} row={row} first={i === 0} danger={section.danger} />
            ))}
          </Card>
        </section>
      ))}
    </div>
  );
}

function Row({
  row,
  first,
  danger,
}: {
  row: SettingsRow;
  first: boolean;
  danger?: boolean;
}) {
  return (
    <div
      className={`flex items-center gap-3 px-4 py-3 ${
        first ? "" : danger ? "border-t border-red-200" : "border-t border-base-border"
      }`}
    >
      <Icon name={row.icon} size={16} className="shrink-0 text-neutral-500" />

      <div
        className={`min-w-0 ${row.value === undefined ? "flex-1" : "w-[184px] shrink-0"}`}
      >
        <p className="truncate text-sm font-medium leading-5 text-neutral-900">
          {row.label}
        </p>
        {row.description && (
          <p className="text-base-xs leading-4 text-neutral-600">{row.description}</p>
        )}
      </div>

      {row.value !== undefined && (
        <p
          className={`min-w-0 flex-1 truncate text-sm leading-5 text-neutral-900 ${
            row.mono ? "font-mono" : ""
          }`}
        >
          {row.value}
        </p>
      )}

      {row.toggle && (
        <span className="flex shrink-0 items-center gap-2">
          <span className="text-base-xs font-medium text-neutral-600">
            {row.toggle.on ? "On" : "Off"}
          </span>
          <BaseSwitch
            on={row.toggle.on}
            label={row.label}
            onClick={row.toggle.onToggle}
          />
        </span>
      )}

      {row.action && <ActionButton action={row.action} />}
    </div>
  );
}

function ActionButton({ action }: { action: SettingsAction }) {
  return (
    <button
      type="button"
      onClick={action.onClick}
      className={`flex shrink-0 items-center gap-1.5 rounded-base px-2 py-1 text-base-xs font-medium leading-4 transition-colors focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 ${
        action.destructive
          ? "bg-red-600 text-white hover:bg-red-700 focus-visible:outline-red-600"
          : "border border-base-border bg-base-card text-base-primary shadow-base-2xs hover:bg-gray-50 focus-visible:outline-base-primary"
      }`}
    >
      {action.label}
      {action.external && <Icon name="squareArrowOutUpRight" size={12} />}
    </button>
  );
}
