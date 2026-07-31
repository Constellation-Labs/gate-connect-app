import type { Group, GroupMember } from "../lib/groups";

const PILL =
  "inline-flex shrink-0 items-center gap-1.5 rounded-gc-pill px-2 py-1 text-[11px] font-medium";
const DOT = "h-1.5 w-1.5 rounded-full";

/** A family's headline state. It answers one question - is this routing? -
 * and never rounds up: "some" reads as partly, never as all. Exceptions are
 * named in the row's sub-line instead of hijacking the pill, so a mostly
 * working group doesn't read as broken. */
export function GroupPill({ group }: { group: Group }) {
  if (group.members.length === 0 || group.routed === 0) {
    return (
      <span className={`${PILL} bg-gc-sunken text-gc-ink-3`}>
        <span className={`${DOT} bg-gc-ink-5`} />
        Not routed
      </span>
    );
  }
  if (group.routed === group.members.length) {
    return (
      <span className={`${PILL} bg-gc-success-wash text-gc-success-deep`}>
        <span className={`${DOT} bg-gc-success`} />
        Routed
      </span>
    );
  }
  return (
    <span className={`${PILL} bg-gc-warning-wash text-gc-ink-2`}>
      <span className={`${DOT} bg-gc-warning`} />
      Partly routed
    </span>
  );
}

/** Per-member pill inside a group. Same grammar as the family pill, with the
 * two states only an individual can be in. */
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
