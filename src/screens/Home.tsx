import { useEffect, useState } from "react";
import type { ProviderState, Tool, ProxyDomain } from "../lib/api";
import type { ChangeNotice } from "../App";
import type { ClassifiedError } from "../lib/errors";
import { launchAtLoginStatus } from "../lib/api";
import type { Group, GroupException } from "../lib/groups";
import { buildGroups, groupSummary } from "../lib/groups";
import { PopHeader } from "../components/gc/PopHeader";
import { Switch, IconButton, ErrorNote, Button } from "../components/gc/ui";
import { GroupPill, groupPillLabel } from "../components/GroupPill";
import { Icon } from "../components/gc/Icon";
import { trustStoreName, usePlatform } from "../lib/platform";
import { openExternal } from "../lib/openExternal";
import { GATE_DASHBOARD_URL } from "../lib/config";

/** Connected home - the one room: the master Routing card, the certificate
 * step when it blocks coverage, and one row per model family, ranked so
 * anything needing a human is the first thing on the screen.
 *
 * The rows carry name and pill only. Their switches, their members and the
 * config-vs-proxy mechanism live one tap in, on the ledger panel; so does the
 * shell-environment channel, which routes every family at once rather than
 * belonging to any one of them. What stays here is the answer to "is anything
 * wrong, and which of my tools", which is the question the popover gets
 * opened with. */
export function Home({
  workspace,
  gatewayHost,
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
  onOpenFamily,
  onOpenSettings,
  envExportSeparable,
  envExportOn,
  onToggleEnvExport,
}: {
  workspace: string;
  /** The gateway host on its own, separate from `workspace`: the header now
   * carries the org, so the identifier traffic actually leaves through needs a
   * line of its own rather than disappearing with it. */
  gatewayHost: string;
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
  /** Opens `groupId`'s own panel. The id is required: every row leads to its
   * own family, so there is no longer a target-less way in. It used to be
   * optional because the ledger heading was a button that opened a list of all
   * four families collapsed; that list is gone, and with it the only caller
   * that had nothing to name. */
  onOpenFamily: (groupId: string) => void;
  onOpenSettings: () => void;
  /** Whether the shell-environment channel can be offered at all. False on
   * Linux, where those variables *are* the system proxy, so a switch could not
   * honour itself. */
  envExportSeparable: boolean;
  envExportOn: boolean;
  onToggleEnvExport: () => void;
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
  // What the user has asked to route, as opposed to what is actually flowing.
  // The sub-line needs this to tell "you haven't switched anything on yet" apart
  // from "you switched things on and the certificate is holding them". Keyed off
  // `routed === 0` those two states printed the same sentence, and the second
  // one is the one where "nothing enabled yet" is simply false.
  const desiredCount = groups.reduce((n, g) => n + g.desired, 0);
  // Errors only: a hand-written setup elsewhere is a choice the user made and
  // the family switch deliberately respects, so it must not turn the header
  // amber. A failure is not a choice. This is the count that decides whether the
  // header may still claim health.
  const errorCount = groups.reduce(
    (n, g) => n + g.members.filter((m) => m.attention === "error").length,
    0,
  );

  // The order the families render in: anything needing a human floats to the
  // top, everything else holds catalog order. `sort` is stable, so the healthy
  // tail never reshuffles between renders.
  //
  // `master-off` is deliberately absent, as it was when this ranking fed the
  // door. It is not per-family news, it is the master switch's own state, and
  // the card above already says "Off · N waiting". Ranked here it would print
  // "waiting on routing" on every row at once - the same sentence the card just
  // said, repeated four times, while naming nothing.
  const EXCEPTION_RANK: Record<string, number> = {
    error: 0,
    "needs-trust": 1,
    drifted: 2,
  };
  const ranked = groups
    .map((group) => ({ group, ...groupSummary(group) }))
    .sort(
      (a, b) =>
        (a.kind !== null ? (EXCEPTION_RANK[a.kind] ?? 9) : 9) -
        (b.kind !== null ? (EXCEPTION_RANK[b.kind] ?? 9) : 9),
    );

  // At most one banner at a time, most actionable first. The stale-port notice
  // outranks the change notice because it means the user's tools are broken
  // right now, not merely out of date. The trust card outranks both: it is the
  // reason nothing is routing, so a change notice stacked under it would just
  // be a second thing to read first.
  //
  // Both render above the ledger, with the trust card. This comment used to say
  // "transient chrome must never bury the ledger (the pills are the point of
  // the screen)", which contradicted DESIGN.md's "Blockers outrank inventory"
  // written the same round, and the banners obeyed this half while the trust
  // card obeyed the other. A banner that explains why traffic is not flowing
  // and carries the fix is not chrome; it is the most important sentence on the
  // screen. The rule survives, scoped to what is actually transient: the launch
  // tip, which is the one element here that speaks only in a quiet room and
  // correctly sits below the list.
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
  // Whether the user has flipped anything on this screen yet. Gates the live
  // region so it reports their changes, not the backend's.
  const [interacted, setInteracted] = useState(false);

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
        // Green is the only thing a mid-task user reads, so it has to mean
        // traffic is flowing. Routing can be on with every row still off,
        // and the pill used to go green over three grey "Not routed" rows -
        // the one place the app claimed something it could not back up.
        // `partial` still outranks the count: "certificate not trusted" is a
        // reason, and demoting it to a bare "Nothing routing" would hide it.
        // A failed tool demotes it too. The pill flew green over an errored
        // family whose own pill read grey "Not routed", so the screen the user
        // opened *because* a tool stopped working answered in the colours it
        // uses for a healthy, deliberately-off setup. The words below are
        // unchanged; what changes is that they stop arriving in green.
        pill={
          proxyOn
            ? partial
              ? "partial"
              : errorCount > 0
                ? "partial"
                : routedCount > 0
                  ? "connected"
                  : "idle"
            : "idle"
        }
        pillLabel={
          !proxyOn || partial
            ? undefined
            : routableCount === 0
              ? "Nothing to route"
              : routedCount === 0
                ? "Nothing routing"
                : errorCount > 0
                  ? // The amber default is "Needs trust", which is a different
                    // and specific reason. Traffic is flowing here and
                    // something failed, so say the honest half-state.
                    "Partly routed"
                  : undefined
        }
        onGear={onOpenSettings}
      />
      {/* Flipping any switch rewrites the header pill, the master sub-line and
          every row description at once, all of it silently. One polite live
          region carries the headline so the change is announced without
          reading the whole screen back. */}
      {/* Silent until the user has actually touched something here. The
          region used to fire on every state change including ones nobody
          asked for, so a screen-reader user got an unprompted "Routing on,
          2 of 5 routing" when the backend enabled itself at startup. */}
      <span aria-live="polite" className="sr-only">
        {interacted
          ? proxyOn
            ? partial
              ? "Routing on, certificate not trusted"
              : `Routing on, ${routedCount} of ${routableCount} routing`
            : "Routing off"
          : ""}
      </span>
      <div className="flex flex-col gap-2.5 p-3.5">
        {/* One box, two parts: the master control and the door to what it
            controls. They were two cards with two 36px tiles stacked 10px apart,
            which read as two unrelated errands when they are the same subject
            seen at two grains.

            No rule between the halves. There was one, inset rather than
            full-bleed, on the reasoning that a hairline running edge to edge
            inside a card still reads as a card boundary - which is the whole
            thing this merge was meant to stop. Dropping it entirely turned out
            better: the host line reads as the card's third line, which is what
            it is. `overflow-hidden` keeps the lower half's hover fill inside the
            radius. */}
        {(showProxy || gatewayHost) && (
          <div className="overflow-hidden rounded-[10px] bg-gc-surface shadow-border">
            {showProxy && (
              // `flex-wrap` with an `em` basis on the text column, same rule as
              // the family rows. At 200% the tile and the switch left the text
              // ~150px, which broke "On · 8 of 8 routing" across three lines with
              // an orphaned "8". Given its own line the switch costs one row and
              // the sentence reads.
              <div className="flex flex-wrap items-center gap-x-3 gap-y-2 px-3.5 pb-2.5 pt-3.5">
                {showProxy && (
                  <div
                    className={`flex h-9 w-9 shrink-0 items-center justify-center rounded-[9px] ${
                      proxyOn ? "bg-gc-accent-wash text-gc-accent" : "bg-gc-sunken text-gc-ink-4"
                    }`}
                  >
                    <Icon name="shieldCheck" size={19} />
                  </div>
                )}
                <div className="min-w-0 flex-1 basis-[8em]">
                  {showProxy && (
                    <>
                      {/* A heading, not a styled div: this is the screen's
                          primary control and it was absent from the document
                          outline, so the outline read h1 -> h2 "What routes
                          through Gate" with the master switch unheaded. */}
                      <h2 className="text-gc-body font-semibold text-gc-ink">Routing</h2>
                      {/* Identified, because it is the screen's one report of
                          whether anything is flowing: the shell-channel switch
                          below points at it for the reality half of its own
                          state, the way a family switch used to point at its
                          row's sentence. */}
                      <div id="routing-status" className="mt-0.5 text-gc-caption text-gc-ink-3">
                        {/* The count survives the certificate state. Dropping it
                            was backwards: that is exactly when the user wants to
                            know how much is still working.

                            No "· N need attention" clause any more, and no
                            "nothing installed to route". The rows below now name
                            each exception and the empty card explains the empty
                            case, so both of those were this line restating what
                            the screen already said better one block down - the
                            mid-task user was reading one fault in three
                            vocabularies (pill, this line, the door) before
                            finding out which tool it was. */}
                        {!proxyOn
                          ? waitingCount > 0
                            ? `Off · ${waitingCount} waiting`
                            : "Off · not routing"
                          : routableCount === 0
                            ? "On"
                            : desiredCount === 0
                              ? "On · nothing enabled yet"
                              : `On · ${routedCount} of ${routableCount} routing`}
                      </div>
                    </>
                  )}
                </div>
                {showProxy && (
                  <Switch
                    className="ml-auto"
                    on={proxyOn}
                    label="Route through Gate"
                    busy={busy}
                    onClick={() => {
                      setInteracted(true);
                      onToggleProxy();
                    }}
                  />
                )}
              </div>
            )}

            {/* Its own full-width line inside the card, not a third line in the
                text column: there it shared 234px with the 36px tile and the
                switch and a production host was already close to wrapping. Here
                it gets the card's whole 332px, which fits a 52-character host on
                one line, so `truncate` costs nothing real and never wraps.
                `title` keeps a staging host recoverable. */}
            {gatewayHost && (
              // Wraps rather than truncates. At 100% the card's 332px fits a
              // 52-character host on one line, so this renders identically to
              // the `truncate` it replaces; at 200% the host wanted 409px in
              // 304 and lost its tail to an ellipsis. A hostname is the one
              // identifier on this screen that cannot be shortened without
              // lying, so it gets a second line instead of an ellipsis.
              <div
                title={gatewayHost}
                className={`px-3.5 pb-2.5 font-mono text-gc-label text-gc-ink-3 [overflow-wrap:anywhere]${
                  showProxy ? "" : " pt-3.5"
                }`}
              >
                {gatewayHost}
              </div>
            )}

          </div>
        )}

        {/* The ledger, back on Home.

            It spent one release behind a door on the theory that one room
            cannot hold a routing card, a certificate ceremony, a wire line, a
            banner, a launch tip and an itemized list. Measured, the room was
            33% empty with nothing scrolling, and the door had replaced the
            inventory with a single ranked exception - so a mid-task user could
            open the popover, read "Claude Code failed", and still not know
            which of four families to go to. PRODUCT.md's second principle puts
            the list here for exactly that reason.

            Its own surface rather than a third part of the routing card: the
            card is one control and its address, and a list of four things it
            governs is a different grain. Rows carry name and pill only - the
            switches stayed on the panel, which is what keeps this from
            re-crowding into the screen the door was invented to fix. */}
        {/* Gated on `partial`, not just an untrusted CA: with no app rows
            switched on the certificate blocks nothing, and a warning card
            would contradict the green header pill. */}
        {showProxy && partial && (
          <div className="rounded-[10px] bg-gc-surface p-3.5 shadow-border">
            <div className="flex items-center gap-2.5">
              <div className="order-1 flex h-9 w-9 shrink-0 items-center justify-center rounded-[9px] bg-gc-warning-wash text-gc-warning-deep">
                {/* Not shieldCheck: the master card's tile directly above is
                    already that glyph, and two shields 40px apart in the same
                    column read as one repeated thing rather than two.

                    warning-deep, not warning: `#f39c12` on the wash measures
                    2.00:1, the only graphical object in the app under 3:1, on
                    the one card whose whole job is to get a blocker cleared.
                    The ring ladder already uses warning-deep for this rung. */}
                <Icon name="info" size={16} />
              </div>
              {/* The action, not the explanation. This card names the reason
                  nothing is routing, and the fix used to be inside a
                  disclosure labelled "What's this?" - so a user who already
                  knows what a root CA is had to open a four-sentence lecture
                  to find the button.

                  Before the text in the DOM, after it visually (`order-2` /
                  `order-3`). The sentence wraps, which put "What's this?" at
                  y=244 while Trust sits at y=211 - so tab reached the explainer
                  33.2px *below* the action, on the one screen whose whole job is
                  a ceremony. Ordering the DOM by what the eye meets first also
                  means a screen reader hears the remedy before the lecture. */}
              <Button
                variant="accent"
                size="sm"
                className="order-3 shrink-0"
                disabled={busy}
                onClick={() => {
                  setInteracted(true);
                  onTrustCa();
                }}
              >
                Trust
              </Button>
              <div className="order-2 min-w-0 flex-1 text-gc-body-sm font-medium leading-snug text-gc-ink">
                Apps with no gateway setting need the Gate certificate.{" "}
                <span className="font-normal text-gc-ink-3">
                  It never leaves this machine.
                </span>{" "}
                {/* Inline, not its own row: the explanation is a footnote to
                    this sentence and a separate line cost 27px in the state
                    that already has the least room. `-my-1.5 inline-block
                    py-1.5` takes the hit area from 17px to 29px without moving
                    the sentence it sits in. */}
                <button
                  type="button"
                  disabled={busy}
                  onClick={() => setCaExplain((v) => !v)}
                  aria-expanded={caExplain}
                  className="-my-1.5 inline-block py-1.5 font-normal text-gc-ink-3 underline decoration-gc-line-strong underline-offset-2 transition hover:text-gc-ink disabled:opacity-45"
                >
                  What&rsquo;s this?
                </button>
              </div>
            </div>
            {caExplain && (
              <>
                <p className="mt-2 text-gc-caption leading-snug text-gc-ink-2">
                  Desktop apps with no gateway setting route through
                  Gate&rsquo;s local proxy, which needs a certificate your{" "}
                  {trustStore} trusts. Until it&rsquo;s trusted, those apps
                  don&rsquo;t route.
                </p>
                <p className="mt-1.5 text-gc-micro leading-snug text-gc-ink-3">
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

        {/* Blockers, above the inventory.

            These three sat *below* the ledger card, 10px under the last family
            row. Measured, that put the sentence "Gate's local address changed.
            Close your apps" at y=393 beneath four rows all reading green
            "Routed" - because `routed` is `enabled && proxyOn && caTrusted` and
            none of those goes false when the local port moves. A mid-task user
            opening the popover for two seconds read four green pills and closed
            it. PRODUCT.md names silent disconnects as the failure signal, and
            this was the screen for that failure reading reassuring.

            So they join the certificate card in the one slot a blocker belongs
            in: directly under the routing card, above the list. The certificate
            card already obeyed that rule; these did not, and the two orderings
            came from two rules written in the same round.

            `error` leads: it is the operation the user just triggered failing,
            so it is the newest news on the screen. */}
        {error && <ErrorNote error={error} />}

        {banner === "stale" && (
          <div role="status" className="flex items-center gap-2.5 rounded bg-gc-sunken px-3 py-2.5">
            <Icon name="info" size={15} className="shrink-0 text-gc-error" />
            <div className="min-w-0 flex-1 text-gc-caption leading-snug text-gc-ink-2">
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
            <div className="min-w-0 flex-1 text-gc-caption font-medium leading-snug text-gc-ink">
              {changeNotice === "pending"
                ? "Set to route, but routing is off, so nothing is going through Gate yet."
                : changeNotice === "started"
                  ? "That turned routing on too. Anything already open isn’t routing through Gate yet."
                  : changeNotice === "on"
                    ? "Routing is on. Anything already open isn’t routing through Gate yet."
                    : "Routing is off. Anything already open still points at Gate."}
            </div>
            {/* Pending has a different remedy: there is nothing running to
                close, so offering "Close them…" would be busywork. The
                ellipsis on the close action signals more steps follow.

                `-my-1.5 py-1.5`: the remedy in a banner about broken routing
                measured 18px tall, under the 24px target minimum, and the
                negative margin buys the height without moving the row. */}
            {changeNotice === "pending" ? (
              <button
                type="button"
                onClick={onEnableRouting}
                className="-my-1.5 shrink-0 py-1.5 text-gc-caption-lg font-medium text-gc-accent transition hover:text-gc-accent-ink"
              >
                Turn on routing
              </button>
            ) : (
              <button
                type="button"
                onClick={onCloseAgents}
                className="-my-1.5 shrink-0 py-1.5 text-gc-caption-lg font-medium text-gc-accent transition hover:text-gc-accent-ink"
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

        {groups.length > 0 && (
          <div>
            {/* Sentence case in sans, not the mono uppercase SectionLabel that
                Settings uses. This is a sentence, not an identifier, and
                DESIGN.md's own mono rule ("never sentence copy") is what
                decides it. The words are the ones the door carried and the
                panel still titles itself with; retiring the button did not
                mean retiring the best sentence on the screen.

                An h2, so the only route off Home stops being absent from the
                document outline - it was a <button> wrapping <div>s, and the
                outline read h1 -> h2 "Routing" with nothing for the list. */}
            <h2 className="pb-1.5 text-gc-caption font-medium text-gc-ink-3">
              What routes through Gate
            </h2>
            <div
              role="list"
              className="overflow-hidden rounded-[10px] bg-gc-surface shadow-border"
            >
              {ranked.map(({ group, count, exception, kind }, i) => (
                <FamilyRow
                  key={group.id}
                  group={group}
                  count={count}
                  // Two states never reach a row, because the card above owns
                  // both and can act on both: the master being off, and the
                  // certificate. Printed here they would repeat one sentence on
                  // up to four rows directly under the card that just said it,
                  // and in the certificate's case alongside the only button that
                  // fixes it. The pill still reports what it costs the family.
                  exception={kind === "master-off" || kind === "needs-trust" ? null : exception}
                  kind={kind}
                  last={i === ranked.length - 1}
                  onOpen={() => onOpenFamily(group.id)}
                />
              ))}
            </div>
          </div>
        )}

        {/* The other channel, back on Home now that the panel it lived on is
            about one family and this belongs to none of them: it routes every
            command-line tool at once, whatever provider they talk to.

            Home is where it started and where it failed once, so the placement
            is the whole design. The recorded failure was geometric: it sat 66px
            under the master switch wearing the same 38x22 track in the same
            indigo, telling the user by proximity that a machine-wide change to
            git and curl was routing's equal. Two things keep that from
            recurring. It sits *below* the ledger, so four family rows put
            ~190px between the two switches and they are no longer a pair. And
            it is a line, not a card - no `shadow-border`, matching the launch
            tip below it - so the master switch keeps card weight and this reads
            as a preference in the margin.

            Only with families on screen. The empty state has no ledger to put
            between the two switches, which is exactly the adjacency that failed,
            and it costs nothing: the old panel was reachable only through a
            family row, so a machine with nothing installed never had a route to
            this control either.

            `ink-3`, not `ink-4`. This is two sentences of instruction about the
            one control in the app that changes something outside the AI tools -
            `HTTPS_PROXY` reaches git, curl and every process in the shell - and
            at ink-4 it measured 3.97:1 on white, the only text in the product
            below the 4.5:1 floor. ink-3 is 6.90:1.

            Absent entirely on Linux, where the `environment.d` drop-in *is* the
            system proxy: there the variables cannot be declined without turning
            routing off, and a switch that cannot honour itself is worse than no
            switch. `env_export_separable` carries that from the backend rather
            than the UI guessing at platforms. */}
        {envExportSeparable && groups.length > 0 && (
          <div className="flex flex-wrap items-center gap-x-3 gap-y-1.5">
            <div className="min-w-0 flex-1 basis-[8em]">
              <div className="text-gc-body-sm font-medium text-gc-ink">Command-line tools</div>
              <div className="mt-0.5 text-gc-micro leading-snug text-gc-ink-3">
                Sets <span className="font-mono">HTTPS_PROXY</span> for your whole
                shell, so OpenCode and other terminal tools route too.
              </div>
              {/* No "Waiting on routing" line here, unlike the panel this came
                  from. That panel had no master card, so the sentence had
                  nowhere else to live; Home's card sits 190px up reporting
                  "Off · N waiting", and DESIGN.md's own rule is that
                  card-owned states never reprint further down - it is why the
                  ledger rows below suppress `master-off` too. Printing it here
                  would be the third copy of one fact on one screen.

                  The switch still has to answer for reading "on" over a channel
                  that cannot be live, which is what Intent versus Reality
                  requires. It points at the card's status line: the reality
                  sentence for this condition on this screen, in the vocabulary
                  the screen already uses, rather than a second phrasing of it
                  20px away. */}
            </div>
            <Switch
              className="ml-auto"
              on={envExportOn}
              label="Route command-line tools through Gate"
              describedBy={envExportOn && !proxyOn && showProxy ? "routing-status" : undefined}
              busy={busy}
              onClick={onToggleEnvExport}
            />
          </div>
        )}

        {groups.length === 0 && (
          // A door into an empty room is worse than the explanation, so the
          // empty case keeps the card that says what to install.
          <div className="flex items-start gap-2.5 rounded-[10px] bg-gc-surface p-3.5 shadow-border">
            <div className="flex h-9 w-9 shrink-0 items-center justify-center rounded-[9px] bg-gc-sunken text-gc-ink-4">
              <Icon name="search" size={16} />
            </div>
            <div className="min-w-0 flex-1">
              {/* Leads with the verb, not with a third restatement. The header
                  pill already says "Nothing to route" and the card above says
                  "On"; this card used to open "Nothing to route yet", so the
                  empty state said one fact three times and never said what to
                  do about it. */}
              <div className="text-gc-body-sm font-medium text-gc-ink">
                Install a tool to route
              </div>
              <p className="mt-1 text-gc-caption leading-snug text-gc-ink-3">
                Gate Connect picks up Claude Code and Codex once they&rsquo;re
                installed. Install one, then reopen this window from the menu bar
                and it will show up.
              </p>
            </div>
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
          <div className="flex items-start gap-2 text-gc-micro leading-snug text-gc-ink-3">
            <span className="min-w-0 flex-1">
              {/* `inline-block` with matched negative margin: the hit area grows
                  from 15px to 27px without moving the sentence it sits in. The
                  action to accept this tip was half the height of the
                  IconButton that declines it. */}
              <button
                type="button"
                onClick={onOpenSettings}
                className="-my-1.5 inline-block py-1.5 font-medium text-gc-ink underline decoration-gc-line-strong underline-offset-2 transition hover:decoration-gc-ink-3"
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

        {/* Last in this zone, under whatever the app has to say. It is the one
            control here that leaves Gate Connect, so a certificate blocker, a
            stale-port warning or a failed toggle all outrank it; it used to sit
            above every one of them.

            Fifth address for this link. The previous four each failed: below the
            last ledger row it fell under the fold; pinned to the footer it
            squeezed the credential promise until "Session in Credential Manager"
            truncated on Windows; riding the ledger heading it outweighed the
            heading it sat on; and sharing the host's line it took width from the
            one identifier on this screen that cannot be shortened without lying.

            14.5px, not 11.5px: at the bottom of the zone it is no longer a
            footnote to the host line, and it was the smallest interactive text
            in the app. 14.5 rather than 15 only because 15 is off the ramp in
            DESIGN.md and this is the nearest step to it; the two are half a
            pixel apart. The padding takes the hit area from 19px to 32px, which
            also clears the 24px target minimum it used to miss. */}
        <button
          type="button"
          onClick={() => {
            void openExternal(GATE_DASHBOARD_URL);
          }}
          className="-ml-1.5 flex w-fit items-center gap-2 rounded px-1.5 py-1.5 text-gc-title font-medium text-gc-accent transition hover:bg-gc-accent-wash hover:text-gc-accent-ink"
        >
          <Icon name="cube" size={15} />
          Gate dashboard
        </button>
      </div>

    </div>
  );
}

/** One family on Home: the name, the pill that answers "is this routing?", and
 * the way in. No switch and no expander - those are the panel's job, and
 * keeping them off this row is what lets four families fit under a routing card
 * without a scroll.
 *
 * The row navigates; it does not expand. Home's glyph has meant "go" since the
 * door used it, the panel's caret has meant "open in place", and the member
 * level says the word "Details" - three depths, three affordances, no glyph
 * doing two jobs.
 *
 * No inline remedy, which was the plan and did not survive contact:
 *
 * - `needs-trust` co-occurs with the certificate card by construction (a member
 *   is only untrusted while `partial` is true), so a per-row Trust button would
 *   put two or three identical buttons on screen for one machine-wide
 *   certificate, next to a card that already explains it and says "It never
 *   leaves this machine". One ceremony, one button.
 * - `error` and `drifted` have no honest family-level action at all:
 *   `setGroupRouted` skips a drifted member by design, and an errored one is
 *   already `desired`, so the loop that looked like a retry would touch nothing
 *   and report success.
 *
 * So the remedy stays where it is truthful and the row is the way to it, which
 * is still one click where it used to be three. */
function FamilyRow({
  group,
  count,
  exception,
  kind,
  last,
  onOpen,
}: {
  group: Group;
  count: string;
  /** Null when there is nothing to say, or when the only thing to say belongs to
   * the card above: the master being off, or the certificate. */
  exception: string | null;
  kind: GroupException | null;
  last: boolean;
  onOpen: () => void;
}) {
  const label = groupPillLabel(group);
  return (
    <div
      role="listitem"
      className={`relative transition hover:bg-gc-subtle${
        last ? "" : " border-b border-gc-line"
      }`}
    >
      {/* Stretch button over the whole row, Trust as a sibling above it - the
          same layering the panel uses for its switch. `aria-describedby` is what
          makes the row readable without eyes: this button is an empty sibling
          covering the row rather than an ancestor of its text, so name
          computation finds nothing inside it and the row would announce
          "Claude details, button" and never "Claude Code failed", which is the
          one thing it exists to say.

          Not because `pointer-events-none` hides anything: it decides clicks,
          not the accessibility tree, and the tree measured on 2026-08-10 has the
          pill, the count and the exception all present as StaticText. The
          description is what binds them to the control; it was never a rescue of
          unreachable content. */}
      <button
        type="button"
        onClick={onOpen}
        aria-label={`${group.name} details`}
        aria-describedby={`home-family-${group.id}`}
        className="absolute inset-0 cursor-pointer focus-visible:outline focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-gc-accent"
      />
      <span id={`home-family-${group.id}`} className="sr-only">
        {label}. {count}
        {/* Not when the pill already said it. A dark family now names its own
            cause, so `master-off` read "Waiting on routing. 0 of 2 routing.
            waiting on routing". The visible row already suppresses this pair;
            the description was the copy that still had both. */}
        {exception && exception.toLowerCase() !== label.toLowerCase() ? `. ${exception}` : ""}
      </span>
      {/* `flex-wrap` with an `em` basis on the name, so the row reflows with the
          type instead of starving it. At 200% the pill is ~200px wide in a 332px
          row, which left ~90px for the family name and rendered "Claude" as
          "Cla…" - loss of content, which is the one thing SC 1.4.4 names. The
          name now asks for 6 characters' worth at the current size (96px at
          100%, 192px at 200%), so at large scales the pill drops to its own line
          and the name keeps the full width. The stretch button is
          `absolute inset-0`, so a taller row stays entirely clickable. */}
      <div className="pointer-events-none relative flex flex-wrap items-center gap-x-2.5 gap-y-1 px-3.5 py-2.5">
        {/* `truncate`, not two lines: a family name is a proper noun and the
            longest one the catalog can produce is "Other tools". An h3, because
            these are sections of the h2 above them, so the outline nests instead
            of flattening four families up to the list's own level. */}
        <h3 className="min-w-0 flex-1 basis-[6em] truncate text-gc-body-md font-medium text-gc-ink">
          {group.name}
        </h3>
        <div className="ml-auto flex shrink-0 items-center gap-2.5">
          <GroupPill group={group} />
          <Icon name="chevronRight" size={15} stroke={2} className="shrink-0 text-gc-ink-4" />
        </div>
      </div>
      {exception && (
        <div className="relative flex items-center gap-2 px-3.5 pb-2">
          {/* The sentence carries its own severity, so a failure and a
              hand-written setup are not typographically identical. Two lines,
              because the exception's verb is at the end and a
              production-length tool name ate it at 360px. */}
          <span
            className={`pointer-events-none min-w-0 flex-1 line-clamp-2 text-gc-micro leading-snug ${
              kind === "error" ? "font-medium text-gc-error-deep" : "text-gc-ink-2"
            }`}
          >
            {exception}
          </span>
        </div>
      )}
    </div>
  );
}
