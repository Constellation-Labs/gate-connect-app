import type { Installation } from "../../lib/activity";

/**
 * Which installation the Overview is reading (AG-572 AC 1).
 *
 * A native `<select>`, styled to the `base.*` tokens rather than reimplemented.
 * The list is short, the options are identifiers, and the platform's own popup
 * gets keyboard behaviour, scrolling and the OS text-size setting for free -
 * none of which a hand-rolled menu would have on all three platforms.
 *
 * "All installations" is the first option and the default, because attribution
 * starts with the gateway migration that added it: everything sent before then,
 * and everything sent by curl or CI, has no installation, and scoping by default
 * would quietly drop it out of totals the user could already see.
 *
 * The picker is hidden entirely until the gateway reports more than one
 * installation. With nothing to choose between, a control that only ever says
 * "All installations" is furniture that implies a filter is doing something.
 */
export function InstallationPicker({
  installations,
  value,
  onChange,
}: {
  installations: Installation[];
  /** The scope the reading actually covers, or `null` for org-wide. */
  value: string | null;
  onChange: (installId: string | null) => void;
}) {
  if (installations.length < 2) return null;
  return (
    <label className="flex items-center gap-2">
      <span className="text-base-xs text-base-muted-foreground">Installation</span>
      <select
        className="rounded-base border border-base-input bg-base-card px-2 py-1 font-mono text-base-xs text-neutral-900"
        value={value ?? ""}
        onChange={(e) => onChange(e.target.value === "" ? null : e.target.value)}
      >
        <option value="">All installations</option>
        {installations.map((i) => (
          <option key={i.installId} value={i.installId}>
            {/* The gateway decides which one is this machine, from the id the
                app sent with the request. Saying so is the whole point: a raw
                uuid tells the user nothing about which laptop it is. */}
            {i.current ? `${i.label} (this machine)` : i.label}
          </option>
        ))}
      </select>
    </label>
  );
}
