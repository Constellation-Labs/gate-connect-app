import type { Group, GroupMember } from "../lib/groups";

const PILL =
  "inline-flex shrink-0 items-center gap-1.5 rounded-gc-pill px-2 py-1 text-[11px] font-medium";
const DOT = "h-1.5 w-1.5 rounded-full";

/** A family's headline state as a word. Exported so the ledger row's
 * screen-reader description says exactly what the pill says: the pill is a
 * `pointer-events-none` span sitting under a stretch button, so it reaches
 * nobody listening unless the row repeats it. Two copies of this rule would
 * drift; there is one. */
export function groupPillLabel(
  group: Group,
): "Routed" | "Partly routed" | "Error" | "Not routed" {
  if (group.members.length === 0 || group.routed === 0) {
    // Nothing is flowing, so the only question left is why. Grey "Not routed"
    // is the same answer this pill gives for "you switched this off", and a
    // family that is dark because a tool failed must not borrow it: the user
    // who opens the popover mid-task is here *because* something stopped
    // working, and that was the state the ledger reported most quietly.
    return group.members.some((m) => m.attention === "error") ? "Error" : "Not routed";
  }
  if (group.routed === group.members.length) return "Routed";
  // A failure inside a family that is otherwise carrying traffic stays out of
  // the pill and rides the row's exception line instead - see the note below
  // on not letting one exception make a mostly working group read as broken.
  return "Partly routed";
}

/** A family's headline state. It answers one question - is this routing? -
 * and never rounds up: "some" reads as partly, never as all. A group that is
 * routing nothing because something failed says so, because grey is what this
 * pill says for a switch the user set and a failure is not that. Exceptions
 * inside a group that is otherwise working stay in the row's sub-line instead
 * of hijacking the pill, so a mostly working group doesn't read as broken.
 *
 * `aria-hidden`: the row's own description carries this text (see
 * [`groupPillLabel`]), so leaving it visible to a screen reader would say the
 * state twice. */
export function GroupPill({ group }: { group: Group }) {
  const label = groupPillLabel(group);
  // The seam, tinted from the pill's own hue and weighted by severity.
  //
  // A wash at 8-14% alpha measures 1.09-1.16:1 against the row it sits on, so
  // the capsule was invisible as an object: the words floated in tinted air
  // beside a switch whose indigo track reads 5.98:1. Reality lost the row to
  // intent by a factor of 5.4, on the element PRODUCT.md calls the most
  // important pixel on the screen. Now that these rows are Home's primary
  // content and there are four of them, a pill has to read as a thing.
  //
  // Not one seam for all four states. The ladder is the point: error is a
  // bordered chip at ~3:1, and "Routed" - the state four rows out of four are
  // in on a healthy launch - stays the quietest of the set at ~1.5:1, because
  // the emotional target is "good, that's handled" and not a wall of edges.
  // A ring rather than a border, per the Seam Rule: solid 1px borders are what
  // this system draws with box-shadow instead.
  const skin =
    label === "Routed"
      ? {
          wrap: "bg-gc-success-wash text-gc-success-deep ring-1 ring-gc-success-deep/30",
          dot: "bg-gc-success-deep",
        }
      : label === "Partly routed"
        ? {
            wrap: "bg-gc-warning-wash text-gc-ink-2 ring-1 ring-gc-warning-deep/45",
            dot: "bg-gc-warning-deep",
          }
        : label === "Error"
          ? // Same skin as the member pill's error state: one vocabulary for
            // one condition, at both levels of the ledger.
            {
              wrap: "bg-gc-error-wash text-gc-ink-2 ring-1 ring-gc-error-deep/65",
              dot: "bg-gc-error-deep",
            }
          : {
              wrap: "bg-gc-sunken text-gc-ink-3 ring-1 ring-gc-ink-4/45",
              dot: "bg-gc-ink-3",
            };
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
