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

/** Other tools: the tools whose provider set is decided by the user's
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
  /** A chat-protocol member: shown under its family, never flipped by the
   * family switch. These intercept a session-cookie surface (claude.ai,
   * chatgpt.com's conversation turn) instead of a key-brokered API, so
   * switching one on stays a per-row act. Mirrors the backend's split between
   * `chat_domain_slugs` and `proxy_domain_slugs`. */
  chat?: boolean;
}

export interface Group {
  id: string;
  name: string;
  /** The group's name inside a sentence. Family names are proper nouns and
   * stay capitalised; "other tools" is a common noun and must not. */
  switchLabel: string;
  /** What the family covers, shown under its name on the family panel, and
   * only where the name does not already say it.
   *
   * Present for the multi-provider group alone. It carried a line for every
   * family until 2026-08-10 and none of them were ever rendered: the field was
   * introduced to hold the definition that "Agent harnesses" used to carry in
   * its name, and the definition then went nowhere for two rounds while the
   * only description of these tools in the whole UI was the 25-character
   * identifier slot on a member row. The per-family lines were dropped rather
   * than shown, because "Everything that talks to Anthropic." under an h1
   * reading "Anthropic" is the same fact twice. "Other tools" is the one family
   * named by exclusion, so it is the one that owes the user a sentence. */
  blurb?: string;
  members: GroupMember[];
  /** How many members are actually carrying traffic. Drives the pill. */
  routed: number;
  /** How many are switched on. Drives the switch, for the same
   * intent-vs-flow reason as `GroupMember.desired`. */
  desired: number;
  /** How many of the members the family switch actually governs are switched
   * on - `desired` minus the chat members. It drives that switch, which
   * `desired` cannot once chat rows exist: a chat member switched on alone
   * would render the family switch "on" while everything it can flip is off,
   * and clicking it would then ask to turn off a set that is already off,
   * leaving the switch stuck on. Reality (`routed`) and the count still speak
   * for every member, including the chat ones. */
  cascadeDesired: number;
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
 *
 * Membership comes from two catalog fields, and the difference between them is
 * the whole reason there are two. `provider.domain_slugs` is what the family
 * switch cascades over; `provider.chat_domain_slugs` is the family's
 * chat-protocol surfaces (`claude-web`, `chatgpt-apps`), which get a row and a
 * switch of their own here but must stay out of that cascade - they intercept
 * the user's session cookie rather than a brokered key, so enabling "Claude"
 * must never start routing claude.ai as a side effect. Visibility used to ride
 * on `domain_slugs` alone, which is why those two were invisible; giving
 * visibility its own field is what lets them be shown without joining the
 * cascade. `App.tsx`'s `setGroupRouted` and `FamilyPanel`'s switch both honour
 * the split via `GroupMember.chat` / `Group.cascadeDesired`.
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
    // After the cascaded domains, so a family reads "what the switch governs,
    // then the surface it deliberately leaves alone".
    for (const domain of routable) {
      if (provider.chat_domain_slugs.includes(domain.slug)) {
        members.push({ ...memberFromDomain(domain, opts), chat: true });
      }
    }
    return {
      id: provider.slug,
      name: provider.display_name,
      switchLabel: `Route ${provider.display_name} through Gate`,
      members,
      routed: members.filter((m) => m.routed).length,
      desired: members.filter((m) => m.desired).length,
      cascadeDesired: members.filter((m) => m.desired && !m.chat).length,
    };
  });

  // Whatever the catalog didn't claim: the tools that route every provider
  // configured in them rather than one model family.
  const leftovers = installed
    .filter((t) => !claimed.has(t.slug))
    .map((t) => ({ ...memberFromTool(t, opts), coversAllProviders: true }));
  if (leftovers.length > 0) {
    groups.push({
      id: MULTI_PROVIDER_ID,
      // "Other tools", not "Agent harnesses". This is the label on a
      // `filter(t => !claimed.has(t.slug))`, and it surfaced as a family name on
      // the screen people read daily. PRODUCT.md's positioning says the UI's
      // nouns are tools and apps; nobody installs a harness, and it was the one
      // word on Home a first-timer could not map to anything on their machine.
      // The blurb below is where the category actually gets explained, which is
      // the right place for a definition the name should not have to carry.
      name: "Other tools",
      switchLabel: "Route other tools through Gate",
      // Which providers, and which not. The old line said these tools "route
      // every provider you've set up in them", which is the reading the code
      // does not support and the more alarming of the two a user might take: it
      // promises Gate stands in front of everything they configured. It does
      // not. OpenCode repoints only providers that are both on Gate's known
      // list and covered by the proxy catalog, and skips the rest at connect
      // time because the relay would 403 them; OpenClaw and Hermes do no
      // provider discovery at all and let the enabled catalog domains decide
      // what the engine intercepts, blind-tunnelling everything else. Three
      // mechanisms, one user-visible boundary: Gate takes what it covers and
      // leaves the rest alone. The second sentence is the one that matters to
      // someone running a local model.
      blurb:
        "Tools that talk to several providers, not one model family. Gate routes the ones it covers; anything else, including a local model, keeps going where it always did.",
      members: leftovers,
      routed: leftovers.filter((m) => m.routed).length,
      desired: leftovers.filter((m) => m.desired).length,
      // Config tools only, so nothing here is ever outside the cascade.
      cascadeDesired: leftovers.filter((m) => m.desired).length,
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

/**
 * The members a family switch may act on, already filtered to the ones the
 * change would actually move.
 *
 * Three rules, each of which cost something to learn:
 *
 * - **Chat members never ride a family switch.** They intercept a session-cookie
 *   surface (claude.ai, the ChatGPT app's own turn) rather than a key-brokered
 *   API, so routing one is a deliberate per-row act. This mirrors the backend,
 *   which keeps those slugs out of `proxy_domain_slugs` for the same reason.
 * - **A drifted member is never switched on by a family.** Its config was written
 *   by hand, and adopting it is a decision that belongs to the review dialog, not
 *   to a switch two levels up. Turning *off* is unaffected: disconnecting
 *   restores what was there.
 * - **Members already in the target state are left alone**, so a family switch
 *   does not rewrite a config that already says the right thing.
 *
 * Returned rather than applied, so both shells share the rules and each keeps
 * its own error handling. The caller still has to trust the CA before the first
 * command: a config member's connect auto-enables the engine, and the system
 * dialog belongs ahead of the loop rather than sprung from member three.
 */
export function cascadeTargets(group: Group, on: boolean): GroupMember[] {
  return group.members.filter((m) => {
    if (m.chat) return false;
    if (on) return !m.desired && m.attention !== "drifted";
    return m.desired;
  });
}
