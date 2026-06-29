import type { ProxyState, ProviderState, Tool } from "../lib/api";
import { SubHeader, Switch, SectionLabel } from "../components/gc/ui";
import { Icon } from "../components/gc/Icon";
import { usePlatform } from "../lib/platform";

/** Subtitle for a tool-integration row, keyed off its current status. */
function toolSubtitle(tool: Tool): string {
  switch (tool.status.kind) {
    case "connected":
      return "On · routed through Gate";
    case "not_installed":
      return "Not installed";
    case "drifted":
      return "Config changed outside Gate";
    case "error":
      return tool.status.message;
    default:
      return "Off · not routed through Gate";
  }
}

/** Routing detail - the master "Route through Gate" toggle (system proxy),
 * the CA-trust notice, and one switch per provider. Each provider switch
 * orchestrates its config integration and (when the proxy is running) its
 * proxy domain, so the user never sees the proxy-vs-config split.
 * Wires to proxy_enable/disable, provider_enable/disable, proxy_trust_ca. */
export function ProxyScreen({
  proxy,
  providers,
  busy,
  error,
  onBack,
  onToggleProxy,
  onSetProvider,
  onTrustCa,
  restartHint,
  relaunchHint,
  openClaw,
  toolBusy,
  onToggleOpenClaw,
  hermes,
  onToggleHermes,
}: {
  proxy: ProxyState;
  providers: ProviderState[];
  busy: boolean;
  error: string | null;
  onBack: () => void;
  onToggleProxy: () => void;
  onSetProvider: (slug: string, enabled: boolean) => void;
  onTrustCa: () => void;
  restartHint: boolean;
  relaunchHint: boolean;
  openClaw: Tool | null;
  toolBusy: boolean;
  onToggleOpenClaw: () => void;
  hermes: Tool | null;
  onToggleHermes: () => void;
}) {
  const platform = usePlatform();
  const trustStore = platform === "windows" ? "certificate store" : "keychain";
  return (
    <div className="flex flex-col">
      <SubHeader title="Routing" onBack={onBack} />

      <div className="flex items-start gap-3 px-3.5 py-3.5">
        <div
          className={`flex h-[34px] w-[34px] shrink-0 items-center justify-center rounded-[9px] ${
            proxy.running ? "bg-gc-accent-wash text-gc-accent" : "bg-gc-sunken text-gc-ink-4"
          }`}
        >
          <Icon name="shieldCheck" size={18} />
        </div>
        <div className="min-w-0 flex-1">
          <div className="text-[14px] font-semibold text-gc-ink">Route through Gate</div>
          <div className="mt-0.5 text-[11.5px] leading-snug text-gc-ink-3">
            {proxy.running
              ? "On · routing your enabled apps through Gate"
              : "Off · nothing is routed through Gate"}
          </div>
          <div className="mt-1 text-[11px] leading-snug text-gc-ink-4">
            Turn on Launch at login in Settings to keep routing on after a restart.
          </div>
        </div>
        <Switch on={proxy.running} disabled={busy} onClick={onToggleProxy} />
      </div>

      {proxy.running && !proxy.ca_trusted && (
        <div className="mx-3.5 mb-1 flex items-center gap-2.5 rounded bg-[rgba(243,156,18,0.1)] px-3 py-2.5">
          <Icon name="info" size={15} className="shrink-0 text-gc-warning" />
          <div className="min-w-0 flex-1 text-[11.5px] leading-snug text-gc-ink-2">
            The local certificate isn’t trusted in your {trustStore} yet.
          </div>
          <button
            type="button"
            onClick={onTrustCa}
            disabled={busy}
            className="shrink-0 text-[12px] font-medium text-gc-accent disabled:opacity-40"
          >
            Trust
          </button>
        </div>
      )}

      {platform === "linux" && relaunchHint && (
        <div className="mx-3.5 mb-1 flex items-center gap-2.5 rounded bg-gc-sunken px-3 py-2.5">
          <Icon name="info" size={15} className="shrink-0 text-gc-ink-3" />
          <div className="min-w-0 flex-1 text-[11.5px] leading-snug text-gc-ink-2">
            Apps already running won’t route through Gate. Log out and back in,
            then reopen them so they pick up the proxy at launch.
          </div>
        </div>
      )}

      <SectionLabel>Providers</SectionLabel>
      <div className="flex flex-col border-t border-gc-line">
        {providers.map((p) => (
          <div
            key={p.slug}
            className="flex items-center gap-3 border-b border-gc-line px-3.5 py-2.5"
          >
            <div className="min-w-0 flex-1">
              <div className="text-[13px] font-medium text-gc-ink">{p.display_name}</div>
              <div className="mt-0.5 truncate text-[11px] text-gc-ink-4">{p.subtitle}</div>
            </div>
            <Switch
              on={p.enabled}
              disabled={busy || !p.available}
              onClick={() => onSetProvider(p.slug, !p.enabled)}
            />
          </div>
        ))}
        {providers.length === 0 && (
          <div className="px-3.5 py-3 text-[11.5px] text-gc-ink-4">No providers available.</div>
        )}
      </div>

      {restartHint && (
        <div className="mx-3.5 mt-2 flex items-center gap-2.5 rounded bg-gc-sunken px-3 py-2.5">
          <Icon name="info" size={15} className="shrink-0 text-gc-ink-3" />
          <div className="min-w-0 flex-1 text-[11.5px] leading-snug text-gc-ink-2">
            Restart your agent to apply the change.
          </div>
        </div>
      )}

      {(openClaw || hermes) && (
        <>
          <SectionLabel>Tool integrations</SectionLabel>
          <div className="flex flex-col border-t border-gc-line">
            {openClaw && (
              <div className="flex items-center gap-3 border-b border-gc-line px-3.5 py-2.5">
                <div className="min-w-0 flex-1">
                  <div className="text-[13px] font-medium text-gc-ink">{openClaw.name}</div>
                  <div className="mt-0.5 truncate text-[11px] text-gc-ink-4">
                    {toolSubtitle(openClaw)}
                  </div>
                </div>
                <Switch
                  on={openClaw.status.kind === "connected"}
                  disabled={toolBusy || openClaw.status.kind === "not_installed"}
                  onClick={onToggleOpenClaw}
                />
              </div>
            )}
            {hermes && (
              <div className="flex items-center gap-3 border-b border-gc-line px-3.5 py-2.5">
                <div className="min-w-0 flex-1">
                  <div className="text-[13px] font-medium text-gc-ink">{hermes.name}</div>
                  <div className="mt-0.5 truncate text-[11px] text-gc-ink-4">
                    {toolSubtitle(hermes)}
                  </div>
                </div>
                <Switch
                  on={hermes.status.kind === "connected"}
                  disabled={toolBusy || hermes.status.kind === "not_installed"}
                  onClick={onToggleHermes}
                />
              </div>
            )}
          </div>
        </>
      )}

      {error && (
        <div className="mx-3.5 mt-2 rounded bg-[rgba(243,156,18,0.1)] px-3 py-2 text-[11.5px] leading-snug text-gc-ink-2">
          {error}
        </div>
      )}
    </div>
  );
}
