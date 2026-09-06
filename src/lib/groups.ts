import type { ProviderState, ProxyDomain, Tool, Verdict } from "./api";
import type { Platform } from "./platform";

/**
 * Home's ledger groups everything routable by the model family it belongs to
 * (Claude, OpenAI, OpenRouter) instead of by mechanism (config file vs local
 * proxy), with the mechanism kept for the group detail where it actually helps.
 *
 * Sizing note for the next reader: up to six rows. Three are the families
 * above; the rest come from `LEFTOVER_GROUPS` - OpenClaw, Hermes and
 * Experimental (OpenCode + the environment channel). Those three used to be
 * one "Other tools" row, and the group that built it survives now only as the
 * catch-all for a tool nothing has placed (see docs/routing-architecture.md).
 *
 * Membership comes from the backend provider catalog (`tool_slugs` +
 * `domain_slugs`), never from `Tool.upstream_provider_name`: that field is
 * display prose, and for OpenCode and OpenClaw it is literally "your existing
 * providers", because those tools route whatever providers the user has
 * configured rather than one model family. Tools the catalog claims for no
 * provider are exactly those multi-provider tools, so they get their own
 * group rather than being wedged into a family they don't belong to.
 */

/** The catch-all for a tool the catalog claims for no provider and
 * `LEFTOVER_GROUPS` below does not name either.
 *
 * It used to hold all of them under one "Other tools" heading. It no longer
 * does: OpenClaw, Hermes and the experimental pair are named groups now, so
 * this group builds only for a tool nobody has placed - a registry and a
 * catalog momentarily out of step, or an integration added without a heading.
 * Kept rather than deleted precisely because that tool still needs a row. */
export const MULTI_PROVIDER_ID = "any-provider";

/**
 * The leftover tools, split into headings of their own.
 *
 * These are the tools the provider catalog claims for nobody: their provider
 * set is decided by the user's config, not by the tool, so there is no model
 * family to file them under. That used to make them one undifferentiated
 * "Other tools" list. The split is better on the one screen that matters,
 * the rail: a heading per tool means the row beneath it can be named for the
 * surface ("CLI") the way the family rows are, instead of having to carry the
 * product name and the surface at once.
 *
 * Order is the order the headings render in. A slug listed here that is not
 * installed simply contributes no member, and a group with no members is
 * dropped, so this list is a layout, not a claim about what is on the machine.
 */
const LEFTOVER_GROUPS: readonly {
  id: string;
  name: string;
  /** The group's name inside `switchLabel`'s sentence. */
  switchNoun: string;
  slugs: readonly string[];
  /** Proxy domains this heading claims, named one by one rather than swept.
   *
   * Sweeping every unclaimed domain would be wrong twice: the catalog carries
   * entries with no row today (`opencode`, the Zen/Go host), and a domain slug
   * can collide with a tool slug - `opencode` is both - which would put two
   * members under one key in the same group. Naming them keeps the ledger a
   * decision rather than a leftover of a leftover. */
  domainSlugs?: readonly string[];
  blurb?: string;
}[] = [
  { id: "openclaw", name: "OpenClaw", switchNoun: "OpenClaw", slugs: ["openclaw"] },
  { id: "hermes", name: "Hermes", switchNoun: "Hermes", slugs: ["hermes"] },
  {
    id: "experimental",
    name: "Experimental",
    // Lower-case in the sentence: "experimental tools" is a common noun, the
    // same rule "other tools" follows. The heading is a section, not a vendor.
    switchNoun: "experimental tools",
    // OpenCode and the environment channel travel together, and this is where
    // that becomes visible. OpenCode has no gateway setting of its own, so Gate
    // routes it with the machine's proxy variables - which is exactly what
    // "Terminal tools" is. Turning OpenCode on turns that on, and the prompt
    // that says so (`opencode-env` in `useRouting`) is only honest if the row it
    // names is on screen beside it.
    slugs: ["opencode", "env-proxy"],
    // `openai` is api.openai.com, and it is here rather than under OpenAI
    // because nothing OpenAI ships rides its switch. Codex is config-routed
    // through the relay, which resolves against the whole catalog rather than
    // the enabled set, so it routes whether this is on or off; the ChatGPT
    // desktop app is on chatgpt.com. Flipping this intercepts that host for any
    // system-proxy-honouring client, and the clients that depend on it are the
    // harnesses beside it - OpenClaw and Hermes blind-tunnel anything outside
    // the enabled catalog, so this switch is what lets Gate see their OpenAI
    // calls. `provider.rs` no longer lists the slug, which is what frees it.
    domainSlugs: ["openai"],
    blurb:
      "Routing here is still being proven out. Gate covers what it can and leaves everything else going where it always did.",
  },
];

/**
 * What each row is, in one sentence.
 *
 * The row labels are surface kinds now - "App", "Web", "CLI", "Proxy" - which
 * is what makes a family readable at a glance and useless in isolation: "Web"
 * under "Anthropic" is only a word until something says it means the claude.ai
 * tab. This is that something, and it is the reason the labels could be
 * shortened at all.
 *
 * Keyed by member key, so tool slugs and domain slugs share one namespace -
 * which they already do on the rail, where a row is one or the other and the
 * user cannot tell which. Copy rather than catalog data: the backend names the
 * surface it routes, and this says what the surface is to the person reading.
 *
 * A slug with no entry gets no sentence rather than a placeholder.
 */
export const MEMBER_DESCRIPTIONS: Readonly<Record<string, string>> = {
  anthropic: "The Claude desktop app and Cowork.",
  "claude-web": "Your Claude chats, in the browser tab.",
  "claude-code": "Claude Code in your terminal.",
  chatgpt: "Your ChatGPT Desktop App.",
  "chatgpt-apps":
    "Your ChatGPT conversations, in a browser tab, plus the tools Codex runs there.",
  codex: "Codex in your terminal.",
  openrouter:
    "Any app that goes through OpenRouter. Gate sees the traffic first, so you get its security and compression on the way.",
  openclaw: "OpenClaw in your terminal.",
  hermes: "Hermes in your terminal.",
  opencode: "The OpenCode editor.",
  "env-proxy": "Command line tools that follow your proxy settings.",
  // The one row whose subject is a host rather than a product, so its sentence
  // is the only place the host is written in the window UI - the rail and the
  // pane show it nowhere else. The label stays "OpenAI API" and the identifier
  // lands here, which is also the rule about mono: the popover prints
  // `api.openai.com` in a mono slot on this row already, and a sans label
  // repeating it would say it twice and set an identifier in body type.
  openai:
    "Anything on this machine that calls api.openai.com directly. Gate intercepts that host, so apps with no gateway setting of their own still route.",
};

/** The sentence for one row, or nothing where no copy exists for it. */
export function describeMember(key: string): string | undefined {
  return MEMBER_DESCRIPTIONS[key];
}

/**
 * What a proxy-routed surface cannot promise about an app that is already open.
 *
 * **Linux only, and the platform gate is the substance rather than caution.**
 * A proxy-routed app has no config file to re-read, so nothing here is the
 * `reopen_required` verdict - that one is measured, per tool, from a process
 * older than the last routing change. This is the other thing, and it is
 * genuinely different per platform:
 *
 * - **Windows** writes `AutoConfigURL` under HKCU and then pokes WinINET with
 *   `INTERNET_OPTION_SETTINGS_CHANGED`, precisely so running apps do not keep
 *   the stale settings (`proxy/system_proxy_windows.rs`).
 * - **macOS** sets the auto-proxy URL through `networksetup`, and CFNetwork and
 *   Chromium-based apps apply it to new connections as it changes.
 * - **Linux** has two channels and they disagree. GNOME's `org.gnome.system.proxy`
 *   keys are re-read live by anything on GLib's proxy resolver; the
 *   `environment.d` variables reach a process only at launch, and
 *   `proxy/system_proxy_linux.rs` says so in as many words - "already-running
 *   processes keep their environment until relaunched - nothing can change
 *   that".
 *
 * Which of the two channels a given app uses is not something Gate can see: it
 * cannot even list these processes (`AGENT_PROCESSES` knows three CLIs). So this
 * is written as advice and says that it is - a line that claimed to know would
 * be a reading with nothing behind it, which is the one thing this app does not
 * do with a routing state.
 */
export const PROXY_REOPEN_ADVICE = {
  title: "Apps already open may need reopening",
  body: "Gate routes these through your session's proxy settings. An app that reads those when it starts, rather than watching them, keeps using whatever was in force when it launched. Gate cannot see these apps, so this is advice rather than a reading.",
} as const;

/** The advice above, where it applies: a proxy-routed row, on Linux. */
export function proxyReopenAdvice(
  kind: GroupMember["kind"],
  platform: Platform,
): typeof PROXY_REOPEN_ADVICE | undefined {
  return kind === "proxy" && platform === "linux" ? PROXY_REOPEN_ADVICE : undefined;
}

export type MemberAttention =
  | "error"
  | "drifted"
  /**
   * Gate's configuration is in place and something the tool ranks higher
   * decides where its traffic goes (AG-674).
   *
   * Its own value rather than `drifted`, for the reason the whole state exists:
   * drift offers "let Gate manage this" and that repair is real, while here the
   * file Gate manages is already correct and the conflict is somewhere this app
   * does not write. Offering the same switch would move nothing and say the
   * problem was ours to fix.
   */
  | "overridden"
  | "needs-trust"
  | "master-off"
  /**
   * Nothing is known to be wrong, and nothing could be confirmed either.
   *
   * The ledger used to read `routed` off the tool's config: connected file plus
   * running engine meant routing. AG-570 rules that out - "a saved preference or
   * completed file write does not produce On or Off without verification" - so
   * `routed` now comes from the verdict sweep, and this is what a row says while
   * the sweep has not answered or could not conclude.
   *
   * One value rather than the verdict layer's five reasons, on purpose. The five
   * are drawn in the window shell, which has the width for "Not protected -
   * Reopen required"; this ledger is 360px and its job here is to stop claiming
   * a route it cannot support. Precision about *why* lives one shell up.
   */
  | "unverified"
  | null;

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
  /** What this row is, in one sentence - `describeMember`'s answer, carried
   * on the member so the rail, the pane and the family panel cannot disagree
   * about it. Absent where no copy exists for the slug.
   *
   * Load-bearing rather than decoration. The label beside it is a surface
   * kind ("App", "Web", "CLI"), which says nothing on its own; this is the
   * half that names the thing on the user's machine. */
  description?: string;
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
  /** This group's members route whatever providers the user configured in
   * them, so there is no one upstream vendor to caption it with.
   *
   * The rail captions a family with its members' `upstream_provider_name`
   * ("Anthropic" over the Claude rows). For these that field is the sentence
   * fragment "your existing providers", which is not a caption, so the
   * group's own name has to stand in. A property rather than an id
   * comparison because there are four such groups now and only one of them
   * is still called "Other tools". */
  multiProvider?: boolean;
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

function memberFromTool(
  tool: Tool,
  { proxyOn, verdicts }: { proxyOn: boolean; verdicts?: Map<string, Verdict> },
): GroupMember {
  // Intent, not flow: an overridden tool carries Gate's values in Gate's file,
  // which is exactly what the user asked for. Reading it as not-connected would
  // render the switch off and make clicking it turn off the setting they were
  // trying to turn on - this module's own opening bug, one state later.
  const connected = tool.status.kind === "connected" || tool.status.kind === "overridden";
  const verdict = verdicts?.get(tool.slug);
  // Verified, or not claimed. A config tool points at the loopback relay, and a
  // file naming that relay is not evidence anything is using it: the relay may
  // be down, the session may be dead server-side, or the process may predate the
  // write. So the sweep decides, and an unanswered sweep decides "no".
  //
  // `verdicts` is optional because a caller that has not run the sweep is a real
  // state (the first render), not a caller opting out - and the fallback is the
  // conservative one either way. What it must never fall back to is the config,
  // which is the claim AG-570 forbids.
  const routed = verdict ? verdict.state === "on" : false;
  return {
    key: tool.slug,
    kind: "config",
    name: tool.name,
    description: describeMember(tool.slug),
    routed,
    // Intent, which is the config: this is the switch's half of the split, and
    // `lib/groups.ts`'s own header documents what happens when the two are
    // conflated. Unchanged by any of the above on purpose.
    desired: connected,
    attention:
      tool.status.kind === "error"
        ? "error"
        : tool.status.kind === "drifted"
          ? "drifted"
          : tool.status.kind === "overridden"
            ? // Above master-off and unverified on purpose: those describe a
              // route that would carry this tool's traffic once something is
              // switched on, and this one says the traffic is not on our route
              // at all.
              "overridden"
            : // Master-off outranks the sweep's own vocabulary because it is the
            // better sentence for the same fact: the sweep would report a dead
            // relay as a connection problem, and "routing is off" is what the
            // user needs to hear.
            connected && !proxyOn
            ? "master-off"
            : connected && !routed
              ? "unverified"
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
    description: describeMember(domain.slug),
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
  opts: {
    proxyOn: boolean;
    caTrusted: boolean;
    /** The routing sweep, by slug. Omit only when it has not answered yet: a
     *  member with no verdict does not count as routing. */
    verdicts?: Map<string, Verdict>;
  },
): Group[] {
  const installed = tools.filter((t) => t.status.kind !== "not_installed");
  const routable = domains.filter((d) => d.supported);
  const claimed = new Set<string>();
  // Domains a family took, so a leftover heading naming the same slug cannot
  // draw it a second row. Tracked separately from `claimed` because tool and
  // domain slugs share no namespace guarantee - `opencode` is both.
  const claimedDomains = new Set<string>();

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
        claimedDomains.add(domain.slug);
        members.push(memberFromDomain(domain, opts));
      }
    }
    // After the cascaded domains, so a family reads "what the switch governs,
    // then the surface it deliberately leaves alone".
    for (const domain of routable) {
      if (provider.chat_domain_slugs.includes(domain.slug)) {
        claimedDomains.add(domain.slug);
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
  //
  // These used to be one "Other tools" row. They are headings of their own now
  // (`LEFTOVER_GROUPS`), for the reason the row labels changed at all: a rail
  // row reading "CLI" is only legible under a heading that says whose CLI, and
  // "Other tools" said the opposite - that the heading could not name it.
  // Anything neither the catalog nor that list places still lands in the
  // catch-all below, so no installed tool can fall off the ledger.
  const leftovers = new Map(
    installed
      .filter((t) => !claimed.has(t.slug))
      .map((t) => [
        t.slug,
        { ...memberFromTool(t, opts), coversAllProviders: true } as GroupMember,
      ]),
  );

  for (const spec of LEFTOVER_GROUPS) {
    const members: GroupMember[] = [];
    for (const slug of spec.slugs) {
      const member = leftovers.get(slug);
      if (!member) continue;
      leftovers.delete(slug);
      members.push(member);
    }
    // Domains after the tools, the same order a family draws them in.
    for (const slug of spec.domainSlugs ?? []) {
      if (claimedDomains.has(slug)) continue;
      const domain = routable.find((d) => d.slug === slug);
      if (!domain) continue;
      members.push(memberFromDomain(domain, opts));
    }
    if (members.length === 0) continue;
    groups.push({
      id: spec.id,
      name: spec.name,
      switchLabel: `Route ${spec.switchNoun} through Gate`,
      blurb: spec.blurb,
      multiProvider: true,
      members,
      routed: members.filter((m) => m.routed).length,
      desired: members.filter((m) => m.desired).length,
      // Nothing here is ever outside the cascade: these headings hold config
      // tools and plain proxy domains, and only a chat member - a session-cookie
      // surface, which belongs to a model family by definition - is excluded.
      cascadeDesired: members.filter((m) => m.desired).length,
    });
  }

  const unplaced = [...leftovers.values()];
  if (unplaced.length > 0) {
    groups.push({
      id: MULTI_PROVIDER_ID,
      // "Other tools", not "Agent harnesses". This is the label on a
      // `filter(t => !claimed.has(t.slug))`, and it surfaced as a family name on
      // the screen people read daily. PRODUCT.md's positioning says the UI's
      // nouns are tools and apps; nobody installs a harness, and it was the one
      // word on Home a first-timer could not map to anything on their machine.
      // The blurb below is where the category actually gets explained, which is
      // the right place for a definition the name should not have to carry.
      //
      // What reaches it has changed, and the name survived that: it is now the
      // tools `LEFTOVER_GROUPS` did not name, which in a shipped build should be
      // none. A tool arriving here is a heading someone forgot to add, and
      // "Other tools" is exactly the right thing to call it until they do.
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
      multiProvider: true,
      members: unplaced,
      routed: unplaced.filter((m) => m.routed).length,
      desired: unplaced.filter((m) => m.desired).length,
      // Config tools only, so nothing here is ever outside the cascade.
      cascadeDesired: unplaced.filter((m) => m.desired).length,
    });
  }

  return groups.filter((g) => g.members.length > 0);
}

/** Which kind of exception `groupSummary` found, so a row can give the sentence
 * its own ink instead of printing every severity in the same grey. Reality is
 * what this ledger is for, and it was losing the row to the switch beside it:
 * intent is one saturated indigo object, so reality has to speak in more than
 * one place to hold its own. */
export type GroupException =
  | "error"
  | "needs-trust"
  | "master-off"
  | "drifted"
  | "overridden"
  | "unverified";

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
  const overridden = group.members.filter((m) => m.attention === "overridden");
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
  if (overridden.length > 0) {
    // After drift and before "not verified": it is a known fault rather than an
    // absence of an answer, and of the two known ones drift is the fixable one,
    // so it keeps the row when both are present.
    return {
      count,
      exception:
        overridden.length === 1
          ? `${overridden[0].name} routed elsewhere`
          : `${overridden.length} routed elsewhere`,
      kind: "overridden",
    };
  }
  const unverified = group.members.filter((m) => m.attention === "unverified");
  if (unverified.length > 0) {
    // Last of the exceptions: every branch above names something known to be
    // wrong, and this one names the absence of an answer. A row that could
    // report a real fault must do that instead.
    return {
      count,
      exception:
        unverified.length === 1
          ? `${unverified[0].name} not verified`
          : `${unverified.length} not verified`,
      kind: "unverified",
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
    // An overridden member is left out for the same reason a drifted one is:
    // the family switch writes Gate's config, and here that config is already
    // written and already losing. Turning it on again is a no-op the user would
    // read as a fix.
    if (on) return !m.desired && m.attention !== "drifted" && m.attention !== "overridden";
    return m.desired;
  });
}
