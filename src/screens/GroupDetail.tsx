import type { Group } from "../lib/groups";
import { SubHeader, SectionLabel, Switch } from "../components/gc/ui";
import { MemberPill } from "../components/GroupPill";
import { Icon } from "../components/gc/Icon";

/** One model family's fine grain. This is where the mechanism finally earns
 * its keep: each member says whether it routes by its own config file or
 * through the local proxy, which is the fact you need when one of them
 * isn't working. Config members drill in one more level for the full error
 * text; proxy members have nothing more to say than their hosts. */
export function GroupDetail({
  group,
  busy,
  onBack,
  onToggleGroup,
  onToggleTool,
  onSetDomain,
  onOpenTool,
}: {
  group: Group;
  busy: boolean;
  onBack: () => void;
  onToggleGroup: (id: string, on: boolean) => void;
  onToggleTool: (slug: string, routed: boolean) => void;
  onSetDomain: (slug: string, enabled: boolean) => void;
  onOpenTool: (slug: string) => void;
}) {
  const drifted = group.members.filter((m) => m.attention === "drifted");

  return (
    <div className="flex flex-col">
      <SubHeader title={group.name} onBack={onBack} />

      <div className="flex items-start gap-3 px-3.5 py-3.5">
        <div className="min-w-0 flex-1">
          <div className="text-[14px] font-semibold text-gc-ink">
            Route {group.name} through Gate
          </div>
          <div className="mt-0.5 text-[11.5px] leading-snug text-gc-ink-3">
            {group.routed} of {group.members.length} routing. This switch moves
            the whole group; each one below moves on its own.
          </div>
        </div>
        <Switch
          on={group.routed > 0}
          label={`Route ${group.name} through Gate`}
          busy={busy}
          onClick={() => onToggleGroup(group.id, group.routed === 0)}
        />
      </div>

      {drifted.length > 0 && (
        <div className="mx-3.5 mb-2 flex items-start gap-2.5 rounded bg-gc-sunken px-3 py-2.5">
          <Icon name="info" size={15} className="mt-px shrink-0 text-gc-ink-3" />
          <div className="min-w-0 flex-1 text-[11.5px] leading-snug text-gc-ink-2">
            {drifted.length === 1 ? `${drifted[0].name} has` : `${drifted.length} of these have`}{" "}
            a Gate setup written outside this app. The group switch leaves{" "}
            {drifted.length === 1 ? "it" : "them"} alone; open{" "}
            {drifted.length === 1 ? "it" : "each one"} to replace that setup.
          </div>
        </div>
      )}

      <SectionLabel>In this group</SectionLabel>
      <div className="flex flex-col border-t border-gc-line">
        {group.members.map((m) => {
          const drillable = m.kind === "config" && m.tool;
          return (
            <div
              key={m.key}
              className={`relative flex items-center gap-2.5 border-b border-gc-line px-3.5 py-2.5 ${
                drillable ? "transition hover:bg-gc-subtle" : ""
              }`}
            >
              {drillable && (
                <button
                  type="button"
                  onClick={() => onOpenTool(m.key)}
                  aria-label={`${m.name} details`}
                  className="absolute inset-0 cursor-pointer focus-visible:outline focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-gc-accent"
                />
              )}
              <div className="pointer-events-none relative min-w-0 flex-1">
                <div className="truncate text-[13px] font-medium text-gc-ink">{m.name}</div>
                <div className="mt-0.5 flex items-center gap-1.5">
                  <span className="shrink-0 rounded bg-gc-sunken px-1.5 py-px font-mono text-[10px] text-gc-ink-3">
                    {m.kind === "config" ? "config file" : "proxy"}
                  </span>
                  <span className="truncate font-mono text-[10px] text-gc-ink-3">
                    {m.kind === "proxy"
                      ? (m.domain?.hosts ?? []).join(" · ")
                      : m.tool?.default_upstream_url}
                  </span>
                </div>
              </div>
              <span className="pointer-events-none relative">
                <MemberPill member={m} />
              </span>
              <span className="relative">
                <Switch
                  on={m.routed}
                  label={`Route ${m.name} through Gate`}
                  busy={busy}
                  onClick={() => {
                    if (m.kind === "proxy") {
                      onSetDomain(m.key, !m.domain?.enabled);
                      return;
                    }
                    // Adopting a hand-written setup deserves its explanation
                    // and confirm, both of which live on the tool detail.
                    if (m.attention === "drifted" && !m.routed) onOpenTool(m.key);
                    else onToggleTool(m.key, !m.routed);
                  }}
                />
              </span>
              {drillable && (
                <span className="pointer-events-none relative">
                  <Icon name="chevronRight" size={14} stroke={2} className="text-gc-ink-4" />
                </span>
              )}
            </div>
          );
        })}
      </div>
    </div>
  );
}
