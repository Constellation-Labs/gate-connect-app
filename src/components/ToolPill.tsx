import type { Tool } from "../lib/api";

/** One truthful pill per tool. Wash background with a solid dot; the text
 * stays ink where the status color alone can't carry AA contrast. Shared by
 * the Home ledger and the tool detail screen. */
export function ToolPill({ status }: { status: Tool["status"] }) {
  if (status.kind === "connected") {
    return (
      <span className="inline-flex shrink-0 items-center gap-1.5 rounded-gc-pill bg-gc-success-wash px-2 py-1 text-[11px] font-medium text-gc-success-deep">
        <span className="h-1.5 w-1.5 rounded-full bg-gc-success" />
        Routed
      </span>
    );
  }
  if (status.kind === "drifted") {
    return (
      <span className="inline-flex shrink-0 items-center gap-1.5 rounded-gc-pill bg-gc-warning-wash px-2 py-1 text-[11px] font-medium text-gc-ink-2">
        <span className="h-1.5 w-1.5 rounded-full bg-gc-warning" />
        Set up elsewhere
      </span>
    );
  }
  if (status.kind === "error") {
    return (
      <span className="inline-flex shrink-0 items-center gap-1.5 rounded-gc-pill bg-gc-error-wash px-2 py-1 text-[11px] font-medium text-gc-ink-2">
        <span className="h-1.5 w-1.5 rounded-full bg-gc-error" />
        Error
      </span>
    );
  }
  // detected: installed but not routed through Gate.
  return (
    <span className="inline-flex shrink-0 items-center gap-1.5 rounded-gc-pill bg-gc-sunken px-2 py-1 text-[11px] font-medium text-gc-ink-3">
      <span className="h-1.5 w-1.5 rounded-full bg-gc-ink-5" />
      Not routed
    </span>
  );
}

export function toolSubtitle(tool: Tool): string {
  switch (tool.status.kind) {
    case "connected":
      return "Routing through Gate";
    case "drifted":
      return "Gate setup written outside this app";
    case "error":
      return tool.status.message;
    default:
      return "Installed · not routing";
  }
}
