import type { Group, GroupMember } from "./groups";

/**
 * Routing notices (AG-572), for two surfaces.
 *
 * Overview draws the two whole-machine causes - routing off, certificate
 * untrusted - collapsed to one card each (`machineNotices`): they are true of
 * every tool at once, and the boot screen is where the word "certificate" and
 * its one-click fix have to be. A tool's own causes - drift, a check error -
 * are drawn only on that tool's pane (`memberNotices`), which also repeats the
 * machine-wide ones worded for that tool. Until 2026-09-16 Overview paged
 * through every per-tool card as well.
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
  /** Stable across refreshes so a dismissal sticks to the right notice. A
   *  per-member notice carries the member's key; Overview's collapsed card is
   *  keyed on the cause alone, so dismissing it there does not dismiss the
   *  per-tool wording on a pane, and the reverse. */
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
    case "not-routing":
      return {
        id: `not-routing:${member.key}`,
        // The app is switched on and routing is not, which is exactly the
        // divergence this notice exists to explain. The drawn copy
        // (banner/alert/single-app, read 2026-08-23) said "Routing is set to
        // off", which named a master switch the user set - and there is no
        // longer a switch to set, nor anybody who set it. The integrations are
        // the switch now, so this state is the engine failing to start under a
        // row that is on, most often a refused admin prompt. Saying it was
        // "set to off" would send the user looking for a control to put back.
        // Typographic apostrophe, as drawn (`228:90612`, "Codex isn’t
        // protected"). The file uses it throughout and `lib/errors.ts` already
        // matches; this line and its plural below did not.
        title: `${name} isn’t protected`,
        body: "Routing isn’t running. Turn it on to restore protection.",
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
      // The drawn drift variant (read 2026-08-23) titles the card with the
      // remedy and puts the cause in the body. Carefully worded there for the
      // same reason it is here: drift is often the user's own doing - a
      // hand-written Gate setup, or another tool rewriting the file - so it
      // does not claim something broke. One deviation: the drawing says "This
      // app's" and names no app; the body names it, so the sentence stays true
      // wherever the card is drawn. Raised with the designer.
      // Adopting overwrites the user's config, which is why it stays an
      // explicit action rather than something reconciled silently.
      return member.tool
        ? {
            id: `drifted:${member.key}`,
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

const RANK = ["not-routing", "needs-trust", "drifted", "error"] as const;

function attentive(groups: Group[]): GroupMember[] {
  return groups.flatMap((g) => g.members).filter((m) => m.attention !== null);
}

/**
 * The whole-machine causes, one card each, for Overview.
 *
 * `not-routing` and `needs-trust` are one switch and fix every affected tool at
 * once, so twelve copies of "turn routing on" would be noise, not information:
 * the card counts the tools instead. Nothing per-tool is included - a drifted
 * config is about one tool and is drawn on that tool's pane.
 */
export function machineNotices(groups: Group[]): RoutingNotice[] {
  const members = attentive(groups);
  const ordered: RoutingNotice[] = [];
  for (const kind of ["not-routing", "needs-trust"] as const) {
    const affected = members.filter((m) => m.attention === kind);
    const notice = affected[0] ? noticeFor(affected[0]) : null;
    if (!notice) continue;
    ordered.push(
      affected.length > 1
        ? {
            ...notice,
            id: kind,
            title:
              kind === "not-routing"
                ? `${affected.length} apps aren’t protected`
                : `${affected.length} apps need the Gate certificate`,
          }
        : notice,
    );
  }
  return ordered;
}

/**
 * Every notice about the members named by `keys`, most actionable first, for
 * one section's pane.
 *
 * No collapsing: a pane draws at most one card, so there is nothing to
 * collapse into, and a count would read "3 apps aren't protected" on a pane
 * headed by one app whose three surfaces are off. Each card names the one
 * member it is about and is dismissed by that member's key.
 */
export function memberNotices(groups: Group[], keys: readonly string[]): RoutingNotice[] {
  const members = attentive(groups).filter((m) => keys.includes(m.key));
  return RANK.flatMap((kind) =>
    members
      .filter((m) => m.attention === kind)
      .map(noticeFor)
      .filter((n): n is RoutingNotice => n !== null),
  );
}
