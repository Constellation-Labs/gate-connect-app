import { useCallback, useRef } from "react";
import { cascadeTargets } from "./groups";
import type { Group, GroupMember } from "./groups";
import type { useRouting } from "./useRouting";
import type { useRunningApps } from "./useRunningApps";

/**
 * The app switch, shared by the window shell and the tray.
 *
 * A rail row is a section now, so its switch is a cascade: one click routes
 * every surface the app uses, and for Claude and ChatGPT that includes a surface
 * the person is signed in to. Three things have to happen in one order - ask if
 * the section has never been accepted, gate the certificate once, then write
 * each member - and both shells draw the same rows from the same
 * `buildGroups`, so both need all three.
 *
 * It lives here rather than in each shell because it was written twice and had
 * already started to drift in a week: the two copies differed on where
 * `setActionError(null)` sat, and carried two copies of the same user-facing
 * failure sentence.
 *
 * Not in `useRouting`, which has no view of the running-apps sequence and should
 * not grow one: this is the seam where a routing write and a "close the app you
 * just reconfigured" offer meet, which is exactly what the per-tool `routeApp`
 * in each shell already is.
 */
export function useSectionRouting({
  groups,
  routing,
  runningApps,
  onBeforeRoute,
  routeApp,
}: {
  groups: Group[];
  routing: ReturnType<typeof useRouting>;
  runningApps: ReturnType<typeof useRunningApps>;
  /** Clear whatever the last failure put on screen. The click is the moment the
   *  last failure stops being the current answer. */
  onBeforeRoute: () => void;
  /** The per-tool path, for a row that is not a section - a catalog entry no
   *  section has claimed yet. Keeps the drift gate and the OpenCode env
   *  coupling, which only `setAppRouted` raises. */
  routeApp: (slug: string, next: boolean) => void;
}) {
  /**
   * One cascade at a time.
   *
   * `routing.busy` cannot do this job here: `settle()` clears it before the
   * resync, so between two members of a cascade every switch in the window is
   * live again, and a click landing in that gap starts a second cascade over the
   * same section. Worse, the switch has already flipped by then - member one
   * landed - so the second cascade asks for the opposite direction and the two
   * interleave. A ref rather than state because the guard has to be visible to a
   * call made in the same tick.
   */
  const inFlight = useRef(false);

  /**
   * Route one whole section: every surface the app uses, in one click.
   *
   * `cascadeTargets` decides which members move - it skips the ones already in
   * the target state and never adopts a drifted or overridden config - and is
   * called with `sessions: true`, which is the one place the frontend's "no
   * group switch reaches a signed-in surface" rule is deliberately broken.
   *
   * Nothing replaces it any more. `SessionConsentDialog` used to stand here and
   * the rule was "cannot happen without being told"; product dropped the dialog
   * (AG-934, 2026-09-23) so a section switch now routes its signed-in surfaces
   * without asking and without saying so. That is the intended behaviour, not
   * an oversight: turning the section off stops it, per row or per section.
   */
  const routeSection = useCallback(
    async (section: Group, next: boolean) => {
      if (inFlight.current) return;
      const targets = cascadeTargets(section, next, { sessions: true });
      if (targets.length === 0) return;
      inFlight.current = true;
      try {
        // One gate for the whole cascade, not one per member: a section spans up
        // to three writes and each would otherwise raise its own certificate
        // dialog, so declining the first left the second one's on screen - the
        // person saying no and being asked again about the same certificate.
        // False means they declined, or trusting failed and has been reported.
        if (next && !(await routing.confirmCaTrusted())) return;
        const moved: GroupMember[] = [];
        for (const m of targets) {
          const wrote =
            m.kind === "proxy"
              ? await routing.setDomainRouted(m.key, next)
              : await routing.setAppRouted(m.key, next);
          if (wrote) moved.push(m);
        }
        // What actually wrote, not what was attempted. A member that failed or
        // whose gate was declined has not moved, and offering to close an app
        // whose config write did not land asks someone to restart a tool for
        // nothing.
        //
        // Both directions, unlike the first version of this: disconnecting puts
        // the tool's own config back, and a running `claude` or `codex` holds the
        // Gate relay URL until it restarts either way. A section turned off with
        // no offer leaves the tool pointed at a route the person just switched
        // off, with nothing on screen saying so.
        // Every member that moved, not only the config ones (AG-900).
        //
        // The `kind === "config"` filter that used to sit here is why closing
        // Claude left the Claude desktop app running while the dialog reported
        // it closed. The Claude section is `claude-code` + `anthropic` +
        // `claude-web`: the CLI writes a config, the desktop app is routed
        // through the system proxy instead, and it resolves that proxy at its
        // own launch - so it is exactly as stale after the switch as the CLI is,
        // and the section's own copy promises to cover it ("Claude Code in your
        // terminal, and the Claude desktop app").
        //
        // Nothing needs to decide here which slugs have processes. Rust's
        // `agent_names_for` already answers that from `AGENT_PROCESSES`, where
        // the two desktop apps are rows on purpose and carry proxy-domain keys
        // for exactly this call; a slug with no process of its own contributes
        // no names and drops out. `claude-web` is one of those, so the scan
        // asks about `claude` and `Claude` and about nothing else.
        const movedKeys = moved.map((m) => m.key);
        if (movedKeys.length > 0) await runningApps.offerAfterChange(movedKeys);
        // No failure summary is built here. `useRouting` reports each member's
        // failure through `onError`, classified and carrying the backend string;
        // the sentence this used to compose ("<section> is partly on - the rest
        // of it moved") was assembled from a `catch` around calls that never
        // throw, so it could not fire, and the `ClassifiedError` it built had an
        // empty `raw` - the one routing path in the app that could fail and leave
        // nothing to debug.
      } finally {
        inFlight.current = false;
      }
    },
    [routing, runningApps],
  );

  /**
   * Route or unroute one row.
   *
   * The section lookup comes FIRST, and that ordering is load-bearing rather
   * than stylistic: a row's slug is its section id, and one section id
   * (`chatgpt`) is also a catalog domain slug. Resolving the member first found
   * that domain - the ChatGPT subscription surface, which is
   * `Credential::Additive` - and routed the person's signed-in session on its
   * own, without Codex, without the app's chat turn, and without ever asking.
   */
  const toggle = useCallback(
    (slug: string, next: boolean) => {
      const section = groups.find((g) => g.id === slug);
      onBeforeRoute();
      if (section) {
        void routeSection(section, next);
        return;
      }
      routeApp(slug, next);
    },
    [groups, onBeforeRoute, routeSection, routeApp],
  );

  return { toggle, routeSection };
}
