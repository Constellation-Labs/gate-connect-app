import type { ProviderState, ProxyDomain, Tool } from "./api";

/**
 * Home's ledger groups everything routable by the model family it belongs to
 * (Claude, OpenAI, OpenRouter) instead of by mechanism (config file vs local
 * proxy), with the mechanism kept for the group detail where it actually helps.
 *
 * Sizing note for the next reader: up to four rows. The fourth is the
 * multi-provider group below, which was dormant while every agent harness was
 * hidden and is live again now that OpenCode, OpenClaw and Hermes are listed
 * (see docs/routing-architecture.md).
 *
 * Membership comes from the backend provider catalog (`tool_slugs` +
 * `domain_slugs`), never from `Tool.upstream_provider_name`: that field is
 * display prose, and for OpenCode and OpenClaw it is literally "your existing
 * providers", because those tools route whatever providers the user has
 * configured rather than one model family. Tools the catalog claims for no
 * provider are exactly those multi-provider tools, so they get their own
 * group rather than being wedged into a family they don't belong to.
 */

/** Agent harnesses: the tools whose provider set is decided by the user's
 * config, not by the tool. Connecting one rewrites every well-known provider
 * block it finds, so it can't sit under a single family.
 *
 * Live: OpenCode, OpenClaw and Hermes are all listed, so this group builds
 * whenever at least one of them is installed. It was dormant for a while and
 * the logic was kept against exactly this moment. */
export const MULTI_PROVIDER_ID = "any-provider";

export type MemberAttention = "error" | "drifted" | "needs-trust" | "master-off" | null;

export interface GroupMember {
  /** Tool slug or domain slug - unique within a group. */
  key: string;
  /** How this one routes: its own config file, or the local proxy. */
  kind: "config" | "proxy";
  name: string;
  /** Traffic is actually flowing through Gate right now. Drives the pill.
   * Strictly narrower than `desired`: a member can be switched on and still
   * not be routing, because the master is off or the certificate is not
   * trusted. */
  routed: boolean;
  /** What the user asked for: the persisted `enabled` / connected value.
   * Drives the switch.
   *
   * These were one field, and conflating them made the switch destructive.
   * With an untrusted certificate an enabled domain has `routed === false`,
   * so the switch rendered off; clicking it sent `!enabled === false` and
   * turned off the setting the user was trying to turn on, without the
   * switch ever moving. Display state is `routed`; intent is `desired`. */
  desired: boolean;
  attention: MemberAttention;
  /** Present for config members. */
  tool?: Tool;
  /** Present for proxy members. */
  domain?: ProxyDomain;
  /** This tool routes every provider configured in it, so there is no one
   * upstream host to name for it. */
  coversAllProviders?: boolean;
}

export interface Group {
  id: string;
  name: string;
  /** The group's name inside a sentence. Family names are proper nouns and
   * stay capitalised; "agent harnesses" is a common noun and must not. */
  switchLabel: string;
  /** One line on what the family covers, shown on its detail screen. */
  blurb: string;
  members: GroupMember[];
  /** How many members are actually carrying traffic. Drives the pill. */
  routed: number;
  /** How many are switched on. Drives the switch, for the same
   * intent-vs-flow reason as `GroupMember.desired`. */
  desired: number;
}

function memberFromTool(tool: Tool, { proxyOn }: { proxyOn: boolean }): GroupMember {
  const connected = tool.status.kind === "connected";
  return {
    key: tool.slug,
    kind: "config",
    name: tool.name,
    // A config tool points at the loopback relay. With the master off that
    // relay is dead, so the tool is broken, not routed - the same reasoning
    // that already gated proxy members. Master-off disconnects tools now, but
    // the sweep is best-effort per tool, so a tool that fails to disconnect
    // must not keep reporting itself as routing.
    routed: connected && proxyOn,
    desired: connected,
    attention:
      tool.status.kind === "error"
        ? "error"
        : tool.status.kind === "drifted"
          ? "drifted"
          : connected && !proxyOn
            ? "master-off"
            : null,
    tool,
  };
}

function memberFromDomain(
  domain: ProxyDomain,
  { proxyOn, caTrusted }: { proxyOn: boolean; caTrusted: boolean },
): GroupMember {
  return {
    key: domain.slug,
    kind: "proxy",
    name: domain.display_name,
    // An enabled domain behind an untrusted certificate is not carrying
    // traffic, so it does not count as routed - same rule as the header's
    // "Partly routed".
    routed: domain.enabled && proxyOn && caTrusted,
    desired: domain.enabled,
    attention: domain.enabled && proxyOn && !caTrusted
      ? "needs-trust"
      : domain.enabled && !proxyOn
        ? "master-off"
        : null,
    domain,
  };
}

/**
 * Build the Home ledger. Not-installed tools and unsupported domains are left
 * out: the ledger lists what could actually route today.
 */
export function buildGroups(
  providers: ProviderState[],
  tools: Tool[],
  domains: ProxyDomain[],
  opts: { proxyOn: boolean; caTrusted: boolean },
): Group[] {
  const installed = tools.filter((t) => t.status.kind !== "not_installed");
  const routable = domains.filter((d) => d.supported);
  const claimed = new Set<string>();

  const groups: Group[] = providers.map((provider) => {
    const members: GroupMember[] = [];
    for (const tool of installed) {
      if (provider.tool_slugs.includes(tool.slug)) {
        claimed.add(tool.slug);
        members.push(memberFromTool(tool, opts));
      }
    }
    for (const domain of routable) {
      if (provider.domain_slugs.includes(domain.slug)) {
        members.push(memberFromDomain(domain, opts));
      }
    }
    return {
      id: provider.slug,
      name: provider.display_name,
      switchLabel: `Route ${provider.display_name} through Gate`,
      blurb: `Everything that talks to ${provider.display_name}.`,
      members,
      routed: members.filter((m) => m.routed).length,
      desired: members.filter((m) => m.desired).length,
    };
  });

  // Whatever the catalog didn't claim: the agent harnesses.
  const leftovers = installed
    .filter((t) => !claimed.has(t.slug))
    .map((t) => ({ ...memberFromTool(t, opts), coversAllProviders: true }));
  if (leftovers.length > 0) {
    groups.push({
      id: MULTI_PROVIDER_ID,
      name: "Agent harnesses",
      switchLabel: "Route agent harnesses through Gate",
      blurb: "Tools that route every provider you’ve set up in them, not one model family.",
      members: leftovers,
      routed: leftovers.filter((m) => m.routed).length,
      desired: leftovers.filter((m) => m.desired).length,
    });
  }

  return groups.filter((g) => g.members.length > 0);
}

/** Which kind of exception `groupSummary` found, so a row can give the sentence
 * its own ink instead of printing every severity in the same grey. Reality is
 * what this ledger is for, and it was losing the row to the switch beside it:
 * intent is one saturated indigo object, so reality has to speak in more than
 * one place to hold its own. */
export type GroupException = "error" | "needs-trust" | "master-off" | "drifted";

/** "2 of 4 routing", plus whatever needs a human, named rather than counted
 * away: the row is a summary, but an exception should never hide inside it. */
export function groupSummary(group: Group): {
  count: string;
  exception: string | null;
  kind: GroupException | null;
} {
  // When there is an exception, the count is the half that survives
  // truncation at 360px and the exception is the half that gets cut - the
  // wrong way round, since the pill already answers "is this routing?".
  // Callers render `count` only when `exception` is null.
  const count = `${group.routed} of ${group.members.length} routing`;
  const errors = group.members.filter((m) => m.attention === "error");
  const drifted = group.members.filter((m) => m.attention === "drifted");
  const untrusted = group.members.filter((m) => m.attention === "needs-trust");
  const masterOff = group.members.filter((m) => m.attention === "master-off");
  if (errors.length > 0) {
    return {
      count,
      exception: errors.length === 1 ? `${errors[0].name} failed` : `${errors.length} failed`,
      kind: "error",
    };
  }
  if (untrusted.length > 0) {
    return { count, exception: "certificate not trusted", kind: "needs-trust" };
  }
  if (masterOff.length > 0) {
    return { count, exception: "waiting on routing", kind: "master-off" };
  }
  if (drifted.length > 0) {
    return {
      count,
      exception:
        drifted.length === 1
          ? `${drifted[0].name} set up elsewhere`
          : `${drifted.length} set up elsewhere`,
      kind: "drifted",
    };
  }
  return { count, exception: null, kind: null };
}
