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
      ? { wrap: "bg-gc-success-wash text-gc-success-deep", dot: "bg-gc-success" }
      : label === "Partly routed"
        ? { wrap: "bg-gc-warning-wash text-gc-ink-2", dot: "bg-gc-warning" }
        : { wrap: "bg-gc-sunken text-gc-ink-3", dot: "bg-gc-ink-5" };
  return (
    <span aria-hidden className={`${PILL} ${skin.wrap}`}>
      <span className={`${DOT} ${skin.dot}`} />
      {label}
    </span>
  );
}

/** Per-member pill inside a group. Same grammar as the family pill, with the
 * states only an individual can be in. The pill reports whether traffic is
 * flowing; the switch beside it reports what the user asked for. Those are
 * different facts, and the "Needs trust" and "Waiting on routing" states are
 * exactly where they diverge. */
export function MemberPill({ member }: { member: GroupMember }) {
  if (member.attention === "error") {
    return (
      <span className={`${PILL} bg-gc-error-wash text-gc-ink-2`}>
        <span className={`${DOT} bg-gc-error`} />
        Error
      </span>
    );
  }
  if (member.attention === "drifted") {
    return (
      <span className={`${PILL} bg-gc-warning-wash text-gc-ink-2`}>
        <span className={`${DOT} bg-gc-warning`} />
        Set up elsewhere
      </span>
    );
  }
  if (member.attention === "needs-trust") {
    return (
      <span className={`${PILL} bg-gc-warning-wash text-gc-ink-2`}>
        <span className={`${DOT} bg-gc-warning`} />
        Needs trust
      </span>
    );
  }
  // Switched on, master off: the config still points at a relay that isn't
  // running. Not "Routed" (nothing flows) and not "Not routed" (the user
  // didn't turn it off).
  if (member.attention === "master-off") {
    return (
      <span className={`${PILL} bg-gc-sunken text-gc-ink-2`}>
        <span className={`${DOT} bg-gc-ink-4`} />
        Waiting on routing
      </span>
    );
  }
  if (member.routed) {
    return (
      <span className={`${PILL} bg-gc-success-wash text-gc-success-deep`}>
        <span className={`${DOT} bg-gc-success`} />
        Routed
      </span>
    );
  }
  return (
    <span className={`${PILL} bg-gc-sunken text-gc-ink-3`}>
      <span className={`${DOT} bg-gc-ink-5`} />
      Not routed
    </span>
  );
}
