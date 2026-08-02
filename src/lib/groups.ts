import type { ProviderState, ProxyDomain, Tool } from "./api";

/**
 * Home's ledger groups everything routable by the model family it belongs to
 * (Claude, OpenAI, OpenRouter) instead of by mechanism (config file vs local
 * proxy). Three or four rows that stay that way as tools are installed, with
 * the mechanism kept for the group detail where it actually helps.
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
 * block it finds, so it can't sit under a single family. */
export const MULTI_PROVIDER_ID = "any-provider";

export type MemberAttention = "error" | "drifted" | "needs-trust" | null;

export interface GroupMember {
  /** Tool slug or domain slug - unique within a group. */
  key: string;
  /** How this one routes: its own config file, or the local proxy. */
  kind: "config" | "proxy";
  name: string;
  routed: boolean;
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
  routed: number;
}

function memberFromTool(tool: Tool): GroupMember {
  return {
    key: tool.slug,
    kind: "config",
    name: tool.name,
    routed: tool.status.kind === "connected",
    attention:
      tool.status.kind === "error"
        ? "error"
        : tool.status.kind === "drifted"
          ? "drifted"
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
    attention: domain.enabled && proxyOn && !caTrusted ? "needs-trust" : null,
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
        members.push(memberFromTool(tool));
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
    };
  });

  // Whatever the catalog didn't claim: the agent harnesses.
  const leftovers = installed
    .filter((t) => !claimed.has(t.slug))
    .map((t) => ({ ...memberFromTool(t), coversAllProviders: true }));
  if (leftovers.length > 0) {
    groups.push({
      id: MULTI_PROVIDER_ID,
      name: "Agent harnesses",
      switchLabel: "Route agent harnesses through Gate",
      blurb: "Tools that route every provider you’ve set up in them, not one model family.",
      members: leftovers,
      routed: leftovers.filter((m) => m.routed).length,
    });
  }

  return groups.filter((g) => g.members.length > 0);
}

/** "2 of 4 routing", plus whatever needs a human, named rather than counted
 * away: the row is a summary, but an exception should never hide inside it. */
export function groupSummary(group: Group): { count: string; exception: string | null } {
  const count = `${group.routed} of ${group.members.length} routing`;
  const errors = group.members.filter((m) => m.attention === "error");
  const drifted = group.members.filter((m) => m.attention === "drifted");
  const untrusted = group.members.filter((m) => m.attention === "needs-trust");
  const exception =
    errors.length > 0
      ? errors.length === 1
        ? `${errors[0].name} failed`
        : `${errors.length} failed`
      : untrusted.length > 0
        ? "certificate not trusted"
        : drifted.length > 0
          ? drifted.length === 1
            ? `${drifted[0].name} set up elsewhere`
            : `${drifted.length} set up elsewhere`
          : null;
  return { count, exception };
}
