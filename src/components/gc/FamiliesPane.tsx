import { BaseSwitch, Card } from "./base";
import { Icon } from "./Icon";
import type { AppStatus } from "./Sidebar";

/**
 * The model families and what each of them routes.
 *
 * **This pane is not in the Figma.** The design lists apps flat, which cannot
 * express the model the app actually runs on: routing is owned by families
 * (Claude, OpenAI, OpenRouter, plus the multi-provider "Other tools" bucket),
 * each with a master switch, and their members route by two different
 * mechanisms - a tool's own config file, or the local proxy. The proxy members
 * are the chat domains, which have no representation in the drawn UI at all.
 *
 * Rather than drop that, it gets a destination here. Expect this to be
 * redrawn; see plans/new-app-ui-figma.md.
 */

export type MemberKind = "config" | "proxy";

export interface FamilyMember {
  /** Tool slug or domain slug - unique within a family. */
  key: string;
  name: string;
  kind: MemberKind;
  /** Observed. Same split as the sidebar: this is not the switch's value. */
  status: AppStatus;
  /** Intent, which drives the switch. */
  on: boolean;
  busy?: boolean;
}

/**
 * The engine itself, above the families that ride on it.
 *
 * Not in the Figma either, and the omission is load-bearing: with routing off,
 * a family switch can still start the engine (a config member's connect does it
 * implicitly) but a chat domain cannot, so the window could reach a state it
 * had no control for. `envExport` is the master's sub-setting - whether the
 * proxy also goes into the shell environment, which reaches `git` and `curl`
 * and not just the AI tools - and is absent on Linux, where those variables
 * *are* the system proxy and cannot be declined separately.
 */
export interface MasterRouting {
  on: boolean;
  busy?: boolean;
  onToggle: (next: boolean) => void;
  /** Whether the certificate is in the system trust store. Routing without it
   * inspects nothing, so the card says so rather than leaving the switch to
   * imply otherwise. */
  caTrusted?: boolean;
  envExport?: { on: boolean; onToggle: (next: boolean) => void };
}

export interface Family {
  id: string;
  name: string;
  /** Whether the family as a whole is routing. */
  on: boolean;
  busy?: boolean;
  members: FamilyMember[];
}

const STATUS_TEXT: Record<AppStatus["kind"], { label: string; className: string }> = {
  protected: { label: "Protected", className: "text-green-600" },
  "not-protected": { label: "Not protected", className: "text-amber-600" },
  drifted: { label: "Config drifted", className: "text-amber-600" },
  "not-routed": { label: "Not routed", className: "text-amber-600" },
};

/** Config members route through their own file; proxy members through the
 *  local proxy. The distinction decides what "turn this off" actually does, so
 *  it is named rather than implied. */
const KIND_TEXT: Record<MemberKind, string> = {
  config: "Config file",
  proxy: "Local proxy",
};

export function FamiliesPane({
  families,
  master,
  onToggleFamily,
  onToggleMember,
}: {
  families: Family[];
  /** Omit on a platform with no proxy subsystem, where there is no engine to
   * turn on and the card would describe nothing. */
  master?: MasterRouting;
  /**
   * Omit to hide the family-level switches. A master switch has to cascade over
   * members, skipping the ones with a hand-written config, and until that is
   * wired a switch that does nothing is worse than no switch at all.
   */
  onToggleFamily?: (id: string, next: boolean) => void;
  onToggleMember: (familyId: string, key: string, next: boolean) => void;
}) {
  return (
    <div className="flex flex-1 flex-col gap-4 overflow-auto bg-gray-100 p-6">
      <header>
        <h1 className="text-xl font-medium tracking-heading text-neutral-900">
          Families
        </h1>
        <p className="mt-1 text-sm leading-5 text-neutral-600">
          Each model family routes its own apps and sites. Turning a family off stops
          everything under it.
        </p>
      </header>

      {master && (
        <Card>
          <div className="flex items-center gap-3 p-4">
            <div className="min-w-0 flex-1">
              <h2 className="truncate text-sm font-medium leading-5 text-neutral-900">
                Route traffic through Gate
              </h2>
              <p className="text-base-xs leading-4 text-base-muted-foreground">
                {master.on
                  ? master.caTrusted === false
                    ? "Running, but the certificate is not trusted - nothing is being inspected"
                    : "The local engine is running"
                  : "Everything below stays off until this is on"}
              </p>
            </div>
            <BaseSwitch
              on={master.on}
              label="Route traffic through Gate"
              busy={master.busy}
              onClick={() => master.onToggle(!master.on)}
            />
          </div>

          {master.envExport && (
            <div className="flex items-center gap-3 border-t border-base-border px-4 py-3">
              <Icon name="codeXml" size={16} className="shrink-0 text-neutral-500" />
              <div className="min-w-0 flex-1">
                <p className="truncate text-sm leading-5 text-neutral-900">
                  Also set shell environment variables
                </p>
                <p className="text-base-2xs leading-4 text-base-muted-foreground">
                  Routes command-line tools too. Machine-wide: it reaches git and curl,
                  not only your AI tools.
                </p>
              </div>
              <BaseSwitch
                on={master.envExport.on}
                label="Also set shell environment variables"
                busy={master.busy}
                onClick={() => master.envExport?.onToggle(!master.envExport.on)}
              />
            </div>
          )}
        </Card>
      )}

      {families.map((family) => {
        const routed = family.members.filter((m) => m.status.kind === "protected").length;
        return (
          <Card key={family.id}>
            <div className="flex items-center gap-3 p-4">
              <div className="min-w-0 flex-1">
                <h2 className="truncate text-sm font-medium leading-5 text-neutral-900">
                  {family.name}
                </h2>
                <p className="text-base-xs leading-4 text-base-muted-foreground">
                  {routed} of {family.members.length} routing
                </p>
              </div>
              {onToggleFamily && (
                <BaseSwitch
                  on={family.on}
                  label={`Route ${family.name}`}
                  busy={family.busy}
                  onClick={() => onToggleFamily(family.id, !family.on)}
                />
              )}
            </div>

            <ul>
              {family.members.map((member) => {
                const status = STATUS_TEXT[member.status.kind];
                const suffix =
                  member.status.kind === "protected"
                    ? member.status.since
                    : member.status.kind === "not-routed"
                      ? member.status.detail
                      : undefined;
                return (
                  <li
                    key={member.key}
                    className="flex items-center gap-3 border-t border-base-border px-4 py-3"
                  >
                    <Icon
                      name={member.kind === "proxy" ? "globe" : "cube"}
                      size={16}
                      className="shrink-0 text-neutral-500"
                    />
                    <div className="min-w-0 flex-1">
                      <p className="truncate text-sm leading-5 text-neutral-900">
                        {member.name}
                      </p>
                      <p className="truncate text-base-2xs font-medium leading-4">
                        <span className={status.className}>{status.label}</span>
                        {suffix && <span className="text-neutral-500"> - {suffix}</span>}
                        <span className="text-base-muted-foreground">
                          {" · "}
                          {KIND_TEXT[member.kind]}
                        </span>
                      </p>
                    </div>
                    <BaseSwitch
                      on={member.on}
                      label={member.name}
                      busy={member.busy}
                      onClick={() => onToggleMember(family.id, member.key, !member.on)}
                    />
                  </li>
                );
              })}
            </ul>
          </Card>
        );
      })}
    </div>
  );
}
