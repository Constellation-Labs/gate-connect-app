import type { Tool } from "../lib/api";
import type { ClassifiedError } from "../lib/errors";
import { PopHeader } from "../components/gc/PopHeader";
import { Switch, IconButton, SectionLabel, ErrorNote } from "../components/gc/ui";
import { ToolPill, toolSubtitle } from "../components/ToolPill";
import { Icon } from "../components/gc/Icon";
import { usePlatform } from "../lib/platform";

/** Connected home - the Routing card (toggle + drill-in) above the tools
 * ledger: one row per detected tool, each carrying its status pill. */
export function Home({
  workspace,
  proxyOn,
  caTrusted,
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
  onCloseAgents,
  staleAgentsHint,
  onDismissStaleAgents,
  onOpenProxy,
  onOpenTool,
  onToggleProxy,
  onOpenSettings,
}: {
  workspace: string;
  proxyOn: boolean;
  caTrusted: boolean;
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
  onCloseAgents: () => void;
  staleAgentsHint: boolean;
  onDismissStaleAgents: () => void;
  onOpenProxy: () => void;
  onOpenTool: (slug: string) => void;
  onToggleProxy: () => void;
  onOpenSettings: () => void;
}) {
  const platform = usePlatform();
  const installedTools = tools.filter((t) => t.status.kind !== "not_installed");
  // Routing up but the CA untrusted means config tools route while
  // proxy-routed apps silently don't: half-on, and the pill says so.
  const partial = proxyOn && !caTrusted;
  return (
    <div className="flex flex-col">
      <PopHeader
        workspace={workspace}
        pill={proxyOn ? (partial ? "partial" : "connected") : "idle"}
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
                {!proxyOn
                  ? "Off · not routing"
                  : partial
                    ? "On · certificate not trusted yet"
                    : `On · ${providerCount} provider${providerCount === 1 ? "" : "s"}`}
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
            {/* Opens the full takeover (confirm step, close, result); the
                ellipsis signals more steps follow. */}
            <button
              type="button"
              onClick={onCloseAgents}
              className="shrink-0 text-[12px] font-medium text-gc-accent transition hover:text-gc-accent-ink"
            >
              Close agents…
            </button>
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
            <button
              key={tool.slug}
              type="button"
              onClick={() => onOpenTool(tool.slug)}
              className="flex items-center gap-3 border-b border-gc-line px-3.5 py-2.5 text-left transition hover:bg-gc-subtle focus-visible:outline focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-gc-accent"
            >
              <div className="min-w-0 flex-1">
                <div className="text-[13px] font-medium text-gc-ink">{tool.name}</div>
                <div className="mt-0.5 truncate text-[11px] text-gc-ink-3">
                  {toolSubtitle(tool)}
                </div>
              </div>
              <ToolPill status={tool.status} />
              <Icon name="chevronRight" size={14} stroke={2} className="text-gc-ink-4" />
            </button>
          ))}
        </div>
      ) : (
        <p className="px-3.5 pb-3 text-[11.5px] leading-snug text-gc-ink-3">
          No AI tools detected yet. Tools like Claude Code, Codex, and OpenCode
          show up here once installed; the Claude and ChatGPT desktop apps are
          covered by Routing without needing a row.
        </p>
      )}
    </div>
  );
}
