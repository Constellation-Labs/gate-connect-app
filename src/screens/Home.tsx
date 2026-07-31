import { useEffect, useState } from "react";
import type { Tool, ProxyDomain } from "../lib/api";
import type { ClassifiedError } from "../lib/errors";
import { launchAtLoginStatus } from "../lib/api";
import { buildGroups, groupSummary } from "../lib/groups";
import { PopHeader } from "../components/gc/PopHeader";
import { Switch, IconButton, SectionLabel, ErrorNote, Button } from "../components/gc/ui";
import { GroupPill } from "../components/GroupPill";
import { Icon } from "../components/gc/Icon";
import { usePlatform } from "../lib/platform";

/** Connected home - the one room: the master Routing card, the certificate
 * step when it blocks coverage, and one row per model family. The families
 * keep the ledger three rows tall however many tools are installed; the
 * config-vs-proxy mechanism lives one tap in, on the group detail. */
export function Home({
  workspace,
  proxyOn,
  caTrusted,
  showProxy,
  tools,
  domains,
  busy,
  error,
  restartHint,
  onDismissRestartHint,
  relaunchHint,
  onDismissRelaunchHint,
  startupRoutingHint,
  onDismissStartupRoutingHint,
  onCloseAgents,
  staleAgentsHint,
  onDismissStaleAgents,
  onToggleProxy,
  onTrustCa,
  onToggleGroup,
  onOpenGroup,
  onOpenSettings,
}: {
  workspace: string;
  proxyOn: boolean;
  caTrusted: boolean;
  showProxy: boolean;
  tools: Tool[];
  domains: ProxyDomain[];
  busy: boolean;
  error?: ClassifiedError | null;
  restartHint: boolean;
  onDismissRestartHint: () => void;
  relaunchHint: boolean;
  onDismissRelaunchHint: () => void;
  startupRoutingHint: boolean;
  onDismissStartupRoutingHint: () => void;
  onCloseAgents: () => void;
  staleAgentsHint: boolean;
  onDismissStaleAgents: () => void;
  onToggleProxy: () => void;
  onTrustCa: () => void;
  onToggleGroup: (id: string, on: boolean) => void;
  onOpenGroup: (id: string) => void;
  onOpenSettings: () => void;
}) {
  const platform = usePlatform();
  const trustStore = platform === "windows" ? "certificate store" : "keychain";
  const groups = buildGroups(tools, domains, { proxyOn, caTrusted });
  // The certificate only gates proxy-routed apps, so the partial state (and
  // the trust card) only exist while at least one app row is switched on.
  const anyDomainOn = domains.some((d) => d.enabled && d.supported);
  const partial = proxyOn && !caTrusted && anyDomainOn;
  // Denominator included so "3 of 8" answers "and what about the rest?"
  // without a scroll; the families below are the itemization.
  const routableCount = groups.reduce((n, g) => n + g.members.length, 0);
  const routedCount = groups.reduce((n, g) => n + g.routed, 0);

  // At most one banner at a time, most actionable first: transient chrome
  // must never bury the ledger (the pills are the point of the screen).
  const banner: "stale" | "startup" | "relaunch" | "restart" | null = staleAgentsHint
    ? "stale"
    : startupRoutingHint
      ? "startup"
      : platform === "linux" && relaunchHint
        ? "relaunch"
        : restartHint
          ? "restart"
          : null;

  // Whether Launch at login is on, so the keep-routing tip only shows when
  // it would actually help (read the state, don't send the user to Settings
  // to check it). null while loading = no tip.
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
      <PopHeader
        workspace={workspace}
        pill={proxyOn ? (partial ? "partial" : "connected") : "idle"}
        onGear={onOpenSettings}
      />
      <div className="flex flex-col gap-2.5 p-3.5">
        {showProxy && (
          <div className="flex items-center gap-3 rounded-[10px] bg-gc-surface p-3.5 shadow-border">
            <div
              className={`flex h-9 w-9 shrink-0 items-center justify-center rounded-[9px] ${
                proxyOn ? "bg-gc-accent-wash text-gc-accent" : "bg-gc-sunken text-gc-ink-4"
              }`}
            >
              <Icon name="shieldCheck" size={19} />
            </div>
            <div className="min-w-0 flex-1">
              <div className="text-[13.5px] font-semibold text-gc-ink">Routing</div>
              <div className="mt-0.5 text-[11.5px] text-gc-ink-3">
                {!proxyOn
                  ? "Off · not routing"
                  : partial
                    ? "On · certificate not trusted yet"
                    : routedCount > 0
                      ? `On · ${routedCount} of ${routableCount} routing`
                      : "On · nothing enabled yet"}
              </div>
              {/* The keep-routing tip only speaks in the quiet room: any
                  warning card or banner outranks a tip. */}
              {proxyOn && launchAtLogin === false && !partial && banner === null && !error && (
                <div className="mt-1 text-[11px] leading-snug text-gc-ink-3">
                  Turn on Launch at login in Settings to keep routing on after
                  a restart.
                </div>
              )}
            </div>
            <Switch
              on={proxyOn}
              label="Route through Gate"
              busy={busy}
              onClick={onToggleProxy}
            />
          </div>
        )}

        {/* Gated on `partial`, not just an untrusted CA: with no app rows
            switched on the certificate blocks nothing, and a warning card
            would contradict the green header pill. */}
        {showProxy && partial && (
          <div className="rounded-[10px] bg-gc-surface p-3.5 shadow-border">
            <div className="flex items-center gap-2.5">
              <div className="flex h-8 w-8 shrink-0 items-center justify-center rounded-[9px] bg-gc-warning-wash text-gc-warning">
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

        {banner === "stale" && (
          <div role="status" className="flex items-center gap-2.5 rounded bg-gc-sunken px-3 py-2.5">
            <Icon name="info" size={15} className="shrink-0 text-gc-error" />
            <div className="min-w-0 flex-1 text-[11.5px] leading-snug text-gc-ink-2">
              Gate&rsquo;s local address changed.{" "}
              <span className="font-semibold">Close your apps</span>; they
              reconnect the next time you open them.
            </div>
            <IconButton
              icon="x"
              size={13}
              onClick={onDismissStaleAgents}
              aria-label="Dismiss restart notice"
            />
          </div>
        )}

        {banner === "startup" && (
          <div role="status" className="flex items-center gap-2.5 rounded bg-gc-highlight px-3 py-2.5 shadow-border">
            <Icon name="info" size={15} className="shrink-0 text-gc-ink" />
            <div className="min-w-0 flex-1 text-[12px] font-medium leading-snug text-gc-ink">
              Routing is on. Anything already open isn&rsquo;t routing yet.
            </div>
            {/* Opens the full takeover (confirm step, close, result); the
                ellipsis signals more steps follow. */}
            <button
              type="button"
              onClick={onCloseAgents}
              className="shrink-0 text-[12px] font-medium text-gc-accent transition hover:text-gc-accent-ink"
            >
              Close them…
            </button>
            <IconButton
              icon="x"
              size={13}
              onClick={onDismissStartupRoutingHint}
              aria-label="Dismiss routing notice"
            />
          </div>
        )}

        {banner === "relaunch" && (
          <div role="status" className="flex items-center gap-2.5 rounded bg-gc-highlight px-3 py-2.5 shadow-border">
            <Icon name="refresh" size={15} className="shrink-0 text-gc-ink" />
            <div className="min-w-0 flex-1 text-[12px] font-medium leading-snug text-gc-ink">
              Anything already open isn&rsquo;t routing yet.{" "}
              <span className="font-semibold">Close your apps</span> and
              they&rsquo;ll pick up Gate when you open them again.
            </div>
            <IconButton
              icon="x"
              size={13}
              onClick={onDismissRelaunchHint}
              aria-label="Dismiss reopen notice"
            />
          </div>
        )}

        {banner === "restart" && (
          <div role="status" className="flex items-center gap-2.5 rounded bg-gc-highlight px-3 py-2.5 shadow-border">
            <Icon name="refresh" size={15} className="shrink-0 text-gc-ink" />
            <div className="min-w-0 flex-1 text-[12px] font-medium leading-snug text-gc-ink">
              <span className="font-semibold">Close your apps</span> to apply
              the change; they pick it up the next time you open them.
            </div>
            <IconButton
              icon="x"
              size={13}
              onClick={onDismissRestartHint}
              aria-label="Dismiss restart hint"
            />
          </div>
        )}

        {error && <ErrorNote error={error} />}
      </div>

      <SectionLabel>Models</SectionLabel>
      {groups.length > 0 ? (
        <div className="flex flex-col border-t border-gc-line">
          {groups.map((group) => {
            const { count, exception } = groupSummary(group);
            return (
              <div
                key={group.id}
                className="relative flex items-center gap-2.5 border-b border-gc-line px-3.5 py-3 transition hover:bg-gc-subtle"
              >
                {/* Stretch button carries the drill-in; the switch is a
                    sibling above it, so one flip routes the whole family and
                    the row body opens the fine grain. */}
                <button
                  type="button"
                  onClick={() => onOpenGroup(group.id)}
                  aria-label={`${group.name} details`}
                  className="absolute inset-0 cursor-pointer focus-visible:outline focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-gc-accent"
                />
                <div className="pointer-events-none relative min-w-0 flex-1">
                  <div className="text-[13.5px] font-medium text-gc-ink">{group.name}</div>
                  <div className="mt-0.5 truncate text-[11px] text-gc-ink-3">
                    {count}
                    {exception && (
                      <>
                        {" · "}
                        <span className="text-gc-ink-2">{exception}</span>
                      </>
                    )}
                  </div>
                </div>
                <span className="pointer-events-none relative">
                  <GroupPill group={group} />
                </span>
                <span className="relative">
                  <Switch
                    on={group.routed > 0}
                    label={`Route ${group.name} through Gate`}
                    busy={busy}
                    onClick={() => onToggleGroup(group.id, group.routed === 0)}
                  />
                </span>
                <span className="pointer-events-none relative">
                  <Icon name="chevronRight" size={14} stroke={2} className="text-gc-ink-4" />
                </span>
              </div>
            );
          })}
        </div>
      ) : (
        <p className="px-3.5 pb-3 text-[11.5px] leading-snug text-gc-ink-3">
          Nothing to route yet. Tools like Claude Code, Codex, and OpenCode
          show up here once installed.
        </p>
      )}
    </div>
  );
}
