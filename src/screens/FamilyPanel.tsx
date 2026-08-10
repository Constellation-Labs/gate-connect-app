import { useState } from "react";
import type { AuthMode } from "../lib/api";
import type { Group } from "../lib/groups";
import { groupSummary } from "../lib/groups";
import { SubHeader, Switch } from "../components/gc/ui";
import { GroupPill, groupPillLabel } from "../components/GroupPill";
import { GroupMembers } from "./GroupMembers";

/** One model family, on a panel of its own: the switch that routes the whole
 * family, whatever needs a human, and the members underneath.
 *
 * This replaced a panel that listed every family behind four identical
 * chevrons. Home already carries all four as read-only rows, so that panel
 * printed the same four names, four pills, four counts and four exception
 * sentences a second time, and only ever opened one of them - `openKey` was
 * single-open by design, because two expanded families in a 360px window put
 * the second one's members below the fold. Four doors led to one room where
 * three of the four visible rows were a copy of the screen the user had just
 * left, and the chevron on Home promised a destination that did not vary.
 *
 * So the chevron now goes where it says. One family is on screen, named by the
 * panel's own h1, and the accordion, the second list and `initialOpen` are all
 * gone with it: arriving already open is what a panel about one family does by
 * existing.
 *
 * Depth grammar is two affordances now, not three. Home's row navigates with a
 * stroked chevron and the member level says the word "Details"; the filled
 * rotating caret that meant "expands in place" has nothing left to open.
 *
 * The count sits under the switch rather than the exception sentence. Every
 * exception `groupSummary` can name - error, needs-trust, master-off, drifted -
 * has a banner below with the remedy attached, so printing the summary here too
 * would state one fact twice on one screen and put the shorter, unactionable
 * copy first. The count is the half the banners do not carry, and on a panel
 * that lists the members it is finally checkable where it is asserted. */
export function FamilyPanel({
  group,
  busy,
  onBack,
  onToggleGroup,
  onToggleTool,
  onSetDomain,
  onTrustCa,
  proxyOn,
  onEnableRouting,
  authMode,
}: {
  group: Group;
  busy: boolean;
  onBack: () => void;
  onToggleGroup: (id: string, on: boolean) => void;
  /** The member-level callbacks, passed through. Rejects on failure so the
   * member row can surface it in place. */
  onToggleTool: (slug: string, routed: boolean) => Promise<void>;
  onSetDomain: (slug: string, enabled: boolean) => Promise<void>;
  onTrustCa: () => Promise<void>;
  proxyOn: boolean;
  onEnableRouting: () => void;
  authMode?: AuthMode;
}) {
  // Whether the user has flipped the family switch on this visit, so the live
  // region below reports the result of their own action and stays silent about
  // the backend's. The pill changes on a flip and does not announce: a
  // description is read when focus arrives, not when it changes.
  const [flipped, setFlipped] = useState(false);
  const { count } = groupSummary(group);

  return (
    <div className="flex grow flex-col">
      {/* The family's own name is the title. It used to be an h2 on a row
          inside a panel titled "What routes through Gate"; that sentence is
          still the heading on Home, above the rows that ask it, which is the
          level where it is a question the user is choosing an answer to. */}
      <SubHeader title={group.name} onBack={onBack} />
      <span aria-live="polite" className="sr-only">
        {flipped ? `${group.name}, ${groupPillLabel(group)}` : ""}
      </span>

      {/* `flex-wrap` with an `em` basis on the text column, the same rule the
          routing card and Home's family rows use. At 200% the pill and the
          switch together need most of the row, so they drop to their own line
          and the label keeps the full width instead of breaking mid-word. */}
      <div className="flex flex-wrap items-center gap-x-2.5 gap-y-1.5 border-b border-gc-line px-3.5 py-3">
        <div className="min-w-0 flex-1 basis-[8em]">
          {/* Not the family name again: the h1 two rows up already said it, and
              this is the control's own label. */}
          <div className="text-gc-body font-semibold text-gc-ink">Route through Gate</div>
          <div className="mt-0.5 text-gc-caption leading-snug text-gc-ink-3">{count}</div>
        </div>
        <div className="ml-auto flex shrink-0 items-center gap-2.5">
          <GroupPill group={group} />
          {/* Intent and reality side by side, which is the pairing DESIGN.md
              allows to disagree. The switch reports the stored choice and can
              read "on" while the pill beside it says nothing is flowing, so it
              points at the pill's own words for anyone listening rather than
              announcing "on" and stopping there. */}
          <Switch
            on={group.desired > 0}
            label={group.switchLabel}
            describedBy={`group-desc-${group.id}`}
            busy={busy}
            onClick={() => {
              setFlipped(true);
              onToggleGroup(group.id, group.desired === 0);
            }}
          />
          <span id={`group-desc-${group.id}`} className="sr-only">
            {groupPillLabel(group)}
          </span>
        </div>
      </div>

      {/* The one family named by exclusion gets a sentence. `blurb` has existed
          since "Agent harnesses" was retired and its definition was moved here
          to be explained properly, but nothing ever rendered it, so for two
          rounds the entire UI description of these tools was 18 characters in a
          truncating slot two rows down - and those 18 characters claimed the
          wrong thing. This is the level that owes the explanation: the panel is
          about one family, and this one's name says only what it is not.

          Its own band under the control row rather than beside the h1, because
          it is two sentences and the header is a fixed zone. Only rendered where
          it exists, which is the multi-provider group alone; a line reading
          "Everything that talks to Anthropic." under an h1 reading "Anthropic"
          is the same fact twice, so those were deleted rather than shown. */}
      {group.blurb && (
        <p className="border-b border-gc-line px-3.5 py-2.5 text-gc-caption leading-snug text-gc-ink-3">
          {group.blurb}
        </p>
      )}

      <GroupMembers
        group={group}
        busy={busy}
        onToggleTool={onToggleTool}
        onSetDomain={onSetDomain}
        onTrustCa={onTrustCa}
        proxyOn={proxyOn}
        onEnableRouting={onEnableRouting}
        authMode={authMode}
      />
    </div>
  );
}
