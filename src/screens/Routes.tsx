import { useState } from "react";
import type { AuthMode } from "../lib/api";
import type { Group } from "../lib/groups";
import { groupSummary } from "../lib/groups";
import { SubHeader, Switch } from "../components/gc/ui";
import { GroupPill, groupPillLabel } from "../components/GroupPill";
import { Icon } from "../components/gc/Icon";
import { GroupMembers } from "./GroupMembers";

/** The ledger, on its own panel: one row per model family, each carrying the
 * pill that answers "is this routing?", a switch that routes the whole family
 * with one flip, and a body that opens the family's fine grain.
 *
 * It used to sit on Home under a section label. Home now carries the master
 * control and a labelled door to this panel, which keeps the room that holds a
 * routing card, a certificate ceremony, a wire line, a banner and a launch tip
 * from also holding an itemized list. The cost is that the pills are one
 * navigation further from the tray, so Home's door reports any exception itself
 * rather than leaving the user to come looking.
 *
 * A family opens in place rather than pushing a third screen. Its members are
 * themselves expandable, so the whole hierarchy - family, member, mechanism and
 * remedy - is two disclosures deep in one panel, and the popover never stacks a
 * screen it has to animate back out of. */
export function Routes({
  groups,
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
  groups: Group[];
  busy: boolean;
  onBack: () => void;
  onToggleGroup: (id: string, on: boolean) => void;
  /** The member-level callbacks, passed through to the expanded family. Rejects
   * on failure so the member row can surface it in place. */
  onToggleTool: (slug: string, routed: boolean) => Promise<void>;
  onSetDomain: (slug: string, enabled: boolean) => Promise<void>;
  onTrustCa: () => Promise<void>;
  proxyOn: boolean;
  onEnableRouting: () => void;
  authMode?: AuthMode;
}) {
  // Which family the user last flipped, so the live region below reports the
  // result of their own action and stays silent about the backend's. The row's
  // pill and sub-line both change on a flip and neither announces: a
  // description is read when focus arrives, not when it changes.
  const [flipped, setFlipped] = useState<string | null>(null);
  const flippedGroup = groups.find((g) => g.id === flipped);
  // Which family is expanded. One at a time: two open families in a 360px panel
  // put the second one's members below the fold with no way to see both.
  const [openKey, setOpenKey] = useState<string | null>(null);

  return (
    <div className="flex grow flex-col">
      {/* The panel's title is the question every row answers, in the same words
          the door on Home carries. */}
      <SubHeader title="What routes through Gate" onBack={onBack} />
      <span aria-live="polite" className="sr-only">
        {flippedGroup ? `${flippedGroup.name}, ${groupPillLabel(flippedGroup)}` : ""}
      </span>
      {groups.length > 0 ? (
        <div role="list" className="flex flex-col">
          {groups.map((group) => {
            const { count, exception, kind } = groupSummary(group);
            const open = openKey === group.id;
            return (
              <div key={group.id} role="listitem" className="border-b border-gc-line">
                <div
                  className={`relative flex items-center gap-2.5 px-3.5 py-3 transition ${
                    open ? "bg-gc-subtle" : "hover:bg-gc-subtle"
                  }`}
                >
                {/* Stretch button carries the drill-in; the switch is a sibling
                    above it, so one flip routes the whole family and the row
                    body opens the fine grain.

                    `aria-describedby` is what makes the row readable without
                    eyes. The count, the pill and the exception are all in
                    `pointer-events-none` spans so the stretch button can sit
                    over them, which also hid them from the accessibility tree:
                    the row announced "Claude details, button" and never
                    "OpenClaw failed", which is the one thing it exists to
                    say. */}
                <button
                  type="button"
                  onClick={() =>
                    setOpenKey((k) => (k === group.id ? null : group.id))
                  }
                  aria-expanded={open}
                  aria-label={`${group.name} details`}
                  aria-describedby={`group-desc-${group.id}`}
                  className="absolute inset-0 cursor-pointer focus-visible:outline focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-gc-accent"
                />
                <div className="pointer-events-none relative min-w-0 flex-1">
                  <div className="text-[13.5px] font-semibold text-gc-ink">{group.name}</div>
                  {/* Exception first. Concatenated as `count · exception` the
                      line truncated at 360px and the actionable half was what
                      got cut ("0 of 2 routing · Codex set up els…"). The pill
                      already answers "is this routing?", so the count is the
                      half that can afford to go.

                      Two lines, not `truncate`: the exception is a sentence
                      whose verb is at the end, so a production-length tool name
                      ate it. The count never needs the second line, so the row
                      only grows in the state that has something to say. */}
                  <div className="mt-0.5 line-clamp-2 text-[11px] leading-snug text-gc-ink-3">
                    {/* The sentence carries its own severity. Every exception
                        used to print in the same ink as every other, so a
                        failure and a hand-written setup were typographically
                        identical and the pill was reality's only voice on a row
                        whose switch reports intent in saturated indigo. */}
                    {exception ? (
                      <span
                        className={
                          kind === "error" ? "font-medium text-gc-error-deep" : "text-gc-ink-2"
                        }
                      >
                        {exception}
                      </span>
                    ) : (
                      count
                    )}
                  </div>
                </div>
                {/* The visible text truncates at 360px; this carries the whole
                    sentence, pill state included, to anyone listening. */}
                <span id={`group-desc-${group.id}`} className="sr-only">
                  {groupPillLabel(group)}. {count}
                  {exception ? `. ${exception}` : ""}
                </span>
                <span className="pointer-events-none relative">
                  <GroupPill group={group} />
                </span>
                <span className="relative">
                  <Switch
                    on={group.desired > 0}
                    label={group.switchLabel}
                    busy={busy}
                    onClick={() => {
                      setFlipped(group.id);
                      onToggleGroup(group.id, group.desired === 0);
                    }}
                  />
                </span>
                  <span className="pointer-events-none relative">
                    <Icon
                      name="chevronRight"
                      size={14}
                      stroke={2}
                      className={`text-gc-ink-4 transition-transform ${
                        open ? "rotate-90" : ""
                      }`}
                    />
                  </span>
                </div>
                {open && (
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
                )}
              </div>
            );
          })}
        </div>
      ) : (
        // Reachable without a dead end: reopening the popover re-reads the
        // ledger, so the last tool can be uninstalled while this panel is open.
        // Home explains the empty case in full; this says enough to get back.
        <p className="px-3.5 py-4 text-[11.5px] leading-snug text-gc-ink-3">
          Nothing is installed to route right now.
        </p>
      )}
    </div>
  );
}
