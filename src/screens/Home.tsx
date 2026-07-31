import type { Tool } from "../lib/api";
import type { ClassifiedError } from "../lib/errors";
import { PopHeader } from "../components/gc/PopHeader";
import { Switch, IconButton, SectionLabel, ErrorNote } from "../components/gc/ui";
import { Icon } from "../components/gc/Icon";
import { usePlatform } from "../lib/platform";

/** One truthful pill per tool row. Wash background with a solid dot; the
 * text stays ink where the status color alone can't carry AA contrast. */
function ToolPill({ status }: { status: Tool["status"] }) {
  if (status.kind === "connected") {
    return (
      <span className="inline-flex shrink-0 items-center gap-1.5 rounded-gc-pill bg-[rgba(46,204,113,0.14)] px-2 py-1 text-[11px] font-medium text-[#1f8a4c]">
        <span className="h-1.5 w-1.5 rounded-full bg-gc-success" />
        Routed
      </span>
    );
  }
  if (status.kind === "drifted") {
    return (
      <span className="inline-flex shrink-0 items-center gap-1.5 rounded-gc-pill bg-[rgba(243,156,18,0.12)] px-2 py-1 text-[11px] font-medium text-gc-ink-2">
        <span className="h-1.5 w-1.5 rounded-full bg-gc-warning" />
        Drifted
      </span>
    );
  }
  if (status.kind === "error") {
    return (
      <span className="inline-flex shrink-0 items-center gap-1.5 rounded-gc-pill bg-[rgba(231,76,60,0.12)] px-2 py-1 text-[11px] font-medium text-gc-ink-2">
        <span className="h-1.5 w-1.5 rounded-full bg-gc-error" />
        Error
      </span>
    );
  }
  // detected: installed but not routed through Gate.
  return (
    <span className="inline-flex shrink-0 items-center gap-1.5 rounded-gc-pill bg-gc-sunken px-2 py-1 text-[11px] font-medium text-gc-ink-4">
      <span className="h-1.5 w-1.5 rounded-full bg-gc-ink-5" />
      Not routed
    </span>
  );
}

function toolSubtitle(tool: Tool): string {
  switch (tool.status.kind) {
    case "connected":
      return "Routing through Gate";
    case "drifted":
      return "Configured outside Gate Connect";
    case "error":
      return tool.status.message;
    default:
      return "Installed · not routing";
  }
}

/** Connected home - the Routing card (toggle + drill-in) above the tools
 * ledger: one row per detected tool, each carrying its status pill. */
export function Home({
  workspace,
  proxyOn,
  providerCount,
  showProxy,
  tools,
  error,
  restartHint,
  onDismissRestartHint,
  relaunchHint,
  onDismissRelaunchHint,
  startupRoutingHint,
  onDismissStartupRoutingHint,
  staleAgentsHint,
  onDismissStaleAgents,
  onOpenProxy,
  onToggleProxy,
  onOpenSettings,
}: {
  workspace: string;
  proxyOn: boolean;
  providerCount: number;
  showProxy: boolean;
  tools: Tool[];
  error?: ClassifiedError | null;
  restartHint: boolean;
  onDismissRestartHint: () => void;
  relaunchHint: boolean;
  onDismissRelaunchHint: () => void;
  startupRoutingHint: boolean;
  onDismissStartupRoutingHint: () => void;
  staleAgentsHint: boolean;
  onDismissStaleAgents: () => void;
  onOpenProxy: () => void;
  onToggleProxy: () => void;
  onOpenSettings: () => void;
}) {
  const platform = usePlatform();
  const installedTools = tools.filter((t) => t.status.kind !== "not_installed");
  return (
    <div className="flex flex-col">
      <PopHeader
        workspace={workspace}
        pill={proxyOn ? "connected" : "idle"}
        onGear={onOpenSettings}
      />
      <div className="flex flex-col gap-2.5 p-3.5">
        {showProxy && (
          <div className="relative flex items-center gap-3 rounded-[10px] bg-gc-surface p-3.5 shadow-border transition hover:shadow-border-hover">
            {/* Stretch button carries the drill-in (real button semantics:
                Enter and Space both work); the switch is a sibling, not a
                nested control. */}
            <button
              type="button"
              onClick={onOpenProxy}
              aria-label="Routing details"
              className="absolute inset-0 cursor-pointer rounded-[10px] focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-gc-accent"
            />
            <div
              className={`pointer-events-none relative flex h-9 w-9 shrink-0 items-center justify-center rounded-[9px] ${
                proxyOn ? "bg-gc-accent-wash text-gc-accent" : "bg-gc-sunken text-gc-ink-4"
              }`}
            >
              <Icon name="shieldCheck" size={19} />
            </div>
            <div className="pointer-events-none relative min-w-0 flex-1">
              <div className="text-[13.5px] font-semibold text-gc-ink">Routing</div>
              <div className="mt-0.5 text-[11.5px] text-gc-ink-3">
                {proxyOn
                  ? `On · ${providerCount} provider${providerCount === 1 ? "" : "s"}`
                  : "Off · not routing"}
              </div>
            </div>
            <div className="relative flex shrink-0 items-center gap-1.5">
              <Switch on={proxyOn} label="Route through Gate" onClick={onToggleProxy} />
              <span className="pointer-events-none">
                <Icon name="chevronRight" size={15} stroke={2} className="text-gc-ink-4" />
              </span>
            </div>
          </div>
        )}

        {staleAgentsHint && (
          <div role="status" className="flex items-center gap-2.5 rounded bg-gc-sunken px-3 py-2.5">
            <Icon name="info" size={15} className="shrink-0 text-gc-error" />
            <div className="min-w-0 flex-1 text-[11.5px] leading-snug text-gc-ink-2">
              Gate&rsquo;s local address changed.{" "}
              <span className="font-semibold">Restart your agents</span> to
              reconnect.
            </div>
            <IconButton
              icon="x"
              size={13}
              onClick={onDismissStaleAgents}
              aria-label="Dismiss restart notice"
            />
          </div>
        )}

        {startupRoutingHint && !staleAgentsHint && (
          <div role="status" className="flex items-center gap-2.5 rounded bg-gc-highlight px-3 py-2.5 shadow-border">
            <Icon name="info" size={15} className="shrink-0 text-gc-ink" />
            <div className="min-w-0 flex-1 text-[12px] font-medium leading-snug text-gc-ink">
              Routing is on from your last session. Agents launched before Gate
              Connect started may not route until{" "}
              <span className="font-semibold">restarted</span>.
            </div>
            <IconButton
              icon="x"
              size={13}
              onClick={onDismissStartupRoutingHint}
              aria-label="Dismiss routing notice"
            />
          </div>
        )}

        {platform === "linux" && relaunchHint && (
          <div role="status" className="flex items-center gap-2.5 rounded bg-gc-highlight px-3 py-2.5 shadow-border">
            <Icon name="refresh" size={15} className="shrink-0 text-gc-ink" />
            <div className="min-w-0 flex-1 text-[12px] font-medium leading-snug text-gc-ink">
              Agents already running won’t route through Gate.{" "}
              <span className="font-semibold">Reopen them</span> and they’ll pick
              up the proxy at launch.
            </div>
            <IconButton
              icon="x"
              size={13}
              onClick={onDismissRelaunchHint}
              aria-label="Dismiss reopen notice"
            />
          </div>
        )}

        {restartHint && !(platform === "linux" && relaunchHint) && (
          <div role="status" className="flex items-center gap-2.5 rounded bg-gc-highlight px-3 py-2.5 shadow-border">
            <Icon name="refresh" size={15} className="shrink-0 text-gc-ink" />
            <div className="min-w-0 flex-1 text-[12px] font-medium leading-snug text-gc-ink">
              <span className="font-semibold">Restart your agents</span> to apply the change.
            </div>
            <IconButton
              icon="x"
              size={13}
              onClick={onDismissRestartHint}
              aria-label="Dismiss restart hint"
            />
          </div>
        )}

        {error && <ErrorNote error={error} />}
      </div>

      <SectionLabel>Tools</SectionLabel>
      {installedTools.length > 0 ? (
        <div className="flex flex-col border-t border-gc-line">
          {installedTools.map((tool) => (
            <div
              key={tool.slug}
              className="flex items-center gap-3 border-b border-gc-line px-3.5 py-2.5"
            >
              <div className="min-w-0 flex-1">
                <div className="text-[13px] font-medium text-gc-ink">{tool.name}</div>
                <div className="mt-0.5 truncate text-[11px] text-gc-ink-3">
                  {toolSubtitle(tool)}
                </div>
              </div>
              <ToolPill status={tool.status} />
            </div>
          ))}
        </div>
      ) : (
        <p className="px-3.5 pb-3 text-[11.5px] leading-snug text-gc-ink-3">
          No AI tools detected yet. Install Claude Code, Codex, or OpenCode and
          it will show up here.
        </p>
      )}
    </div>
  );
}
