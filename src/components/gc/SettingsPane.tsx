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
  /** Filled red rather than outline: Disconnect, Review reset. */
  destructive?: boolean;
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
      className={`shrink-0 rounded-base px-2 py-1 text-base-xs font-medium leading-4 transition-colors focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 ${
        action.destructive
          ? "bg-red-600 text-white hover:bg-red-700 focus-visible:outline-red-600"
          : "border border-base-border bg-base-card text-base-primary shadow-base-2xs hover:bg-gray-50 focus-visible:outline-base-primary"
      }`}
    >
      {action.label}
    </button>
  );
}
