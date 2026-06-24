import { useCallback, useEffect, useState } from "react";
import type { ProxyDomain, ProxyState } from "../lib/api";
import { proxyDisable, proxyEnable, proxySetDomain, proxyStatus } from "../lib/api";
import { usePlatform } from "../lib/platform";

/**
 * Proxy control surface, rendered at the top of the tool list on macOS and
 * Windows. A master toggle turns the built-in MITM proxy on/off; a disclosure
 * exposes per-provider routing. Enabling installs a trusted root CA (in the
 * macOS keychain / Windows certificate store) and points the OS proxy at a
 * loopback engine; disabling reverts both. The platform-specific wording is
 * swapped via `usePlatform`. The section owns its own state and refreshes
 * after every action.
 */
export function ProxySection() {
  const platform = usePlatform();
  const proxyLabel = platform === "windows" ? "Windows proxy" : "system proxy";
  const trustStore = platform === "windows" ? "certificate store" : "keychain";
  const [state, setState] = useState<ProxyState | null>(null);
  const [loaded, setLoaded] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [settingsOpen, setSettingsOpen] = useState(false);

  const load = useCallback(async () => {
    try {
      setState(await proxyStatus());
      setError(null);
    } catch (err) {
      setError(String(err));
    } finally {
      setLoaded(true);
    }
  }, []);

  useEffect(() => {
    load();
  }, [load]);

  const running = state?.running ?? false;
  const enabledCount = state?.domains.filter((d) => d.enabled && d.supported).length ?? 0;

  const run = useCallback(async (action: () => Promise<ProxyState>) => {
    setBusy(true);
    setError(null);
    try {
      setState(await action());
    } catch (err) {
      // A declined admin prompt surfaces here as a cancel - re-sync so the
      // toggle reflects reality rather than the optimistic click.
      setError(String(err));
      try {
        setState(await proxyStatus());
      } catch {
        /* keep the original error */
      }
    } finally {
      setBusy(false);
    }
  }, []);

  const toggleMaster = () => run(running ? proxyDisable : proxyEnable);
  const toggleDomain = (d: ProxyDomain) => run(() => proxySetDomain(d.slug, !d.enabled));

  const statusLine = !loaded
    ? "Checking…"
    : busy
      ? running
        ? "Turning off…"
        : "Turning on…"
      : running
        ? `On · ${enabledCount} ${enabledCount === 1 ? "provider" : "providers"}${
            state?.port ? ` · :${state.port}` : ""
          }`
        : "Off";

  return (
    <section className="mb-2.5 rounded-md bg-white p-3 shadow-border">
      <div className="flex items-center gap-3">
        <div className="min-w-0 flex-1">
          <div className="text-[13px] font-medium leading-tight tracking-[-0.005em] text-ink-900">Proxy</div>
          <div className="mt-0.5 truncate font-mono text-[11px] leading-tight text-ink-500">{statusLine}</div>
        </div>
        <Switch checked={running} disabled={busy || !loaded} onChange={toggleMaster} label="Toggle proxy" />
      </div>

      {!running && (
        <p className="mt-2 text-[11px] leading-snug text-ink-500">
          Routes supported apps through Gate without per-app setup. Turning on installs a trusted certificate in your{" "}
          {trustStore} and points your {proxyLabel} at Gate. Turning off restores your {proxyLabel} right away; the
          certificate stays trusted so re-enabling is instant.
        </p>
      )}

      {error && (
        <div className="mt-2 rounded-md bg-danger-50 px-2.5 py-2 text-[11px] text-danger-700 shadow-[inset_0_0_0_1px_oklch(0.885_0.062_18.334)]">
          <div className="font-mono break-all">{error}</div>
        </div>
      )}

      <button
        type="button"
        onClick={() => setSettingsOpen((v) => !v)}
        className="mt-2 flex items-center gap-1 text-[11px] font-medium text-ink-500 transition-colors hover:text-ink-700 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ink-400"
        aria-expanded={settingsOpen}
      >
        <svg
          viewBox="0 0 24 24"
          className={`h-3 w-3 transition-transform ${settingsOpen ? "rotate-90" : ""}`}
          fill="none"
          stroke="currentColor"
          strokeWidth="2.5"
          strokeLinecap="round"
          strokeLinejoin="round"
          aria-hidden
        >
          <path d="M9 6l6 6-6 6" />
        </svg>
        Proxy settings
      </button>

      {settingsOpen && (
        <ul className="mt-2 space-y-1">
          {(state?.domains ?? []).map((d) => (
            <DomainRow key={d.slug} domain={d} busy={busy} onToggle={() => toggleDomain(d)} />
          ))}
        </ul>
      )}
    </section>
  );
}

function DomainRow({ domain, busy, onToggle }: { domain: ProxyDomain; busy: boolean; onToggle: () => void }) {
  return (
    <li className="flex items-center gap-3 rounded-md bg-ink-50 px-2.5 py-2">
      <div className="min-w-0 flex-1">
        <div className="truncate text-[12px] font-medium leading-tight text-ink-900">{domain.display_name}</div>
        <div className="mt-0.5 truncate font-mono text-[10px] leading-tight text-ink-500">
          {domain.supported ? domain.hosts.join(", ") : "Not supported yet"}
        </div>
      </div>
      <Switch
        checked={domain.enabled}
        disabled={busy || !domain.supported}
        onChange={onToggle}
        label={`Toggle ${domain.display_name}`}
      />
    </li>
  );
}

function Switch({
  checked,
  disabled,
  onChange,
  label,
}: {
  checked: boolean;
  disabled: boolean;
  onChange: () => void;
  label: string;
}) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={checked}
      aria-label={label}
      disabled={disabled}
      onClick={onChange}
      className={`relative inline-flex h-5 w-9 shrink-0 items-center rounded-full transition-colors focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ink-400 disabled:cursor-not-allowed disabled:opacity-40 ${
        checked ? "bg-success-600" : "bg-ink-300"
      }`}
    >
      <span
        className={`inline-block h-4 w-4 transform rounded-full bg-white shadow transition-transform ${
          checked ? "translate-x-[18px]" : "translate-x-0.5"
        }`}
      />
    </button>
  );
}
