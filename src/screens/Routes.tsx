import { useState } from "react";
import type { AuthMode } from "../lib/api";
import type { Group } from "../lib/groups";
import { groupSummary } from "../lib/groups";
import { SubHeader, Switch } from "../components/gc/ui";
import { GroupPill, groupPillLabel } from "../components/GroupPill";
import { Icon } from "../components/gc/Icon";
import { GroupMembers } from "./GroupMembers";

/** The ledger's fine grain, on its own panel: one row per model family, each
 * carrying the pill that answers "is this routing?", a switch that routes the
 * whole family with one flip, and a body that opens the family's members.
 *
 * Home carries the same families as read-only rows, so this panel is where a
 * family is *changed* rather than where it is first learned about. A row on Home
 * arrives here with its own family already open, which is why `initialOpen`
 * exists: tapping a row that says "Claude Code failed" and landing on a
 * collapsed list would charge a second click for the thing just tapped.
 *
 * It also owns the shell-environment channel, at the bottom. That switch is not
 * a family - it routes every command-line tool at once, whatever provider they
 * talk to - so it cannot be a row here, and it was a near-identical peer of the
 * master switch on Home: same 38x22 track, same indigo, 66px apart, telling the
 * user by geometry that a machine-wide change to git and curl was the routing
 * switch's equal.
 *
 * A family opens in place rather than pushing a third screen, so the whole
 * hierarchy - family, member, mechanism and remedy - is two disclosures deep in
 * one panel, and the popover never stacks a screen it has to animate back out
 * of. */
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
  initialOpen,
  envExportSeparable,
  envExportOn,
  onToggleEnvExport,
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
  /** The family to arrive with already expanded, when the user got here from a
   * Home row rather than from the heading. */
  initialOpen?: string | null;
  /** Whether the shell-environment channel can be offered at all. False on
   * Linux, where those variables *are* the system proxy, so a switch could not
   * honour itself. */
  envExportSeparable: boolean;
  envExportOn: boolean;
  onToggleEnvExport: () => void;
}) {
  // Which family the user last flipped, so the live region below reports the
  // result of their own action and stays silent about the backend's. The row's
  // pill and sub-line both change on a flip and neither announces: a
  // description is read when focus arrives, not when it changes.
  const [flipped, setFlipped] = useState<string | null>(null);
  const flippedGroup = groups.find((g) => g.id === flipped);
  // Which family is expanded. One at a time: two open families in a 360px panel
  // put the second one's members below the fold with no way to see both. Seeded
  // from the Home row that got the user here.
  const [openKey, setOpenKey] = useState<string | null>(initialOpen ?? null);

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
                  {/* An h2 under the panel's h1, so four families and up to six
                      pills are navigable by heading instead of being one
                      undifferentiated list to a screen reader. */}
                  <h2 className="text-[13.5px] font-semibold text-gc-ink">{group.name}</h2>
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
                      name="caretRight"
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

      {/* The other channel, after the families rather than among them. Routing
          reaches GUI apps through the OS proxy setting and command-line tools
          through the shell environment; this is the second one, and it spans
          every family at once, so it is not a row and never was.

          `ink-3`, not `ink-4`. This is two sentences of instruction about the
          one control in the app that changes something outside the AI tools -
          `HTTPS_PROXY` reaches git, curl and every process in the shell - and at
          ink-4 it measured 3.97:1 on white, the only text in the product below
          the 4.5:1 floor. DESIGN.md scopes ink-4 to "placeholders, muted icons,
          and incidental mono identifiers only; never sentence copy or labels",
          and OAuthOffer already carries the note that ink-3 is the smallest ink
          that may carry real text. ink-3 is 6.90:1.

          Absent entirely on Linux, where the `environment.d` drop-in *is* the
          system proxy: there the variables cannot be declined without turning
          routing off, and a switch that cannot honour itself is worse than no
          switch. `env_export_separable` carries that from the backend rather
          than the UI guessing at platforms. */}
      {envExportSeparable && (
        <div className="flex items-start gap-3 px-3.5 py-3">
          <div className="min-w-0 flex-1">
            <div className="text-[12.5px] font-medium text-gc-ink">Command-line tools</div>
            <div className="mt-0.5 text-[11px] leading-snug text-gc-ink-3">
              Sets <span className="font-mono">HTTPS_PROXY</span> for your whole
              shell, so OpenCode and other terminal tools route too.
            </div>
            {/* Intent and reality, kept apart here the way every ledger row
                keeps them apart. The switch reports the stored choice, which
                survives routing being turned off; this line reports that the
                choice is not in effect. Without it the row painted a saturated
                indigo track - the colour DESIGN.md reserves for live state - for
                a channel that cannot be live, in the app whose whole thesis is
                that its status is truthful. Same words the member pill uses for
                the same condition, so there is one vocabulary for it. */}
            {envExportOn && !proxyOn && (
              <div className="mt-1 text-[11px] font-medium leading-snug text-gc-ink-2">
                Waiting on routing
              </div>
            )}
          </div>
          <Switch
            on={envExportOn}
            label="Route command-line tools through Gate"
            busy={busy}
            onClick={onToggleEnvExport}
          />
        </div>
      )}
    </div>
  );
}
