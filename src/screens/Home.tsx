import { useEffect, useState } from "react";
import { openUrl } from "@tauri-apps/plugin-opener";
import type { ProviderState, Tool, ProxyDomain } from "../lib/api";
import type { ClassifiedError } from "../lib/errors";
import { launchAtLoginStatus } from "../lib/api";
import { buildGroups, groupSummary } from "../lib/groups";
import { PopHeader } from "../components/gc/PopHeader";
import { Switch, IconButton, SectionLabel, ErrorNote, Button } from "../components/gc/ui";
import { GroupPill, groupPillLabel } from "../components/GroupPill";
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
  providers,
  tools,
  domains,
  busy,
  error,
  changeNotice,
  onDismissChangeNotice,
  onCloseAgents,
  onEnableRouting,
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
  providers: ProviderState[];
  tools: Tool[];
  domains: ProxyDomain[];
  busy: boolean;
  error?: ClassifiedError | null;
  /** What the last routing change actually resulted in, or null once
   * dismissed. "pending" means something is switched on but the engine is
   * down, so nothing routes. */
  changeNotice: "on" | "off" | "pending" | null;
  onDismissChangeNotice: () => void;
  onCloseAgents: () => void;
  /** Turn the master on from the pending banner: the remedy belongs on the
   * notice that reports the problem. */
  onEnableRouting: () => void;
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
  const groups = buildGroups(providers, tools, domains, { proxyOn, caTrusted });
  // The certificate only gates proxy-routed apps, so the partial state (and
  // the trust card) only exist while at least one app row is switched on.
  const anyDomainOn = domains.some((d) => d.enabled && d.supported);
  const partial = proxyOn && !caTrusted && anyDomainOn;
  // Denominator included so "3 of 8" answers "and what about the rest?"
  // without a scroll; the families below are the itemization.
  const routableCount = groups.reduce((n, g) => n + g.members.length, 0);
  const routedCount = groups.reduce((n, g) => n + g.routed, 0);

  // At most one banner at a time, most actionable first: transient chrome
  // must never bury the ledger (the pills are the point of the screen). The
  // stale-port notice outranks the change notice because it means the user's
  // tools are broken right now, not merely out of date. The trust card
  // outranks both: it is the reason nothing is routing, so a change notice
  // stacked under it would just be a second thing to read first.
  const banner: "stale" | "change" | null =
    showProxy && partial
      ? null
      : staleAgentsHint
        ? "stale"
        : changeNotice
          ? "change"
          : null;

  // Whether the certificate explanation is open. Collapsed by default; see the
  // card below for why.
  const [caExplain, setCaExplain] = useState(false);

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
      {/* Flipping any switch rewrites the header pill, the master sub-line and
          every row description at once, all of it silently. One polite live
          region carries the headline so the change is announced without
          reading the whole screen back. */}
      <span aria-live="polite" className="sr-only">
        {proxyOn
          ? partial
            ? "Routing on, certificate not trusted"
            : `Routing on, ${routedCount} of ${routableCount} routing`
          : "Routing off"}
      </span>
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
                {/* The count survives the certificate state. Dropping it was
                    backwards: that is exactly when the user wants to know how
                    much is still working. */}
                {!proxyOn
                  ? "Off · not routing"
                  : partial
                    ? `On · ${routedCount} of ${routableCount} routing · certificate not trusted`
                    : routedCount > 0
                      ? `On · ${routedCount} of ${routableCount} routing`
                      : routableCount === 0
                        ? "On · nothing installed to route"
                        : "On · nothing enabled yet"}
              </div>
              {/* The keep-routing tip only speaks in the quiet room: any
                  warning card or banner outranks a tip. */}
              {proxyOn && launchAtLogin === false && !partial && banner === null && !error && (
                <div className="mt-1 text-[11px] leading-snug text-gc-ink-3">
                  {/* It named a control and left the user to find it. */}
                  <button
                    type="button"
                    onClick={onOpenSettings}
                    className="relative z-[1] font-medium text-gc-ink underline decoration-gc-line-strong underline-offset-2 transition hover:decoration-gc-ink-3"
                  >
                    Turn on Launch at login
                  </button>{" "}
                  to keep routing on after a restart.
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
        {/* Compact by default. This state used to open with a 60-word card and
            a full-width button, which pushed every ledger row off-screen in
            the one state where the pills matter most: the untrusted CA is the
            usual answer to "why isn't my tool routing?", and the answer sat
            below a fold in a window with no visible scrollbar. The full
            explanation is one tap away, before consent, because a root CA
            deserves it - but it no longer taxes the user who already knows. */}
        {showProxy && partial && (
          <div className="rounded-[10px] bg-gc-surface p-3.5 shadow-border">
            <div className="flex items-center gap-2.5">
              <div className="flex h-8 w-8 shrink-0 items-center justify-center rounded-[9px] bg-gc-warning-wash text-gc-warning">
                <Icon name="shieldCheck" size={16} />
              </div>
              <div className="min-w-0 flex-1 text-[12.5px] font-medium leading-snug text-gc-ink">
                Apps with no gateway setting need the Gate certificate.
              </div>
              <button
                type="button"
                disabled={busy}
                onClick={() => setCaExplain((v) => !v)}
                aria-expanded={caExplain}
                className="shrink-0 text-[12px] font-medium text-gc-accent transition hover:text-gc-accent-ink disabled:opacity-40"
              >
                What&rsquo;s this?
              </button>
            </div>
            {caExplain && (
              <>
                <p className="mt-2.5 text-[11.5px] leading-snug text-gc-ink-2">
                  Desktop apps with no gateway setting route through
                  Gate&rsquo;s local proxy, which needs a certificate your{" "}
                  {trustStore} trusts. The certificate and its private key are
                  created on this machine and never leave it. Until it&rsquo;s
                  trusted, those apps don&rsquo;t route.
                </p>
                <p className="mt-1.5 text-[11px] leading-snug text-gc-ink-3">
                  You can remove it anytime in Settings under Certificate.
                </p>
                <Button variant="accent" full className="mt-2.5" disabled={busy} onClick={onTrustCa}>
                  Trust certificate
                </Button>
              </>
            )}
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

        {/* One notice for every routing change, worded by direction. Both
            directions carry the same remedy - close what's already open - so
            both offer the same action, rather than the close route existing
            only for the notice raised at startup. */}
        {banner === "change" && (
          <div role="status" className="flex items-center gap-2.5 rounded bg-gc-highlight px-3 py-2.5 shadow-border">
            <Icon name="info" size={15} className="shrink-0 text-gc-ink" />
            <div className="min-w-0 flex-1 text-[12px] font-medium leading-snug text-gc-ink">
              {changeNotice === "pending"
                ? "Set to route, but routing is off, so nothing is going through Gate yet."
                : changeNotice === "on"
                  ? "Routing is on. Anything already open isn't routing through Gate yet."
                  : "Routing is off. Anything already open still points at Gate."}
            </div>
            {/* Pending has a different remedy: there is nothing running to
                close, so offering "Close them…" would be busywork. The
                ellipsis on the close action signals more steps follow. */}
            {changeNotice === "pending" ? (
              <button
                type="button"
                onClick={onEnableRouting}
                className="shrink-0 text-[12px] font-medium text-gc-accent transition hover:text-gc-accent-ink"
              >
                Turn on routing
              </button>
            ) : (
              <button
                type="button"
                onClick={onCloseAgents}
                className="shrink-0 text-[12px] font-medium text-gc-accent transition hover:text-gc-accent-ink"
              >
                Close them…
              </button>
            )}
            <IconButton
              icon="x"
              size={13}
              onClick={onDismissChangeNotice}
              aria-label="Dismiss routing notice"
            />
          </div>
        )}

        {error && <ErrorNote error={error} />}
      </div>

      {/* Not "Models": the last row is a tool category, not a model family,
          and a label the list contradicts is worse than a plain one. This
          names the question every row answers. */}
      <SectionLabel>What routes through Gate</SectionLabel>
      {groups.length > 0 ? (
        <div role="list" className="flex flex-col border-t border-gc-line">
          {groups.map((group) => {
            const { count, exception } = groupSummary(group);
            return (
              <div
                key={group.id}
                role="listitem"
                className="relative flex items-center gap-2.5 border-b border-gc-line px-3.5 py-3 transition hover:bg-gc-subtle"
              >
                {/* Stretch button carries the drill-in; the switch is a
                    sibling above it, so one flip routes the whole family and
                    the row body opens the fine grain.

                    `aria-describedby` is what makes the row readable without
                    eyes. The count, the pill and the exception are all in
                    `pointer-events-none` spans so the stretch button can sit
                    over them, which also hid them from the accessibility tree:
                    the row announced "Claude details, button" and never
                    "OpenClaw failed", which is the one thing the row exists to
                    say. */}
                <button
                  type="button"
                  onClick={() => onOpenGroup(group.id)}
                  aria-label={`${group.name} details`}
                  aria-describedby={`group-desc-${group.id}`}
                  className="absolute inset-0 cursor-pointer focus-visible:outline focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-gc-accent"
                />
                <div className="pointer-events-none relative min-w-0 flex-1">
                  <div className="text-[13.5px] font-medium text-gc-ink">{group.name}</div>
                  {/* Exception first. Concatenated as `count · exception` the
                      line truncated at 360px and the actionable half was what
                      got cut ("0 of 2 routing · Codex set up els…"). The pill
                      already answers "is this routing?", so the count is the
                      half that can afford to go. */}
                  <div className="mt-0.5 truncate text-[11px] text-gc-ink-3">
                    {exception ? (
                      <>
                        <span className="text-gc-ink-2">{exception}</span>
                        {" · "}
                        {count}
                      </>
                    ) : (
                      count
                    )}
                  </div>
                </div>
                {/* The visible text truncates at 360px; this carries the whole
                    sentence, pill state included, to anyone listening. */}
                <span id={`group-desc-${group.id}`} className="sr-only">
                  {groupPillLabel(group)}. {count}
                  {exception ? `. ${exception}` : ""}
                </span>
                <span className="pointer-events-none relative">
                  <GroupPill group={group} />
                </span>
                <span className="relative">
                  <Switch
                    on={group.desired > 0}
                    label={group.switchLabel}
                    busy={busy}
                    onClick={() => onToggleGroup(group.id, group.desired === 0)}
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
        <div className="mx-3.5 mb-1 flex items-start gap-2.5 rounded-[10px] bg-gc-surface p-3.5 shadow-border">
          <div className="flex h-8 w-8 shrink-0 items-center justify-center rounded-[9px] bg-gc-sunken text-gc-ink-4">
            <Icon name="search" size={16} />
          </div>
          <div className="min-w-0 flex-1">
            <div className="text-[12.5px] font-medium text-gc-ink">
              Nothing to route yet
            </div>
            <p className="mt-1 text-[11.5px] leading-snug text-gc-ink-3">
              Gate Connect picks up Claude Code, Codex, OpenCode and friends
              once they&rsquo;re installed. Install one and reopen this window.
            </p>
          </div>
        </div>
      )}

      {/* The dashboard is where the traffic this screen routes actually ends
          up - keys, usage, org. That belongs one click from the ledger, not
          behind the gear. Quiet weight: it leaves the app, so it must not
          compete with the switches. */}
      <div className="px-3.5 pt-3">
        <button
          type="button"
          onClick={() => {
            void openUrl("https://app.constellationgate.ai");
          }}
          className="inline-flex items-center gap-1.5 text-[12.5px] font-medium text-gc-ink-3 transition hover:text-gc-ink"
        >
          <Icon name="cube" size={14} />
          Open Gate dashboard
        </button>
      </div>
    </div>
  );
}
