import { useCallback, useEffect, useState } from "react";
import type { ReactNode } from "react";
import type { Account, ProxyState, ProviderState } from "./lib/api";
import {
  getAccount,
  saveAccount,
  clearAccount,
  proxyStatus,
  proxyEnable,
  proxyDisable,
  proxyTrustCa,
  listProviders,
  providerEnable,
  providerDisable,
} from "./lib/api";
import { FirstRun } from "./screens/FirstRun";
import { Home } from "./screens/Home";
import { ProxyScreen } from "./screens/ProxyScreen";
import { Settings } from "./screens/Settings";
import { Success } from "./screens/Success";
import { ComingSoon } from "./screens/ComingSoon";
import { UpdateBanner } from "./components/UpdateBanner";

type Screen = "loading" | "firstrun" | "home" | "proxy" | "settings" | "success" | "coming-soon";

function hostOf(url: string | undefined): string {
  if (!url) return "";
  try {
    return new URL(url).host;
  } catch {
    return url;
  }
}

export function App() {
  const [screen, setScreen] = useState<Screen>("loading");
  const [account, setAccount] = useState<Account | null>(null);
  const [proxy, setProxy] = useState<ProxyState | null>(null);
  const [proxyBusy, setProxyBusy] = useState(false);
  const [providers, setProviders] = useState<ProviderState[]>([]);
  const [providerError, setProviderError] = useState<string | null>(null);

  // Initial load: account decides first-run vs home; proxy status is
  // best-effort (the proxy commands exist on all three desktop OSes, but a
  // failure here just hides the proxy UI via `showProxy`).
  useEffect(() => {
    let alive = true;
    (async () => {
      const acct = await getAccount().catch(() => null);
      let px: ProxyState | null = null;
      try {
        px = await proxyStatus();
      } catch {
        px = null;
      }
      const provs = await listProviders().catch(() => []);
      if (!alive) return;
      setAccount(acct);
      setProxy(px);
      setProviders(provs);
      setScreen(acct ? "home" : "firstrun");
    })();
    return () => {
      alive = false;
    };
  }, []);

  const refreshAccount = useCallback(async () => {
    setAccount(await getAccount().catch(() => null));
  }, []);

  const onConnected = useCallback(async () => {
    await refreshAccount();
    setScreen("success");
  }, [refreshAccount]);

  const toggleProxy = useCallback(async () => {
    if (proxyBusy) return;
    setProxyBusy(true);
    setProviderError(null);
    try {
      const next = proxy?.running ? await proxyDisable() : await proxyEnable();
      setProxy(next);
      // The master toggle also disconnects/restores providers, so refresh them.
      setProviders(await listProviders().catch(() => []));
    } catch {
      // surfaced state stays; a follow-up status refresh keeps us honest
      try {
        setProxy(await proxyStatus());
      } catch {
        /* noop */
      }
      setProviders(await listProviders().catch(() => []));
    } finally {
      setProxyBusy(false);
    }
  }, [proxy, proxyBusy]);

  const setProvider = useCallback(
    async (slug: string, enabled: boolean) => {
      if (proxyBusy) return;
      setProxyBusy(true);
      setProviderError(null);
      try {
        if (enabled) await providerEnable(slug);
        else await providerDisable(slug);
        // Refresh provider state and proxy (enabling may have flipped a domain).
        setProviders(await listProviders().catch(() => []));
        try {
          setProxy(await proxyStatus());
        } catch {
          /* non-macOS: no proxy subsystem */
        }
      } catch (e) {
        setProviderError(typeof e === "string" ? e : String(e));
        // Re-sync the switch to its true state after a failed toggle.
        setProviders(await listProviders().catch(() => []));
      } finally {
        setProxyBusy(false);
      }
    },
    [proxyBusy],
  );

  const trustCa = useCallback(async () => {
    if (proxyBusy) return;
    setProxyBusy(true);
    try {
      setProxy(await proxyTrustCa());
    } catch {
      // a cancelled trust dialog rejects; re-sync instead of leaking an
      // unhandled rejection with the banner stuck in its old state
      try {
        setProxy(await proxyStatus());
      } catch {
        /* noop */
      }
    } finally {
      setProxyBusy(false);
    }
  }, [proxyBusy]);

  const replaceKey = useCallback(
    async (key: string) => {
      const base = account?.gateway_base_url;
      if (!base) return;
      await saveAccount(base, key);
      await refreshAccount();
    },
    [account, refreshAccount],
  );

  const disconnect = useCallback(async () => {
    if (proxy?.running) {
      // A failed disable can leave system HTTPS pointed at a dead engine
      // port — abort the sign-out and surface it instead of silently
      // proceeding to first-run with the machine's traffic stranded.
      try {
        setProxy(await proxyDisable());
      } catch (err) {
        try {
          setProxy(await proxyStatus());
        } catch {
          /* noop */
        }
        throw err;
      }
    }
    // clear_account disconnects managed tools before wiping the account;
    // if that fails we are still signed in, so let the rejection reach
    // Settings instead of showing first-run over a half-signed-out state.
    await clearAccount();
    setAccount(null);
    setScreen("firstrun");
  }, [proxy]);

  const workspace = hostOf(account?.gateway_base_url);
  const proxyOn = proxy?.running ?? false;
  const domainCount = proxy?.domains.filter((d) => d.enabled && d.supported).length ?? 0;
  const showProxy = proxy !== null;

  let body: ReactNode = null;
  if (screen === "loading") {
    body = null; // blank popover while we resolve account + proxy
  } else if (screen === "firstrun") {
    body = <FirstRun onConnected={onConnected} />;
  } else if (screen === "success") {
    body = (
      <Success
        workspace={workspace}
        proxyOn={proxyOn}
        onDone={() => setScreen("home")}
        onOpenSettings={() => setScreen("settings")}
      />
    );
  } else if (screen === "proxy" && proxy) {
    body = (
      <ProxyScreen
        proxy={proxy}
        providers={providers}
        busy={proxyBusy}
        error={providerError}
        onBack={() => {
          setProviderError(null);
          setScreen("home");
        }}
        onToggleProxy={toggleProxy}
        onSetProvider={setProvider}
        onTrustCa={trustCa}
      />
    );
  } else if (screen === "settings" && account) {
    body = (
      <Settings
        account={account}
        onBack={() => setScreen("home")}
        onReplaceKey={replaceKey}
        onDisconnect={disconnect}
      />
    );
  } else if (screen === "coming-soon") {
    body = <ComingSoon onBack={() => setScreen("home")} />;
  } else {
    // home (and any fallback once loaded)
    body = (
      <Home
        workspace={workspace}
        proxyOn={proxyOn}
        domainCount={domainCount}
        showProxy={showProxy}
        onOpenProxy={() => setScreen("proxy")}
        onToggleProxy={toggleProxy}
        onOpenDirectGateway={() => setScreen("coming-soon")}
        onOpenSettings={() => setScreen("settings")}
      />
    );
  }

  return (
    <div className="flex h-full w-full flex-col overflow-y-auto overflow-x-hidden rounded-gc-lg bg-gc-surface text-gc-ink">
      <UpdateBanner />
      {body}
    </div>
  );
}
