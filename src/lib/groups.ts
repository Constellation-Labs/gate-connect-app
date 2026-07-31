import type { ProxyDomain, Tool } from "./api";

/**
 * Home's ledger groups everything routable by the model family it talks to
 * (Claude, OpenAI, OpenRouter) instead of by mechanism (config file vs local
 * proxy). Three rows that stay three as tools are installed, with the
 * mechanism kept for the detail screen where it actually helps.
 *
 * Grouping is derived here rather than read from the backend's provider
 * catalog on purpose: that catalog maps only Claude Code to Anthropic, so
 * OpenCode / OpenClaw / Hermes would have no home. A tool's own
 * `upstream_provider_name` is the honest family key, and an unrecognised
 * upstream still gets a group of its own rather than disappearing.
 */

/** Family id -> the name a user would recognise. */
const FAMILY_NAMES: Record<string, string> = {
  anthropic: "Claude",
  openai: "OpenAI",
  openrouter: "OpenRouter",
};

/** Stable ordering; unknown families sort after these, alphabetically. */
const FAMILY_ORDER = ["anthropic", "openai", "openrouter"];

export type MemberAttention = "error" | "drifted" | "needs-trust" | null;

export interface GroupMember {
  /** Tool slug or domain slug - unique within a group. */
  key: string;
  /** How this one routes: its own config file, or the local proxy. */
  kind: "config" | "proxy";
  name: string;
  routed: boolean;
  attention: MemberAttention;
  /** Present for config members; drives the per-tool detail screen. */
  tool?: Tool;
  /** Present for proxy members. */
  domain?: ProxyDomain;
}

export interface Group {
  id: string;
  name: string;
  members: GroupMember[];
  routed: number;
}

function familyId(raw: string): string {
  return raw.trim().toLowerCase().replace(/\s+/g, "-");
}

function familyName(id: string): string {
  return FAMILY_NAMES[id] ?? id.charAt(0).toUpperCase() + id.slice(1);
}

/**
 * Build the Home ledger. Not-installed tools and unsupported domains are left
 * out: the ledger lists what the user could actually route today.
 *
 * A proxy member only counts as routed when the engine is up *and* the
 * certificate is trusted, matching the "Partly routed" rule - an enabled
 * domain behind an untrusted certificate is not carrying traffic.
 */
export function buildGroups(
  tools: Tool[],
  domains: ProxyDomain[],
  { proxyOn, caTrusted }: { proxyOn: boolean; caTrusted: boolean },
): Group[] {
  const byFamily = new Map<string, GroupMember[]>();
  const push = (id: string, member: GroupMember) => {
    const list = byFamily.get(id);
    if (list) list.push(member);
    else byFamily.set(id, [member]);
  };

  for (const tool of tools) {
    if (tool.status.kind === "not_installed") continue;
    push(familyId(tool.upstream_provider_name), {
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
    });
  }

  for (const domain of domains) {
    if (!domain.supported) continue;
    push(familyId(domain.slug), {
      key: domain.slug,
      kind: "proxy",
      name: domain.display_name,
      routed: domain.enabled && proxyOn && caTrusted,
      attention: domain.enabled && proxyOn && !caTrusted ? "needs-trust" : null,
      domain,
    });
  }

  return [...byFamily.entries()]
    .map(([id, members]) => ({
      id,
      name: familyName(id),
      members,
      routed: members.filter((m) => m.routed).length,
    }))
    .sort((a, b) => {
      const ai = FAMILY_ORDER.indexOf(a.id);
      const bi = FAMILY_ORDER.indexOf(b.id);
      if (ai !== -1 && bi !== -1) return ai - bi;
      if (ai !== -1) return -1;
      if (bi !== -1) return 1;
      return a.name.localeCompare(b.name);
    });
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
