import { PopHeader } from "../components/gc/PopHeader";
import { Switch, CardButton } from "../components/gc/ui";
import { Icon } from "../components/gc/Icon";

/** Connected home - Proxy card (toggle + drill-in) and Direct Gateway card. */
export function Home({
  workspace,
  proxyOn,
  domainCount,
  requestCount,
  showProxy,
  error,
  onOpenProxy,
  onToggleProxy,
  onOpenDirectGateway,
  onOpenSettings,
}: {
  workspace: string;
  proxyOn: boolean;
  domainCount: number;
  requestCount: number;
  showProxy: boolean;
  error?: string | null;
  onOpenProxy: () => void;
  onToggleProxy: () => void;
  onOpenDirectGateway: () => void;
  onOpenSettings: () => void;
}) {
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
                  ? `On · intercepting ${domainCount} domain${domainCount === 1 ? "" : "s"}`
                  : "Off · not routing"}
              </div>
              {proxyOn && (
                <div className="mt-0.5 text-[11.5px] text-gc-ink-3">
                  <span className="font-mono tabular-nums">{requestCount.toLocaleString()}</span>{" "}
                  request{requestCount === 1 ? "" : "s"} routed
                </div>
              )}
            </div>
            <span
              className="flex"
              onClick={(e) => {
                e.stopPropagation();
                onToggleProxy();
              }}
            >
              <Switch on={proxyOn} />
            </span>
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
