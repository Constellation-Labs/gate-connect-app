import type { Group, GroupMember } from "../lib/groups";

const PILL =
  "inline-flex shrink-0 items-center gap-1.5 rounded-gc-pill px-2 py-1 text-gc-micro font-medium";
const DOT = "h-1.5 w-1.5 rounded-full";

/** Every word either pill can show. */
type PillLabel =
  | "Routed"
  | "Partly routed"
  | "Error"
  | "Needs trust"
  | "Set up elsewhere"
  | "Waiting on routing"
  | "Not routed";

/** The pill's skin, including the seam ring tinted from its own hue and
 * weighted by severity.
 *
 * A wash at 8-14% alpha measures 1.09-1.16:1 against the row it sits on, so a
 * ringless capsule is invisible as an object: the words float in tinted air
 * beside a switch whose indigo track reads 5.98:1. Reality loses the row to
 * intent by a factor of ~5.4, on the element PRODUCT.md calls the most
 * important pixel on the screen.
 *
 * Not one seam for all states. The ladder is the point: error is a bordered
 * chip at ~3:1, and "Routed" - the state four rows out of four are in on a
 * healthy launch - stays the quietest of the set at ~1.5:1, because the
 * emotional target is "good, that's handled" and not a wall of edges. A ring
 * rather than a border, per the Seam Rule: solid 1px borders are what this
 * system draws with box-shadow instead.
 *
 * One table for both levels. The ladder shipped on the family pill only, which
 * left the member rows - the one place in the app where an intent control and a
 * reality report sit side by side and are *allowed to disagree* - at 1.08:1
 * beside a 5.68:1 switch, i.e. exactly the measurement the ring was introduced
 * to fix, unfixed. Two copies of a severity ladder would drift; there is one. */
const SKIN: Record<PillLabel, { wrap: string; dot: string }> = {
  Routed: {
    wrap: "bg-gc-success-wash text-gc-success-deep ring-1 ring-gc-success-deep/30",
    dot: "bg-gc-success-deep",
  },
  "Partly routed": {
    wrap: "bg-gc-warning-wash text-gc-ink-2 ring-1 ring-gc-warning-deep/45",
    dot: "bg-gc-warning-deep",
  },
  Error: {
    wrap: "bg-gc-error-wash text-gc-ink-2 ring-1 ring-gc-error-deep/65",
    dot: "bg-gc-error-deep",
  },
  // Both are "half-on for a reason outside this row": the certificate, or a
  // setup written elsewhere. Same rung as Partly routed.
  "Needs trust": {
    wrap: "bg-gc-warning-wash text-gc-ink-2 ring-1 ring-gc-warning-deep/45",
    dot: "bg-gc-warning-deep",
  },
  "Set up elsewhere": {
    wrap: "bg-gc-warning-wash text-gc-ink-2 ring-1 ring-gc-warning-deep/45",
    dot: "bg-gc-warning-deep",
  },
  // Switched on with the master off. Sunken like "Not routed" because nothing
  // is flowing, but ink-2 rather than ink-3: the user did not ask for this.
  "Waiting on routing": {
    wrap: "bg-gc-sunken text-gc-ink-2 ring-1 ring-gc-ink-4/45",
    dot: "bg-gc-ink-3",
  },
  "Not routed": {
    wrap: "bg-gc-sunken text-gc-ink-3 ring-1 ring-gc-ink-4/45",
    dot: "bg-gc-ink-3",
  },
};

function Pill({ label }: { label: PillLabel }) {
  const skin = SKIN[label];
  return (
    <span aria-hidden className={`${PILL} ${skin.wrap}`}>
      <span className={`${DOT} ${skin.dot}`} />
      {label}
    </span>
  );
}

/** A family's headline state as a word. Exported so the ledger row's
 * screen-reader description says exactly what the pill says: the pill is a
 * `pointer-events-none` span sitting under a stretch button, so it reaches
 * nobody listening unless the row repeats it. Two copies of this rule would
 * drift; there is one. */
export function groupPillLabel(
  group: Group,
): "Routed" | "Partly routed" | "Error" | "Needs trust" | "Not routed" {
  if (group.members.length === 0 || group.routed === 0) {
    // Nothing is flowing, so the only question left is why. Grey "Not routed"
    // is the same answer this pill gives for "you switched this off", and a
    // family that is dark for a reason the user did not choose must not borrow
    // it: they are here *because* something stopped working, and that was the
    // state the ledger reported most quietly.
    //
    // Two of the four causes get their own word; the other two do not, and the
    // test is whether the word discriminates between families on this screen.
    //
    // `error` and `needs-trust` do. In `home-partial` the certificate blocks
    // OpenRouter while Claude and OpenAI keep routing, so "Needs trust" names
    // which family is stuck - and grey there was actively misleading, because
    // pressing Trust turned a row the user read as "you turned this off" green.
    //
    // `master-off` cannot. It is set as `enabled && !proxyOn`, and `proxyOn` is
    // global, so every enabled member of every family gets it at the same
    // moment: printed here it is four identical pills saying what the card
    // directly above already said as a count ("Off · 8 waiting"), which is
    // DESIGN.md's "Card-owned states never print on a row" and the repetition
    // Home.tsx's ranking comment was written against. `drifted` stays out for
    // the existing reason - it rides the row's exception line, which names the
    // tool.
    if (group.members.some((m) => m.attention === "error")) return "Error";
    if (group.members.some((m) => m.attention === "needs-trust")) return "Needs trust";
    return "Not routed";
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
  return <Pill label={groupPillLabel(group)} />;
}

/** A member's observable state as a word. Exported so the switch beside the
 * pill can point at the same sentence: the switch reports intent and can read
 * "on" while nothing is flowing, so without this a screen-reader user hears
 * "on" for something that is not working. One rule, two renderings. */
export function memberPillLabel(member: GroupMember): PillLabel {
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
  return <Pill label={memberPillLabel(member)} />;
}
