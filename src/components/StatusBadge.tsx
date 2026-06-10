import type { Status } from "../lib/api";

/**
 * Status pill — cg-system badge: rounded-xs (4px), mono tabular text,
 * 20px height, soft tonal background with darker label per the
 * `.cg-badge` contract. Renders a 6px filled dot in the same hue.
 */

const styles: Record<Status["kind"], string> = {
  not_installed: "bg-ink-100 text-ink-500",
  detected: "bg-warning-100 text-warning-700",
  connected: "bg-success-100 text-success-800",
  drifted: "bg-warning-100 text-warning-700",
  error: "bg-danger-100 text-danger-700",
};

const labels: Record<Status["kind"], string> = {
  not_installed: "not installed",
  detected: "detected",
  connected: "connected",
  drifted: "out of sync",
  error: "error",
};

export function StatusBadge({ status }: { status: Status }) {
  return (
    <span
      className={`inline-flex h-5 shrink-0 items-center gap-1.5 whitespace-nowrap rounded-[4px] px-2 font-mono text-[11px] font-medium tabular-nums ${styles[status.kind]}`}
    >
      <span
        className="h-1.5 w-1.5 rounded-full bg-current opacity-90"
        aria-hidden
      />
      {labels[status.kind]}
    </span>
  );
}
