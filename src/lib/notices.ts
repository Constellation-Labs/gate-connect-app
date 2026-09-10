import type { Group, GroupMember } from "./groups";

/**
 * Routing notices for the Overview pane (AG-572).
 *
 * A tool can stop carrying traffic without anyone deciding it should: a tool
 * update rewrites its own config, the certificate stops being trusted, or
 * routing goes off and takes the loopback relay with it. Traffic then goes
 * straight to the provider, unprotected, and nothing on screen says so.
 *
 * **This is deliberately client-side.** The gateway cannot know that a local
 * `~/.codex/config.toml` drifted; only the app can. The states already exist as
 * `GroupMember.attention`, so this module is copy and an action per state, not
 * new detection.
 *
 * Each notice names the tool, the reason, and the action, which is what the
 * ticket asks for. The action is real work, not a link: `AlertBanner` renders a
 * switch, so a notice the user cannot act on would be worse than none.
 */

export type NoticeAction =
  | { kind: "enable-routing" }
  | { kind: "trust-certificate" }
  /**
   * Re-apply Gate's config to a tool that drifted, or retry one that errored.
   *
   * Names the tool and nothing else: this goes through `useRouting`, which
   * resolves the upstream URL and, for a drifted tool, puts the review dialog in
   * front of the write. Carrying the URL here would invite a caller to connect
   * directly and skip that gate.
   */
  | { kind: "reconnect"; slug: string };

export interface RoutingNotice {
  /** Stable across refreshes so a dismissal sticks to the right notice. */
  id: string;
  /** The member this is about, so a surface scoped to ONE app can pick its
   *  own. The app pane used to build its own drift-only card from the first
   *  drifted tool anywhere, which meant Claude Desktop's pane could draw a
   *  card whose body named Codex. */
  memberKey: string;
  title: string;
  body: string;
  /** Accessible name for the switch, which the title does not supply. */
  switchLabel: string;
  action: NoticeAction;
}

function noticeFor(member: GroupMember): RoutingNotice | null {
  const name = member.name;
  switch (member.attention) {
    case "master-off":
      return {
        id: `master-off:${member.key}`,
        memberKey: member.key,
        // The drawn copy (banner/alert/single-app, read 2026-08-23). "Routing"
        // is the master: the app is switched on and routing is not, which is
        // exactly the divergence this notice exists to explain.
        title: `${name} isn't protected`,
        body: "Routing is set to off. Reconnect to restore protection.",
        switchLabel: "Turn routing on",
        action: { kind: "enable-routing" },
      };
    case "needs-trust":
      return {
        id: `needs-trust:${member.key}`,
        memberKey: member.key,
        title: `${name} needs the Gate certificate`,
        body: "Gate cannot read this app's traffic until its certificate is trusted on this machine.",
        switchLabel: `Trust the certificate so ${name} can route`,
        action: { kind: "trust-certificate" },
      };
    case "drifted":
      // The drawn drift variant (read 2026-08-23) titles the card with the
      // remedy and puts the cause in the body. Carefully worded there for the
      // same reason it is here: drift is often the user's own doing - a
      // hand-written Gate setup, or another tool rewriting the file - so it
      // does not claim something broke. One deviation: the drawing says "This
      // app's" and names no app anywhere on a card that pages between apps,
      // so the name goes where that phrase was. Raised with the designer.
      // Adopting overwrites the user's config, which is why it stays an
      // explicit action rather than something reconciled silently.
      return member.tool
        ? {
            id: `drifted:${member.key}`,
            memberKey: member.key,
            title: "Reconnect to restore protection",
            body: `${name}'s config changed outside Gate, so its traffic isn't routed.`,
            switchLabel: `Let Gate Connect manage ${name}`,
            action: { kind: "reconnect", slug: member.key },
          }
        : null;
    case "overridden":
      // No card, deliberately. This module's rule is at the top of the file:
      // `AlertBanner` renders a switch, and every notice here is a switch that
      // does real work. An override has no such action - the winning value is in
      // a file Gate does not write, often somebody else's - so a card would
      // either offer a reconnect that changes nothing or a switch that lies.
      // The row's own status line carries the state ("Not protected -
      // Configuration overridden") and the tool's status names the file.
      return null;
    case "error":
      return member.tool
        ? {
            id: `error:${member.key}`,
            memberKey: member.key,
            title: `${name} could not be checked`,
            body:
              member.tool.status.kind === "error"
                ? member.tool.status.message
                : "Gate Connect could not read this tool's configuration.",
            switchLabel: `Try ${name} again`,
            action: { kind: "reconnect", slug: member.key },
          }
        : null;
    default:
      return null;
  }
}

/**
 * Every member needing attention, most actionable first.
 *
 * Ordered by how directly the user can fix it: routing and certificate are one
 * switch and fix every affected tool at once, so they lead. `master-off` is
 * collapsed to a single notice because twelve copies of "turn routing on" is
 * noise, not information - the others stay per-tool because each needs its own
 * decision.
 */
export function buildNotices(groups: Group[]): RoutingNotice[] {
  const members = groups.flatMap((g) => g.members).filter((m) => m.attention !== null);

  const ordered: RoutingNotice[] = [];
  const seenGlobal = new Set<string>();
  for (const kind of ["master-off", "needs-trust", "drifted", "error"] as const) {
    for (const m of members.filter((x) => x.attention === kind)) {
      const notice = noticeFor(m);
      if (!notice) continue;
      // One notice for the two whole-machine causes, however many tools show it.
      if (kind === "master-off" || kind === "needs-trust") {
        if (seenGlobal.has(kind)) continue;
        seenGlobal.add(kind);
        const count = members.filter((x) => x.attention === kind).length;
        ordered.push(
          count > 1
            ? {
                ...notice,
                id: kind,
                title:
                  kind === "master-off"
                    ? `${count} apps aren't protected`
                    : `${count} apps need the Gate certificate`,
              }
            : notice,
        );
        continue;
      }
      ordered.push(notice);
    }
  }
  return ordered;
}
