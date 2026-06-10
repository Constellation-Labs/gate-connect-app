import { useCallback, useEffect, useState } from "react";
import type { ReactNode } from "react";
import type { Account, ProxyState } from "./lib/api";
import {
  getAccount,
  saveAccount,
  clearAccount,
  proxyStatus,
  proxyEnable,
  proxyDisable,
  proxySetDomain,
  proxyTrustCa,
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

  // Initial load: account decides first-run vs home; proxy status is best-effort
  // (the proxy commands exist on macOS + Windows — on Linux they throw and we
  // hide the proxy UI via `showProxy`).
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
      if (!alive) return;
      setAccount(acct);
      setProxy(px);
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
    try {
      const next = proxy?.running ? await proxyDisable() : await proxyEnable();
      setProxy(next);
    } catch {
      // surfaced state stays; a follow-up status refresh keeps us honest
      try {
        setProxy(await proxyStatus());
      } catch {
        /* noop */
      }
    } finally {
      setProxyBusy(false);
    }
  }, [proxy, proxyBusy]);

  const setDomain = useCallback(
    async (slug: string, enabled: boolean) => {
      if (proxyBusy) return;
      setProxyBusy(true);
      try {
        setProxy(await proxySetDomain(slug, enabled));
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
      try {
        setProxy(await proxyDisable());
      } catch {
        /* noop */
      }
    }
    await clearAccount().catch(() => undefined);
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
        busy={proxyBusy}
        onBack={() => setScreen("home")}
        onToggleProxy={toggleProxy}
        onSetDomain={setDomain}
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
