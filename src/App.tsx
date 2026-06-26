import { useCallback, useEffect, useState } from "react";
import type { ReactNode } from "react";
import type { Account, ProxyState, ProviderState, Tool } from "./lib/api";
import { relaunch } from "@tauri-apps/plugin-process";
import {
  getAccount,
  saveAccount,
  clearAccount,
  switchGateway,
  proxyStatus,
  proxyEnable,
  proxyDisable,
  proxyTrustCa,
  listProviders,
  providerEnable,
  providerDisable,
  listTools,
  connectTool,
  disconnectTool,
  toolStatus,
  unpinPopover,
} from "./lib/api";
import { FirstRun } from "./screens/FirstRun";
import { Home } from "./screens/Home";
import { ProxyScreen } from "./screens/ProxyScreen";
import { Settings } from "./screens/Settings";
import { Success } from "./screens/Success";
import { ComingSoon } from "./screens/ComingSoon";
import { UpdateBanner } from "./components/UpdateBanner";
import { LinuxTitleBar } from "./components/LinuxTitleBar";
import { ConstellationHexMark } from "./components/gc/ConstellationHexMark";
import { track, trackError } from "./lib/analytics";
import { usePlatform } from "./lib/platform";

type Screen = "loading" | "firstrun" | "home" | "proxy" | "settings" | "success" | "coming-soon";

// Providers hidden from the UI for now. Slugs match the backend provider list.
const HIDDEN_PROVIDER_SLUGS = new Set(["openrouter"]);

function hostOf(url: string | undefined): string {
  if (!url) return "";
  try {
    return new URL(url).host;
  } catch {
    return url;
  }
}

export function App() {
  const platform = usePlatform();
  const [screen, setScreen] = useState<Screen>("loading");
  const [account, setAccount] = useState<Account | null>(null);
  const [proxy, setProxy] = useState<ProxyState | null>(null);
  const [proxyBusy, setProxyBusy] = useState(false);
  const [providers, setProviders] = useState<ProviderState[]>([]);
  const [providerError, setProviderError] = useState<string | null>(null);
  const [openClaw, setOpenClaw] = useState<Tool | null>(null);
  const [toolBusy, setToolBusy] = useState(false);
  // Set after a successful routing change; the Routing screen shows a
  // "restart your agent" note. Cleared when the user leaves the screen.
  const [restartHint, setRestartHint] = useState(false);
  // Flashed briefly when routing is turned on; the Routing screen shows a
  // Linux-only "relaunch your already-open apps" note that auto-dismisses.
  const [relaunchHint, setRelaunchHint] = useState(false);
  useEffect(() => {
    if (!relaunchHint) return;
    const t = setTimeout(() => setRelaunchHint(false), 8000);
    return () => clearTimeout(t);
  }, [relaunchHint]);

  // Initial load: account decides first-run vs home; proxy status is
  // best-effort (the proxy commands exist on all three desktop OSes, but a
  // failure here just hides the proxy UI via `showProxy`).
  useEffect(() => {
    let alive = true;
    (async () => {
      const acct = await getAccount().catch(() => null);
      let px: ProxyState | null;
      try {
        px = await proxyStatus();
      } catch {
        px = null;
      }
      const provs = await listProviders().catch(() => []);
      const tools = await listTools().catch(() => []);
      if (!alive) return;
      setAccount(acct);
      setProxy(px);
      setProviders(provs);
      setOpenClaw(tools.find((t) => t.slug === "openclaw") ?? null);
      setScreen(acct ? "home" : "firstrun");
      track("app_launched", { has_account: !!acct, proxy_available: px !== null });
    })();
    return () => {
      alive = false;
    };
  }, []);

  // On first launch the popover is pinned open so the macOS keychain dialog
  // (triggered by the initial load reading the key) can't dismiss it before
  // it's seen. Once the user actually interacts, release the pin so normal
  // click-away dismissal resumes.
  useEffect(() => {
    const engage = () => {
      void unpinPopover().catch(() => {});
      window.removeEventListener("pointerdown", engage);
      window.removeEventListener("keydown", engage);
    };
    window.addEventListener("pointerdown", engage);
    window.addEventListener("keydown", engage);
    return () => {
      window.removeEventListener("pointerdown", engage);
      window.removeEventListener("keydown", engage);
    };
  }, []);

  const refreshAccount = useCallback(async () => {
    setAccount(await getAccount().catch(() => null));
  }, []);

  const onConnected = useCallback(async () => {
    await refreshAccount();
    track("signed_in");
    setScreen("success");
  }, [refreshAccount]);

  const toggleProxy = useCallback(async () => {
    if (proxyBusy) return;
    setProxyBusy(true);
    setProviderError(null);
    try {
      const next = proxy?.running ? await proxyDisable() : await proxyEnable();
      setProxy(next);
      track(next.running ? "proxy_enabled" : "proxy_disabled");
      setRestartHint(true);
      if (next.running) setRelaunchHint(true);
      // The master toggle also disconnects/restores providers, so refresh them.
      let current = await listProviders().catch(() => []);
      // Turning the proxy on turns every available integration on with it.
      if (next.running) {
        await Promise.all(
          current
            .filter((p) => p.available && !p.enabled)
            .map((p) => providerEnable(p.slug)),
        );
        current = await listProviders().catch(() => current);
      }
      setProviders(current);
    } catch (e) {
      trackError(e, "generic");
      // Surface why the toggle failed (e.g. on Linux the CA-trust admin step
      // or a missing network service) instead of silently reverting - a
      // swallowed error reads as "the toggle does nothing".
      setProviderError(typeof e === "string" ? e : String(e));
      // Re-sync to the true state after the failed toggle.
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
        track("provider_toggled", { provider: slug, enabled });
        setRestartHint(true);
        if (enabled) setRelaunchHint(true);
        // Refresh provider state and proxy (enabling may have flipped a domain).
        setProviders(await listProviders().catch(() => []));
        try {
          setProxy(await proxyStatus());
        } catch {
          /* non-macOS: no proxy subsystem */
        }
      } catch (e) {
        setProviderError(typeof e === "string" ? e : String(e));
        trackError(e, "generic");
        // Re-sync the switch to its true state after a failed toggle.
        setProviders(await listProviders().catch(() => []));
      } finally {
        setProxyBusy(false);
      }
    },
    [proxyBusy],
  );

  const toggleOpenClaw = useCallback(async () => {
    if (toolBusy || !openClaw) return;
    setToolBusy(true);
    setProviderError(null);
    try {
      const connected = openClaw.status.kind === "connected";
      const status = connected
        ? await disconnectTool(openClaw.slug)
        : await connectTool(openClaw.slug, openClaw.default_upstream_url);
      setOpenClaw({ ...openClaw, status });
    } catch (e) {
      // Surface the failure — a swallowed error reads as "the toggle does
      // nothing". Then re-sync the switch to its true state.
      setProviderError(typeof e === "string" ? e : String(e));
      trackError(e, "generic");
      const status = await toolStatus(openClaw.slug).catch(() => null);
      if (status) setOpenClaw({ ...openClaw, status });
    } finally {
      setToolBusy(false);
    }
  }, [openClaw, toolBusy]);

  const trustCa = useCallback(async () => {
    if (proxyBusy) return;
    setProxyBusy(true);
    try {
      setProxy(await proxyTrustCa());
      track("ca_trusted");
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
      track("key_replaced");
    },
    [account, refreshAccount],
  );

  const switchGatewayServer = useCallback(async (url: string) => {
    await switchGateway(url);
    // Switching forgets the stored key, disconnects managed tools, and leaves
    // the running proxy/providers pointed at the old gateway. Rather than patch
    // all that live, relaunch into a clean session that re-reads the new
    // account and lets the user enter an environment-appropriate key.
    await relaunch();
  }, []);

  const disconnect = useCallback(async () => {
    if (proxy?.running) {
      // A failed disable can leave system HTTPS pointed at a dead engine
      // port - abort the sign-out and surface it instead of silently
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
    track("disconnected");
    setAccount(null);
    setScreen("firstrun");
  }, [proxy]);

  const workspace = hostOf(account?.gateway_base_url);
  const proxyOn = proxy?.running ?? false;
  const domainCount = proxy?.domains.filter((d) => d.enabled && d.supported).length ?? 0;
  const showProxy = proxy !== null;

  let body: ReactNode;
  if (screen === "loading") {
    // Startup screen while we resolve account + proxy (and the macOS keychain
    // dialog reads the key) - show the brand lockup instead of a blank popover.
    body = (
      <div className="flex flex-1 flex-col items-center justify-center gap-3 py-16">
        <ConstellationHexMark size={40} fill="#002a5f" />
        <span className="text-[15px] font-semibold tracking-[-0.02em] text-gc-navy">
          Gate <span className="text-gc-accent">Connect</span>
        </span>
      </div>
    );
  } else if (screen === "firstrun") {
    body = <FirstRun onConnected={onConnected} initialGateway={account?.gateway_base_url} />;
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
        providers={providers.filter((p) => !HIDDEN_PROVIDER_SLUGS.has(p.slug))}
        busy={proxyBusy}
        error={providerError}
        restartHint={restartHint}
        relaunchHint={relaunchHint}
        onBack={() => {
          setProviderError(null);
          setRestartHint(false);
          setRelaunchHint(false);
          setScreen("home");
        }}
        onToggleProxy={toggleProxy}
        onSetProvider={setProvider}
        onTrustCa={trustCa}
        openClaw={openClaw}
        toolBusy={toolBusy}
        onToggleOpenClaw={toggleOpenClaw}
      />
    );
  } else if (screen === "settings" && account) {
    body = (
      <Settings
        account={account}
        onBack={() => setScreen("home")}
        onReplaceKey={replaceKey}
        onDisconnect={disconnect}
        onSwitchGateway={switchGatewayServer}
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
        error={providerError}
        onOpenProxy={() => setScreen("proxy")}
        onToggleProxy={toggleProxy}
        onOpenDirectGateway={() => setScreen("coming-soon")}
        onOpenSettings={() => setScreen("settings")}
      />
    );
  }

  return (
    <div
      className={`flex h-full w-full flex-col overflow-y-auto overflow-x-hidden bg-gc-surface text-gc-ink${
        // Linux runs as a borderless, opaque window, so a rounded card just
        // exposes the square window corners behind it. macOS/Windows round the
        // window itself, so the card rounds to match.
        platform === "linux" ? "" : " rounded-gc-lg"
      }${
        // While a proxy toggle is in flight, show the OS busy cursor everywhere
        // (the `!` overrides children's cursor-pointer / cursor-not-allowed) so
        // the slow enable/disable reads as "working", not "did nothing".
        proxyBusy ? " cursor-wait [&_*]:!cursor-wait" : ""
      }`}
    >
      {platform === "linux" && <LinuxTitleBar />}
      <UpdateBanner />
      {body}
    </div>
  );
}
