import { PopHeader } from "../components/gc/PopHeader";
import { Switch, CardButton, IconButton } from "../components/gc/ui";
import { Icon } from "../components/gc/Icon";
import { usePlatform } from "../lib/platform";

/** Connected home - Proxy card (toggle + drill-in) and Direct Gateway card. */
export function Home({
  workspace,
  proxyOn,
  providerCount,
  showProxy,
  error,
  restartHint,
  relaunchHint,
  staleAgentsHint,
  onDismissStaleAgents,
  onOpenProxy,
  onToggleProxy,
  onOpenDirectGateway,
  onOpenSettings,
}: {
  workspace: string;
  proxyOn: boolean;
  providerCount: number;
  showProxy: boolean;
  error?: string | null;
  restartHint: boolean;
  relaunchHint: boolean;
  staleAgentsHint: boolean;
  onDismissStaleAgents: () => void;
  onOpenProxy: () => void;
  onToggleProxy: () => void;
  onOpenDirectGateway: () => void;
  onOpenSettings: () => void;
}) {
  const platform = usePlatform();
  return (
    <div className="flex flex-col">
      <PopHeader
        workspace={workspace}
        pill={proxyOn ? "connected" : "idle"}
        onGear={onOpenSettings}
      />
      <div className="flex flex-col gap-2.5 p-3.5">
        {showProxy && (
          <div
            role="button"
            tabIndex={0}
            onClick={onOpenProxy}
            onKeyDown={(e) => {
              if (e.key === "Enter") onOpenProxy();
            }}
            className="flex cursor-pointer items-center gap-3 rounded-[10px] bg-gc-surface p-3.5 shadow-border transition hover:shadow-border-hover"
          >
            <div
              className={`flex h-9 w-9 shrink-0 items-center justify-center rounded-[9px] ${
                proxyOn ? "bg-gc-accent-wash text-gc-accent" : "bg-gc-sunken text-gc-ink-4"
              }`}
            >
              <Icon name="shieldCheck" size={19} />
            </div>
            <div className="min-w-0 flex-1">
              <div className="text-[13.5px] font-semibold text-gc-ink">Proxy</div>
              <div className="mt-0.5 text-[11.5px] text-gc-ink-3">
                {proxyOn
                  ? `On · ${providerCount} provider${providerCount === 1 ? "" : "s"}`
                  : "Off · not routing"}
              </div>
            </div>
            <div className="flex shrink-0 items-center gap-1.5">
              <span
                className="flex"
                onClick={(e) => {
                  e.stopPropagation();
                  onToggleProxy();
                }}
              >
                <Switch on={proxyOn} />
              </span>
              <Icon name="chevronRight" size={15} stroke={2} className="text-gc-ink-4" />
            </div>
          </div>
        )}

        {staleAgentsHint && (
          <div className="flex items-center gap-2.5 rounded bg-gc-sunken px-3 py-2.5">
            <Icon name="info" size={15} className="shrink-0 text-gc-error" />
            <div className="min-w-0 flex-1 text-[11.5px] leading-snug text-gc-error">
              Gate&rsquo;s local address changed. Restart your AI apps to
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

        {platform === "linux" && relaunchHint && (
          <div className="flex items-center gap-2.5 rounded bg-gc-highlight px-3 py-2.5 shadow-border">
            <Icon name="refresh" size={15} className="shrink-0 text-gc-ink" />
            <div className="min-w-0 flex-1 text-[12px] font-medium leading-snug text-gc-ink">
              Apps already running won’t route through Gate.{" "}
              <span className="font-semibold">Reopen them</span> and they’ll pick
              up the proxy at launch.
            </div>
          </div>
        )}

        {restartHint && !(platform === "linux" && relaunchHint) && (
          <div className="flex items-center gap-2.5 rounded bg-gc-highlight px-3 py-2.5 shadow-border">
            <Icon name="refresh" size={15} className="shrink-0 text-gc-ink" />
            <div className="min-w-0 flex-1 text-[12px] font-medium leading-snug text-gc-ink">
              <span className="font-semibold">Restart your agent</span> to apply the change.
            </div>
          </div>
        )}

        {error && <p className="px-1 text-[11.5px] leading-snug text-gc-error">{error}</p>}

        <CardButton onClick={onOpenDirectGateway}>
          <div className="flex h-9 w-9 shrink-0 items-center justify-center rounded-[9px] bg-gc-sunken text-gc-ink-3">
            <Icon name="layers" size={19} />
          </div>
          <div className="min-w-0 flex-1">
            <div className="text-[13.5px] font-semibold text-gc-ink">Direct Gateway</div>
            <div className="mt-0.5 text-[11.5px] text-gc-ink-3">
              Add Gate models to your coding agents
            </div>
          </div>
          <Icon name="chevronRight" size={15} stroke={2} className="text-gc-ink-4" />
        </CardButton>
      </div>
    </div>
  );
}
