import type { Group, GroupMember } from "../lib/groups";

const PILL =
  "inline-flex shrink-0 items-center gap-1.5 rounded-gc-pill px-2 py-1 text-[11px] font-medium";
const DOT = "h-1.5 w-1.5 rounded-full";

/** A family's headline state as a word. Exported so the ledger row's
 * screen-reader description says exactly what the pill says: the pill is a
 * `pointer-events-none` span sitting under a stretch button, so it reaches
 * nobody listening unless the row repeats it. Two copies of this rule would
 * drift; there is one. */
export function groupPillLabel(group: Group): "Routed" | "Partly routed" | "Not routed" {
  if (group.members.length === 0 || group.routed === 0) return "Not routed";
  if (group.routed === group.members.length) return "Routed";
  return "Partly routed";
}

/** A family's headline state. It answers one question - is this routing? -
 * and never rounds up: "some" reads as partly, never as all. Exceptions are
 * named in the row's sub-line instead of hijacking the pill, so a mostly
 * working group doesn't read as broken.
 *
 * `aria-hidden`: the row's own description carries this text (see
 * [`groupPillLabel`]), so leaving it visible to a screen reader would say the
 * state twice. */
export function GroupPill({ group }: { group: Group }) {
  const label = groupPillLabel(group);
  const skin =
    label === "Routed"
      ? { wrap: "bg-gc-success-wash text-gc-success-deep", dot: "bg-gc-success-deep" }
      : label === "Partly routed"
        ? { wrap: "bg-gc-warning-wash text-gc-ink-2", dot: "bg-gc-warning-deep" }
        : { wrap: "bg-gc-sunken text-gc-ink-3", dot: "bg-gc-ink-3" };
  return (
    <span aria-hidden className={`${PILL} ${skin.wrap}`}>
      <span className={`${DOT} ${skin.dot}`} />
      {label}
    </span>
  );
}

/** A member's observable state as a word. Exported so the switch beside the
 * pill can point at the same sentence: the switch reports intent and can read
 * "on" while nothing is flowing, so without this a screen-reader user hears
 * "on" for something that is not working. One rule, two renderings. */
export function memberPillLabel(member: GroupMember): string {
  if (member.attention === "error") return "Error";
  if (member.attention === "drifted") return "Set up elsewhere";
  if (member.attention === "needs-trust") return "Needs trust";
  // Switched on, master off: the config still points at a relay that isn't
  // running. Not "Routed" (nothing flows) and not "Not routed" (the user
  // didn't turn it off).
  if (member.attention === "master-off") return "Waiting on routing";
  return member.routed ? "Routed" : "Not routed";
}

/** Per-member pill inside a group. Same grammar as the family pill, with the
 * states only an individual can be in. The pill reports whether traffic is
 * flowing; the switch beside it reports what the user asked for.
 *
 * `aria-hidden`: the row description and the switch's own `aria-describedby`
 * already carry this text, so exposing it here would say the state twice. */
export function MemberPill({ member }: { member: GroupMember }) {
  const label = memberPillLabel(member);
  const skin =
    label === "Error"
      ? { wrap: "bg-gc-error-wash text-gc-ink-2", dot: "bg-gc-error-deep" }
      : label === "Set up elsewhere" || label === "Needs trust"
        ? { wrap: "bg-gc-warning-wash text-gc-ink-2", dot: "bg-gc-warning-deep" }
        : label === "Waiting on routing"
          ? { wrap: "bg-gc-sunken text-gc-ink-2", dot: "bg-gc-ink-3" }
          : label === "Routed"
            ? { wrap: "bg-gc-success-wash text-gc-success-deep", dot: "bg-gc-success-deep" }
            : { wrap: "bg-gc-sunken text-gc-ink-3", dot: "bg-gc-ink-3" };
  return (
    <span aria-hidden className={`${PILL} ${skin.wrap}`}>
      <span className={`${DOT} ${skin.dot}`} />
      {label}
    </span>
  );
}
