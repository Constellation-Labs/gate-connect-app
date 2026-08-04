import { useEffect, useState } from "react";
import type { Group, GroupMember } from "../lib/groups";
import type { AuthMode } from "../lib/api";
import { classifyError, type ClassifiedError } from "../lib/errors";
import { trackError } from "../lib/analytics";
import { SubHeader, SectionLabel, Switch, ErrorNote, IconButton, Button } from "../components/gc/ui";
import { MemberPill, memberPillLabel } from "../components/GroupPill";
import { secretStoreName, usePlatform, type Platform } from "../lib/platform";
import { Icon } from "../components/gc/Icon";

/** Host only, for the mono identifier slot. */
function hostOf(url: string | undefined): string {
  if (!url) return "";
  try {
    return new URL(url).host;
  } catch {
    return url;
  }
}

/** What a member's current state means, in plain language. This is the copy
 * that used to live on a separate tool screen; it belongs next to the row it
 * describes, not a level deeper.
 *
 * Takes the platform because two of these branches name the secret store, and
 * naming the wrong vault undoes the reassurance they exist to give. */
function explain(member: GroupMember, platform: Platform): string {
  if (member.attention === "master-off") {
    return member.kind === "proxy"
      ? `${member.name} is switched on, but routing is off, so nothing is going through Gate yet.`
      : `${member.name}'s config points at Gate, but routing is off, so it can't reach the gateway.`;
  }
  if (member.kind === "proxy") {
    return member.attention === "needs-trust"
      ? `${member.name} is switched on, but the local certificate isn't trusted yet, so its traffic isn't routing.`
      : member.routed
        ? `${member.name} has no gateway setting of its own, so Gate routes it through the local proxy.`
        : `${member.name} routes through Gate's local proxy once you switch it on.`;
  }
  switch (member.tool?.status.kind) {
    case "connected":
      return `${member.name}'s own config points at your Gate gateway. Requests carry the key from ${secretStoreName(platform)}; the key itself never lands in the config file.`;
    case "drifted":
      return `${member.name} has a Gate setup written outside this app. Switching it on replaces that configuration and manages the key from ${secretStoreName(platform)}.`;
    case "error":
      return `Gate Connect couldn't read ${member.name}'s routing state. The details below name the cause; fix that, then reopen this window from the menu bar to re-check.`;
    default:
      return `${member.name} is installed, but its config doesn't point at Gate. Switch it on and Gate Connect will write the config for you.`;
  }
}

/** The raw payload worth showing: a failure's whole message, or the evidence
 * behind a drift verdict. */
function rawDetail(member: GroupMember): string | null {
  const status = member.tool?.status;
  if (status?.kind === "error") return status.message;
  if (status?.kind === "drifted") return status.reason || null;
  return null;
}

/** A backend payload, with a way to get it out of the popover. Both callers
 * print something the user is expected to act on or forward - a failure
 * message naming a file and line, or the evidence behind a drift verdict - and
 * neither is selectable by hand in a 360px window with any dignity. `ErrorNote`
 * already made this argument for its own disclosure; this is the same rule for
 * the payload that sits out in the open. */
function RawDetail({ raw, alert }: { raw: string; alert: boolean }) {
  const [copied, setCopied] = useState(false);
  // Same cleanup as ErrorNote's: collapsing the row inside the 1.6s window used
  // to set state on an unmounted component, and collapsing the row is exactly
  // what the user does after copying.
  useEffect(() => {
    if (!copied) return;
    const t = setTimeout(() => setCopied(false), 1600);
    return () => clearTimeout(t);
  }, [copied]);
  return (
    <div
      role={alert ? "alert" : undefined}
      className="mt-2 rounded bg-gc-surface px-3 py-2.5 shadow-border"
    >
      <div className="font-mono text-[10.5px] leading-snug text-gc-ink-2 [overflow-wrap:anywhere]">
        {raw}
      </div>
      <button
        type="button"
        onClick={() => {
          void navigator.clipboard.writeText(raw).then(() => setCopied(true));
        }}
        className="mt-1.5 inline-flex items-center gap-1 text-[11px] font-medium text-gc-ink-3 transition hover:text-gc-ink"
      >
        <Icon name={copied ? "check" : "copy"} size={12} />
        {copied ? "Copied" : "Copy details"}
      </button>
    </div>
  );
}

/** One model family's fine grain, two levels deep and no further: the members
 * expand in place rather than pushing the user into a third screen. This is
 * also the only screen that shows the mechanism (config file vs proxy), which
 * is the fact you need when one member isn't working. */
export function GroupDetail({
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
  /** Rejects on failure so the row can surface it in place. */
  onToggleTool: (slug: string, routed: boolean) => Promise<void>;
  onSetDomain: (slug: string, enabled: boolean) => Promise<void>;
  /** The remedy for a needs-trust member, offered where the problem is named
   * rather than back on Home. Rejects on failure so the row can show it. */
  onTrustCa: () => Promise<void>;
  /** Whether the engine is running. A member can be switched on and still not
   * route, which is what the master-off state is. */
  proxyOn: boolean;
  /** The remedy for the master-off state, for the same reason `onTrustCa`
   * exists: naming a problem without offering the fix is half a screen. */
  onEnableRouting: () => void;
  /** So a gateway 401 sends an OAuth user to sign-in and a key user to the
   * key field, instead of naming a control their Settings does not render. */
  authMode?: AuthMode;
}) {
  const platform = usePlatform();
  const [openKey, setOpenKey] = useState<string | null>(null);
  const [error, setError] = useState<ClassifiedError | null>(null);
  const [changed, setChanged] = useState<string | null>(null);
  // Adopting a hand-written setup replaces someone's config, so the first
  // flip arms a confirm instead of writing.
  const [confirmingAdopt, setConfirmingAdopt] = useState<string | null>(null);
  const drifted = group.members.filter((m) => m.attention === "drifted");
  const errored = group.members.filter((m) => m.attention === "error");

  function toggleOpen(key: string) {
    setOpenKey((k) => (k === key ? null : key));
    setError(null);
  }

  /** Trusting can fail (a cancelled admin prompt is the common case), so the
   * failure belongs next to the button that caused it, exactly like a member
   * toggle failure. */
  async function trustFromRow(member: GroupMember) {
    setError(null);
    try {
      await onTrustCa();
    } catch (e) {
      setError(classifyError(e, "trust_ca"));
      setOpenKey(member.key);
    }
  }

  async function toggleMember(member: GroupMember) {
    setError(null);
    if (member.kind === "config" && !member.desired && member.attention === "drifted") {
      if (confirmingAdopt !== member.key) {
        setConfirmingAdopt(member.key);
        setOpenKey(member.key);
        return;
      }
    }
    setConfirmingAdopt(null);
    try {
      if (member.kind === "proxy") await onSetDomain(member.key, !member.desired);
      else await onToggleTool(member.key, !member.desired);
      setChanged(member.key);
    } catch (e) {
      trackError(e, "connect", { tool: member.key });
      setError(classifyError(e, "connect", authMode));
      setOpenKey(member.key);
    }
  }

  return (
    <div className="flex flex-col">
      <SubHeader title={group.name} onBack={onBack} />

      <div className="flex items-start gap-3 px-3.5 py-3.5">
        <div className="min-w-0 flex-1">
          {/* aria-hidden: this is verbatim the adjacent switch's accessible
              name, so leaving it exposed made AT read the same sentence
              twice in a row. */}
          <div aria-hidden className="text-[14px] font-semibold text-gc-ink">
            {group.switchLabel}
          </div>
          <div className="mt-0.5 text-[11.5px] leading-snug text-gc-ink-3">
            {group.blurb} {group.routed} of {group.members.length} routing.
          </div>
        </div>
        <Switch
          on={group.desired > 0}
          label={group.switchLabel}
          busy={busy}
          onClick={() => onToggleGroup(group.id, group.desired === 0)}
        />
      </div>

      {group.desired > 0 && !proxyOn && (
        <div className="mx-3.5 mb-2 flex items-center gap-2.5 rounded bg-gc-sunken px-3 py-2.5">
          <Icon name="info" size={15} className="shrink-0 text-gc-ink-3" />
          <div className="min-w-0 flex-1 text-[11.5px] leading-snug text-gc-ink-2">
            Switched on, but routing is off, so nothing here is going through
            Gate.
          </div>
          <Button variant="accent" size="sm" disabled={busy} onClick={onEnableRouting}>
            Turn on routing
          </Button>
        </div>
      )}

      {errored.length > 0 && (
        // The only member state with no banner, and the most severe one. The
        // two below it announce a setup the user chose and a switch they can
        // flip; a failure announces neither, and it was the one the screen let
        // the user find for themselves. Error wash with the colour on the icon
        // and the sentence in ink, per the Wash-First rule.
        <div className="mx-3.5 mb-2 flex items-start gap-2.5 rounded bg-gc-error-wash px-3 py-2.5">
          <Icon name="info" size={15} className="mt-px shrink-0 text-gc-error" />
          <div className="min-w-0 flex-1 text-[11.5px] leading-snug text-gc-ink-2">
            {errored.length === 1
              ? `${errored[0].name} isn’t reporting its routing state, so Gate Connect can’t tell whether it is going through Gate. Open it below for what went wrong.`
              : `${errored.length} of these aren’t reporting their routing state, so Gate Connect can’t tell whether they are going through Gate. Open each below for what went wrong.`}
          </div>
        </div>
      )}

      {drifted.length > 0 && (
        <div className="mx-3.5 mb-2 flex items-start gap-2.5 rounded bg-gc-sunken px-3 py-2.5">
          <Icon name="info" size={15} className="mt-px shrink-0 text-gc-ink-3" />
          <div className="min-w-0 flex-1 text-[11.5px] leading-snug text-gc-ink-2">
            {drifted.length === 1 ? `${drifted[0].name} has` : `${drifted.length} of these have`}{" "}
            a Gate setup written outside this app. The group switch leaves{" "}
            {drifted.length === 1 ? "it" : "them"} alone; switch{" "}
            {drifted.length === 1 ? "it" : "each"} on below to replace that setup.
          </div>
        </div>
      )}

      <SectionLabel>In this group</SectionLabel>
      <div className="flex flex-col border-t border-gc-line">
        {group.members.map((member) => {
          const open = openKey === member.key;
          const raw = rawDetail(member);
          return (
            <div key={member.key} className="border-b border-gc-line">
              {/* One hit area for the whole row, identifier line included.
                  The stretch button used to cover only the upper line, so the
                  bottom 40% of a row that highlights as one block did nothing
                  when clicked - and on Home the equivalent row is clickable
                  end to end. */}
              <div
                className={`relative transition ${
                  open ? "bg-gc-subtle" : "hover:bg-gc-subtle"
                }`}
              >
                <button
                  type="button"
                  onClick={() => toggleOpen(member.key)}
                  aria-expanded={open}
                  aria-label={`${member.name} details`}
                  className="absolute inset-0 cursor-pointer focus-visible:outline focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-gc-accent"
                />
                <div className="relative flex items-center gap-2.5 px-3.5 py-2.5">
                  <div className="pointer-events-none relative min-w-0 flex-1">
                    <div className="truncate text-[13px] font-medium text-gc-ink">{member.name}</div>
                  </div>
                  <span className="pointer-events-none relative">
                    <MemberPill member={member} />
                  </span>
                  <span className="relative">
                    {/* `desired`, not `routed`: the switch is the user's intent.
                        Driving it from `routed` meant an enabled domain behind
                        an untrusted certificate rendered off, and clicking it
                        turned the domain off while the switch never moved. */}
                    {/* The switch reports intent, so it can read "on" while the
                        pill beside it says the traffic isn't flowing. Point it at
                        the same description so a screen reader hears both. */}
                    <Switch
                      on={member.desired}
                      label={`Route ${member.name} through Gate`}
                      describedBy={`member-state-${member.key}`}
                      busy={busy}
                      onClick={() => void toggleMember(member)}
                    />
                    <span id={`member-state-${member.key}`} className="sr-only">
                      {memberPillLabel(member)}
                    </span>
                  </span>
                  <span className="pointer-events-none relative">
                    <Icon
                      name="chevronRight"
                      size={14}
                      stroke={2}
                      className={`text-gc-ink-4 transition-transform ${open ? "rotate-90" : ""}`}
                    />
                  </span>
                </div>

              {/* Its own full-width line, below the name/pill/switch row. In
                  that row the host shared width with the pill, so the wider
                  the pill the shorter the identifier: "Set up elsewhere" left
                  49px and rendered "api.ope…", "Waiting on routing" left 44px
                  and rendered "api.an…". The identifier was cut hardest in
                  exactly the two states that report a problem. */}
              <div className="pointer-events-none relative flex items-center gap-1.5 px-3.5 pb-2">
                <span className="shrink-0 rounded bg-gc-sunken px-1.5 py-px font-mono text-[10.5px] text-gc-ink-3">
                  {member.kind === "config" ? "config file" : "proxy"}
                </span>
                {/* Mono is for identifiers only. A harness has no single
                    upstream to name - its `default_upstream_url` is a
                    placeholder constant, not what it actually routes - so say
                    so in prose rather than print a host that lies. */}
                {member.coversAllProviders ? (
                  <span className="truncate text-[10.5px] text-gc-ink-3">all your providers</span>
                ) : (
                  <span className="truncate font-mono text-[10.5px] text-gc-ink-3">
                    {member.kind === "proxy"
                      ? (member.domain?.hosts ?? []).join(" · ")
                      : hostOf(member.tool?.default_upstream_url)}
                  </span>
                )}
              </div>
              </div>

              {open && (
                <div className="bg-gc-subtle px-3.5 pb-3">
                  <p className="text-[11.5px] leading-snug text-gc-ink-2">
                    {explain(member, platform)}
                  </p>

                  {/* The remedy belongs where the problem is named. Without
                      this the user reads "the certificate isn't trusted yet"
                      and has to navigate back to Home to act on it. */}
                  {member.attention === "needs-trust" && (
                    <Button
                      variant="accent"
                      size="sm"
                      className="mt-2.5"
                      disabled={busy}
                      onClick={() => void trustFromRow(member)}
                    >
                      Trust certificate
                    </Button>
                  )}

                  {/* Same reasoning as the Trust button above. This state was
                      introduced with prose only, so the one attention state
                      most reachable by accident was also the only one with no
                      way out of the screen. */}
                  {/* Suppressed when the group banner above already offers it:
                      with two members expanded the screen showed three
                      identical "Turn on routing" buttons for one action. The
                      remedy-travels-with-the-problem rule needs this clause. */}
                  {member.attention === "master-off" && !(group.desired > 0 && !proxyOn) && (
                    <Button
                      variant="accent"
                      size="sm"
                      className="mt-2.5"
                      disabled={busy}
                      onClick={onEnableRouting}
                    >
                      Turn on routing
                    </Button>
                  )}

                  {raw && <RawDetail raw={raw} alert={member.attention === "error"} />}

                  {confirmingAdopt === member.key && (
                    <div className="mt-2 rounded bg-gc-surface p-3 shadow-border">
                      <div className="text-[11.5px] leading-snug text-gc-ink-2">
                        Replace {member.name}&rsquo;s existing Gate setup? Switching it
                        off later restores {member.name}&rsquo;s own settings.
                      </div>
                      {/* One destructive grammar across the app. This confirm
                          overwrites a config the user wrote by hand - the most
                          destructive of the three confirms - and it wore the
                          lightest treatment: an indigo text link beside a grey
                          one. */}
                      <div className="mt-2.5 flex items-center gap-2">
                        <Button
                          variant="danger"
                          size="sm"
                          disabled={busy}
                          onClick={() => void toggleMember(member)}
                        >
                          Replace setup
                        </Button>
                        <Button
                          variant="secondary"
                          size="sm"
                          disabled={busy}
                          onClick={() => setConfirmingAdopt(null)}
                        >
                          Cancel
                        </Button>
                      </div>
                    </div>
                  )}

                  {error && openKey === member.key && (
                    <ErrorNote error={error} className="mt-2 bg-gc-surface shadow-border" />
                  )}

                  {/* Only when the change actually put traffic in flight.
                      Closing and reopening the app changes nothing while the
                      engine is down, so the advice would be busywork. */}
                  {changed === member.key && !error && member.routed && (
                    <div
                      role="status"
                      className="mt-2 flex items-center gap-2.5 rounded bg-gc-highlight px-3 py-2.5 shadow-border"
                    >
                      <Icon name="refresh" size={15} className="shrink-0 text-gc-ink" />
                      <div className="min-w-0 flex-1 text-[12px] font-medium leading-snug text-gc-ink">
                        <span className="font-semibold">Close {member.name}</span> to
                        apply the change; it picks this up the next time you open it.
                      </div>
                      <IconButton
                        icon="x"
                        size={13}
                        onClick={() => setChanged(null)}
                        aria-label="Dismiss restart hint"
                      />
                    </div>
                  )}
                </div>
              )}
            </div>
          );
        })}
      </div>
    </div>
  );
}
