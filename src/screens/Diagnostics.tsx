import { useEffect, useState } from "react";
import type {
  Account,
  Diagnostics as BackendDiagnostics,
  LaunchAtLoginStatus,
  OAuthStatus,
  ProviderState,
  ProxyState,
  RunningAgents,
  Tool,
} from "../lib/api";
import {
  diagnostics as fetchDiagnostics,
  launchAtLoginStatus,
  routedClientsStale,
  runningAgents as fetchRunningAgents,
} from "../lib/api";
import { buildDiagnosticsReport } from "../lib/diagnosticsReport";
import { trackError } from "../lib/analytics";
import { usePlatform } from "../lib/platform";
import { SubHeader, Button } from "../components/gc/ui";
import { Icon } from "../components/gc/Icon";

/** The whole state of this install, as text a user can hand to someone else.
 *
 * Shown before it is copied, never copied blind. This app installs a root
 * certificate, runs a local MITM proxy and holds a credential; a button that
 * silently loads the clipboard with an unseen description of that setup is the
 * opposite of the reassurance the rest of the popover is built on. So the
 * report renders in full, the user reads it, and Copy takes exactly what is on
 * screen. The report itself carries no key, token or password - see
 * `diagnosticsReport.ts` for what is excluded and why.
 *
 * The freshness rule: everything the ledger already knows (account, routing,
 * providers, tools) is passed in from App rather than re-read here, so the
 * paste matches the screen the user just left. Only the facts the webview has
 * no other way to see - OS build, persisted ports, the live OS-side proxy
 * readback, the process counts - are fetched on mount.
 */
export function Diagnostics({
  onBack,
  version,
  account,
  oauth,
  proxy,
  providers,
  tools,
}: {
  onBack: () => void;
  version: string;
  account: Account | null;
  oauth: OAuthStatus | null;
  proxy: ProxyState | null;
  providers: ProviderState[];
  tools: Tool[];
}) {
  const platform = usePlatform();
  const [backend, setBackend] = useState<BackendDiagnostics | null>(null);
  const [launchAtLogin, setLaunchAtLogin] = useState<LaunchAtLoginStatus | null>(null);
  const [clientsStale, setClientsStale] = useState(false);
  const [agents, setAgents] = useState<RunningAgents | null>(null);
  // When the snapshot was taken, frozen rather than read at render time. The
  // panel re-renders (the Copy button flashes), and a `new Date()` in the body
  // would move the timestamp *after* the user copied it - so the paste and the
  // screen would disagree about the one field whose whole job is to be pinned
  // to a moment.
  const [generatedAt, setGeneratedAt] = useState<Date | null>(null);
  // Until the probes land the report would claim "0 running agents" and
  // "unknown OS", which is worse than saying nothing: a wrong report gets
  // pasted just as readily as a right one.
  const loaded = generatedAt !== null;
  const [copied, setCopied] = useState(false);

  useEffect(() => {
    let active = true;
    // Best-effort across the board, and sequential rather than parallel: on
    // macOS the backend snapshot shells out per network service, and these
    // all touch the same subsystems.
    (async () => {
      const snapshot = await fetchDiagnostics().catch((err) => {
        trackError(err, "generic");
        return null;
      });
      const lal = await launchAtLoginStatus().catch(() => null);
      const stale = await routedClientsStale().catch(() => false);
      // One process-table walk for the whole section: the list carries both
      // counts the routing takeover uses, so there is no reason to scan three
      // times over.
      const running = await fetchRunningAgents().catch(() => null);
      if (!active) return;
      setBackend(snapshot);
      setLaunchAtLogin(lal);
      setClientsStale(stale);
      setAgents(running);
      setGeneratedAt(new Date());
    })();
    return () => {
      active = false;
    };
  }, []);

  // Timed reset with a cleanup, the same shape ErrorNote uses: the panel can
  // be dismissed inside the window, and setting state after that unmounts.
  useEffect(() => {
    if (!copied) return;
    const t = setTimeout(() => setCopied(false), 1600);
    return () => clearTimeout(t);
  }, [copied]);

  const report = generatedAt
    ? buildDiagnosticsReport({
        now: generatedAt,
        version,
        platform,
        backend,
        account,
        oauth,
        proxy,
        providers,
        tools,
        launchAtLogin,
        clientsStale,
        agents,
      })
    : "";

  async function copy() {
    try {
      await navigator.clipboard.writeText(report);
      setCopied(true);
    } catch (err) {
      trackError(err, "generic");
    }
  }

  return (
    <div className="flex grow flex-col">
      <SubHeader title="Diagnostics" onBack={onBack} />

      <p className="px-3.5 pb-1 pt-2.5 text-gc-caption leading-snug text-gc-ink-2">
        Everything Gate Connect knows about this machine&rsquo;s setup, ready to
        paste into a support thread. No keys, tokens or passwords are included.
      </p>

      <div className="px-3.5 pt-2">
        <Button variant="accent" full disabled={!loaded} onClick={() => void copy()}>
          <Icon name={copied ? "check" : "copy"} size={15} />
          {copied ? "Copied" : "Copy report"}
        </Button>
      </div>

      {/* The report itself, verbatim. `pre-wrap` rather than a horizontal
          scroller: a value that runs past 360px should wrap where the user can
          read it, not hide off the side of a window this narrow. Mono because
          every line of it is an identifier, a path or a port - exactly what
          DESIGN.md reserves the mono face for. */}
      <div className="mt-3 px-3.5 pb-3">
        <pre
          aria-label="Diagnostics report"
          className="whitespace-pre-wrap break-words rounded bg-gc-subtle p-3 font-mono text-gc-label leading-relaxed text-gc-ink-2 shadow-border"
        >
          {loaded ? report : "Collecting…"}
        </pre>
      </div>
    </div>
  );
}
