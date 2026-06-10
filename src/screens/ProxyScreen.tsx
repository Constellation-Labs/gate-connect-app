import type { ProxyState } from "../lib/api";
import { SubHeader, Switch, SectionLabel } from "../components/gc/ui";
import { Icon } from "../components/gc/Icon";
import { usePlatform } from "../lib/platform";

/** Proxy detail — master route toggle, CA-trust notice, and per-provider
 *  domain toggles. Wires to proxy_enable/disable, proxy_set_domain, proxy_trust_ca. */
export function ProxyScreen({
  proxy,
  busy,
  onBack,
  onToggleProxy,
  onSetDomain,
  onTrustCa,
}: {
  proxy: ProxyState;
  busy: boolean;
  onBack: () => void;
  onToggleProxy: () => void;
  onSetDomain: (slug: string, enabled: boolean) => void;
  onTrustCa: () => void;
}) {
  const platform = usePlatform();
  const trustStore = platform === "windows" ? "certificate store" : "keychain";
  const enabledCount = proxy.domains.filter((d) => d.enabled && d.supported).length;
  return (
    <div className="flex flex-col">
      <SubHeader title="Proxy" onBack={onBack} />

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
              ? `On · intercepting ${enabledCount} domain${enabledCount === 1 ? "" : "s"}`
              : "Off · apps connect directly to their providers"}
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

      <SectionLabel>Providers</SectionLabel>
      <div className="flex flex-col border-t border-gc-line">
        {proxy.domains.map((d) => (
          <div key={d.slug} className="flex items-center gap-3 border-b border-gc-line px-3.5 py-2.5">
            <div className="min-w-0 flex-1">
              <div className="text-[13px] font-medium text-gc-ink">{d.display_name}</div>
              <div className="mt-0.5 truncate font-mono text-[10.5px] text-gc-ink-4">{d.hosts.join(", ")}</div>
            </div>
            {d.supported ? (
              <Switch on={d.enabled} disabled={busy} onClick={() => onSetDomain(d.slug, !d.enabled)} />
            ) : (
              <span className="shrink-0 text-[11px] font-medium text-gc-ink-5">Soon</span>
            )}
          </div>
        ))}
      </div>
    </div>
  );
}
