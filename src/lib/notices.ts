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
  /** Re-apply Gate's config to a tool that drifted, or retry one that errored. */
  | { kind: "reconnect"; slug: string; upstreamUrl: string };

export interface RoutingNotice {
  /** Stable across refreshes so a dismissal sticks to the right notice. */
  id: string;
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
        title: `${name} is switched on but not routing`,
        body: "Routing is off, so its traffic is going straight to the provider. Turn routing on to protect it.",
        switchLabel: "Turn routing on",
        action: { kind: "enable-routing" },
      };
    case "needs-trust":
      return {
        id: `needs-trust:${member.key}`,
        title: `${name} needs the Gate certificate`,
        body: "Gate cannot read this app's traffic until its certificate is trusted on this machine.",
        switchLabel: `Trust the certificate so ${name} can route`,
        action: { kind: "trust-certificate" },
      };
    case "drifted":
      // Copy is the design's own (Figma `banner/alert/single-app`), minus its
      // "It's config" typo. Carefully worded there for the same reason it is
      // here: drift is often the user's own doing - a hand-written Gate setup,
      // or another tool rewriting the file - so it does not claim something
      // broke. Adopting it overwrites their config, which is why it stays an
      // explicit action rather than something reconciled silently.
      return member.tool
        ? {
            id: `drifted:${member.key}`,
            title: `${name} isn't protected`,
            body: "Its config changed outside Gate, so its traffic isn't routed. Reconnect to restore protection.",
            switchLabel: `Let Gate Connect manage ${name}`,
            action: {
              kind: "reconnect",
              slug: member.key,
              upstreamUrl: member.tool.default_upstream_url,
            },
          }
        : null;
    case "error":
      return member.tool
        ? {
            id: `error:${member.key}`,
            title: `${name} could not be checked`,
            body:
              member.tool.status.kind === "error"
                ? member.tool.status.message
                : "Gate Connect could not read this tool's configuration.",
            switchLabel: `Try ${name} again`,
            action: {
              kind: "reconnect",
              slug: member.key,
              upstreamUrl: member.tool.default_upstream_url,
            },
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
                    ? `${count} apps are switched on but not routing`
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
