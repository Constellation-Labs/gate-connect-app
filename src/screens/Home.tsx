import { useEffect, useState } from "react";
import type { ProviderState, Tool, ProxyDomain } from "../lib/api";
import type { ChangeNotice } from "../App";
import type { ClassifiedError } from "../lib/errors";
import { launchAtLoginStatus } from "../lib/api";
import { buildGroups, groupSummary } from "../lib/groups";
import { PopHeader } from "../components/gc/PopHeader";
import { Switch, IconButton, SectionLabel, ErrorNote, Button } from "../components/gc/ui";
import { GroupPill, groupPillLabel } from "../components/GroupPill";
import { Icon } from "../components/gc/Icon";
import { trustStoreName, usePlatform } from "../lib/platform";

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
   * dismissed. */
  changeNotice: ChangeNotice;
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
  const trustStore = trustStoreName(platform);
  const groups = buildGroups(providers, tools, domains, { proxyOn, caTrusted });
  // The certificate only gates proxy-routed apps, so the partial state (and
  // the trust card) only exist while at least one app row is switched on.
  const anyDomainOn = domains.some((d) => d.enabled && d.supported);
  const partial = proxyOn && !caTrusted && anyDomainOn;
  // Denominator included so "3 of 8" answers "and what about the rest?"
  // without a scroll; the families below are the itemization.
  const routableCount = groups.reduce((n, g) => n + g.members.length, 0);
  const routedCount = groups.reduce((n, g) => n + g.routed, 0);
  // Members no switch on this screen can fix: a family switch deliberately
  // skips a hand-written setup, and an errored tool can't be connected at all.
  // Without naming them the denominator sets a target the controls cannot
  // reach, which reads as the app failing rather than as work to do.
  // Switched on but not flowing because the master is off. Saying so is what
  // makes flipping the switch feel safe rather than speculative.
  const waitingCount = groups.reduce(
    (n, g) => n + g.members.filter((m) => m.attention === "master-off").length,
    0,
  );
  const stuckCount = groups.reduce(
    (n, g) =>
      n + g.members.filter((m) => m.attention === "drifted" || m.attention === "error").length,
    0,
  );

  // At most one banner at a time, most actionable first: transient chrome
  // must never bury the ledger (the pills are the point of the screen). The
  // stale-port notice outranks the change notice because it means the user's
  // tools are broken right now, not merely out of date. The trust card
  // outranks both: it is the reason nothing is routing, so a change notice
  // stacked under it would just be a second thing to read first.
  const banner: "stale" | "change" | null =
    // Nothing installed means nothing can be routing, so an offer to close
    // running apps is an offer to close nothing.
    routableCount === 0
      ? null
      : showProxy && partial
      ? null
      : staleAgentsHint
        ? "stale"
        : changeNotice
          ? "change"
          : null;

  // Whether the certificate explanation is open. Collapsed by default; see the
  // card below for why.
  const [caExplain, setCaExplain] = useState(false);
  // Session-scoped: the tip is already rare (quiet room only), so forgetting
  // the dismissal on relaunch is a smaller cost than another persisted flag.
  const [launchTipDismissed, setLaunchTipDismissed] = useState(false);

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
        pill={proxyOn && routableCount > 0 ? (partial ? "partial" : "connected") : "idle"}
        pillLabel={
          proxyOn && routableCount === 0 ? "Nothing to route" : undefined
        }
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
              {/* A heading, not a styled div: this is the screen's primary
                  control and it was absent from the document outline, so the
                  outline read h1 -> h2 "What routes through Gate" with the
                  master switch unheaded. */}
              <h2 className="text-[13.5px] font-semibold text-gc-ink">Routing</h2>
              <div className="mt-0.5 text-[11.5px] text-gc-ink-3">
                {/* The count survives the certificate state. Dropping it was
                    backwards: that is exactly when the user wants to know how
                    much is still working. */}
                {!proxyOn
                  ? waitingCount > 0
                    ? `Off · ${waitingCount} waiting`
                    : "Off · not routing"
                  : partial
                    ? `On · ${routedCount} of ${routableCount} routing`
                    : routableCount === 0
                      ? "On · nothing installed to route"
                      : routedCount === 0 && stuckCount === 0
                        ? "On · nothing enabled yet"
                        : stuckCount > 0
                          ? `On · ${routedCount} of ${routableCount} routing · ${stuckCount} need${stuckCount === 1 ? "s" : ""} attention`
                          : `On · ${routedCount} of ${routableCount} routing`}
              </div>
            </div>
            <Switch
              on={proxyOn}
              label="Route through Gate"
              busy={busy}
              onClick={onToggleProxy}
            />
          </div>
        )}

        {/* Its own line, not a footnote inside the routing card: that card
            holds the switch that routes traffic, and a link into Settings is
            an unrelated errand. Still only speaks in the quiet room - any
            warning card or banner outranks a tip. */}
        {proxyOn &&
          routableCount > 0 &&
          !launchTipDismissed &&
          launchAtLogin === false &&
          !partial &&
          banner === null &&
          !error && (
          <div className="flex items-start gap-2 text-[11px] leading-snug text-gc-ink-3">
            <span className="min-w-0 flex-1">
              <button
                type="button"
                onClick={onOpenSettings}
                className="font-medium text-gc-ink underline decoration-gc-line-strong underline-offset-2 transition hover:decoration-gc-ink-3"
              >
                Turn on Launch at login
              </button>{" "}
              to keep routing on after a restart.
            </span>
            {/* A tip the user has read and declined should stop asking. It had
                no dismissal at all, so someone who does not want launch-at-login
                saw this under their routing card on every quiet launch. */}
            <IconButton
              icon="x"
              size={12}
              onClick={() => setLaunchTipDismissed(true)}
              aria-label="Dismiss launch at login tip"
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
                {/* Not shieldCheck: the master card's tile directly above is
                    already that glyph, and two shields 40px apart in the same
                    column read as one repeated thing rather than two. */}
                <Icon name="info" size={16} />
              </div>
              <div className="min-w-0 flex-1 text-[12.5px] font-medium leading-snug text-gc-ink">
                Apps with no gateway setting need the Gate certificate.{" "}
                <span className="font-normal text-gc-ink-3">
                  It never leaves this machine.
                </span>{" "}
                {/* Inline, not its own row: the explanation is a footnote to
                    this sentence and a separate line cost 27px in the state
                    that already has the least room. */}
                <button
                  type="button"
                  disabled={busy}
                  onClick={() => setCaExplain((v) => !v)}
                  aria-expanded={caExplain}
                  className="font-normal text-gc-ink-3 underline decoration-gc-line-strong underline-offset-2 transition hover:text-gc-ink disabled:opacity-40"
                >
                  What&rsquo;s this?
                </button>
              </div>
              {/* The action, not the explanation. This card names the reason
                  nothing is routing, and the fix used to be inside a
                  disclosure labelled "What's this?" - so a user who already
                  knows what a root CA is had to open a four-sentence lecture
                  to find the button. */}
              <Button
                variant="accent"
                size="sm"
                className="shrink-0"
                disabled={busy}
                onClick={onTrustCa}
              >
                Trust
              </Button>
            </div>
            {caExplain && (
              <>
                <p className="mt-2 text-[11.5px] leading-snug text-gc-ink-2">
                  Desktop apps with no gateway setting route through
                  Gate&rsquo;s local proxy, which needs a certificate your{" "}
                  {trustStore} trusts. Until it&rsquo;s trusted, those apps
                  don&rsquo;t route.
                </p>
                <p className="mt-1.5 text-[11px] leading-snug text-gc-ink-3">
                  {/* "This machine", not "Certificate": that section stopped
                      existing when Settings collapsed from six headings to
                      four, and this cross-reference survived the rename.
                      Not "anytime" either - removal is offered only while
                      routing is off. */}
                  You can remove it in Settings under This machine whenever
                  routing is off.
                </p>
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
          <div role="status" className="flex items-center gap-2 rounded bg-gc-highlight px-3 py-2 shadow-border">
            <Icon name="info" size={14} className="shrink-0 text-gc-ink" />
            <div className="min-w-0 flex-1 text-[11.5px] font-medium leading-snug text-gc-ink">
              {changeNotice === "pending"
                ? "Set to route, but routing is off, so nothing is going through Gate yet."
                : changeNotice === "started"
                  ? "That turned routing on too. Anything already open isn't routing through Gate yet."
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
          names the question every row answers.
          No longer `dense`: that was tuned when four families put Home exactly
          at the frame, and the harnesses are hidden now, so the ledger is three
          rows with room to spare. */}
      <SectionLabel>What routes through Gate</SectionLabel>
      {groups.length > 0 ? (
        <div role="list" className="flex flex-col border-t border-gc-line">
          {groups.map((group) => {
            const { count, exception } = groupSummary(group);
            return (
              <div
                key={group.id}
                role="listitem"
                // Back to py-3. The 16px this saved bought the fourth family
                // room it no longer needs.
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
                    {/* No count alongside an exception: concatenated, the
                        line truncated to "Codex set up elsewhere · 0 of 2…",
                        and a half-printed number is worse than none. The pill
                        already answers "is this routing?". */}
                    {exception ? <span className="text-gc-ink-2">{exception}</span> : count}
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
              Gate Connect picks up Claude Code and Codex once they&rsquo;re
              installed. Install one, then reopen this window from the menu bar
              and it will show up.
            </p>
          </div>
        </div>
      )}

    </div>
  );
}
