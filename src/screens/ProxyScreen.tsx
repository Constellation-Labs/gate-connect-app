import { useEffect, useState } from "react";
import type { ProxyState, ProviderState } from "../lib/api";
import type { ClassifiedError } from "../lib/errors";
import { launchAtLoginStatus } from "../lib/api";
import { SubHeader, Switch, SectionLabel, Button, IconButton, ErrorNote } from "../components/gc/ui";
import { Icon } from "../components/gc/Icon";
import { usePlatform } from "../lib/platform";

/** Routing detail - the master "Route through Gate" toggle (system proxy),
 * the CA-trust step, and one switch per provider. Each provider switch
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
  onDismissRestartHint,
  relaunchHint,
  onDismissRelaunchHint,
  codexDrifted,
}: {
  proxy: ProxyState;
  providers: ProviderState[];
  busy: boolean;
  error: ClassifiedError | null;
  onBack: () => void;
  onToggleProxy: () => void;
  onSetProvider: (slug: string, enabled: boolean) => void;
  onTrustCa: () => void;
  restartHint: boolean;
  onDismissRestartHint: () => void;
  relaunchHint: boolean;
  onDismissRelaunchHint: () => void;
  codexDrifted: boolean;
}) {
  const platform = usePlatform();
  const trustStore = platform === "windows" ? "certificate store" : "keychain";
  // Whether Launch at login is on, so the restart tip only shows when it
  // would actually help (recognition over recall: don't send the user to
  // Settings to check a state we can read). null while loading = no tip.
  const [launchAtLogin, setLaunchAtLogin] = useState<boolean | null>(null);
  useEffect(() => {
    let alive = true;
    launchAtLoginStatus()
      .then((status) => {
        if (alive) setLaunchAtLogin(status.enabled);
      })
      .catch(() => {});
    return () => {
      alive = false;
    };
  }, []);

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
              ? "On · routing enabled providers through Gate"
              : "Off · nothing routes through Gate"}
          </div>
          {launchAtLogin === false && (
            <div className="mt-1 text-[11px] leading-snug text-gc-ink-3">
              Turn on Launch at login in Settings to keep routing on after a
              restart.
            </div>
          )}
        </div>
        <Switch
          on={proxy.running}
          label="Route through Gate"
          disabled={busy}
          onClick={onToggleProxy}
        />
      </div>

      {proxy.running && !proxy.ca_trusted && (
        <div className="mx-3.5 mb-2 rounded-[10px] bg-gc-surface p-3.5 shadow-border">
          <div className="flex items-center gap-2.5">
            <div className="flex h-8 w-8 shrink-0 items-center justify-center rounded-[9px] bg-[rgba(243,156,18,0.12)] text-gc-warning">
              <Icon name="shieldCheck" size={16} />
            </div>
            <div className="text-[13px] font-semibold text-gc-ink">
              Trust the Gate certificate
            </div>
          </div>
          <p className="mt-2 text-[11.5px] leading-snug text-gc-ink-2">
            Desktop apps with no gateway setting route through Gate&rsquo;s
            local proxy, which needs a certificate your {trustStore} trusts.
            The certificate and its private key are created on this machine
            and never leave it. Until it&rsquo;s trusted, those apps
            don&rsquo;t route.
          </p>
          <p className="mt-1.5 text-[11px] leading-snug text-gc-ink-3">
            You can remove it anytime in Settings under Certificate.
          </p>
          <Button variant="accent" full className="mt-2.5" disabled={busy} onClick={onTrustCa}>
            Trust certificate
          </Button>
        </div>
      )}

      {platform === "linux" && relaunchHint && (
        <div role="status" className="mx-3.5 mb-1 flex items-center gap-2.5 rounded bg-gc-highlight px-3 py-2.5 shadow-border">
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
        <div role="status" className="mx-3.5 mb-1 flex items-center gap-2.5 rounded bg-gc-highlight px-3 py-2.5 shadow-border">
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

      <SectionLabel>Providers</SectionLabel>
      <div className="flex flex-col border-t border-gc-line">
        {providers.map((p) => (
          <div
            key={p.slug}
            className="flex items-center gap-3 border-b border-gc-line px-3.5 py-2.5"
          >
            <div className="min-w-0 flex-1">
              <div className="text-[13px] font-medium text-gc-ink">{p.display_name}</div>
              <div className="mt-0.5 truncate text-[11px] text-gc-ink-3">{p.subtitle}</div>
            </div>
            <Switch
              on={p.enabled}
              label={p.display_name}
              disabled={busy || !p.available}
              onClick={() => onSetProvider(p.slug, !p.enabled)}
            />
          </div>
        ))}
        {providers.length === 0 && (
          <div className="px-3.5 py-3 text-[11.5px] text-gc-ink-3">No providers available.</div>
        )}
      </div>

      {codexDrifted && (
        <div className="mx-3.5 mt-2 flex items-center gap-2.5 rounded bg-gc-sunken px-3 py-2.5">
          <Icon name="info" size={15} className="shrink-0 text-gc-ink-3" />
          <div className="min-w-0 flex-1 text-[11.5px] leading-snug text-gc-ink-2">
            Codex has a Gate setup written outside this app. Turning its
            provider on replaces that configuration and manages the key from
            your keychain.
          </div>
        </div>
      )}

      {error && <ErrorNote error={error} className="mx-3.5 mt-2" />}
    </div>
  );
}
