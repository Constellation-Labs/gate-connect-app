import type {
  ClientId,
  Credential,
  ProxyDomain,
  ProxyState,
  Scope,
  Tool,
  Verdict,
} from "./api";
import { browserScopeNote, trustStoreName, type Platform } from "./platform";

/**
 * Home's ledger groups everything routable by the CLIENT it is aimed at - the
 * program on the user's machine whose traffic the row exists to route - rather
 * than by the vendor whose models that program talks to.
 *
 * Grouping by vendor is what this file used to do, and the leftovers were the
 * evidence against it. A vendor heading cannot file a tool that routes whatever
 * providers the user configured in it, so OpenClaw, Hermes and an "Experimental"
 * pair each grew a heading of their own, plus an `any-provider` catch-all whose
 * whole job was to catch what the taxonomy could not place. Every row answers
 * `client` now, so nothing can fall off the ledger and the catch-all is gone by
 * construction.
 *
 * Sizing note for the next reader: up to eight rows, one per `ClientId` that has
 * a member. Two of them are Anthropic's (Claude Code and Claude Desktop) where
 * there used to be one, which is the visible cost of the change and the point of
 * it: those are two programs, they are switched on separately, and a heading
 * reading "Anthropic" over both said otherwise.
 *
 * Membership comes from each row's own `client` field - `Tool.client` and
 * `ProxyDomain.client`, both from the backend catalog. It does NOT come from
 * `ProviderState`, which still exists for the vendor-shaped things that are
 * genuinely vendor-shaped (`provider::enable`, the CLI), nor from
 * `Tool.upstream_provider_name`, which is display prose and is literally "your
 * existing providers" for the multi-provider tools.
 */


/**
 * What each row is, in one sentence.
 *
 * The row labels are surface kinds - "API", "Chat", "CLI" - which is what makes
 * a group readable at a glance and useless in isolation: "Chat" under "Claude
 * Desktop" is only a word until something says it means claude.ai. This is that
 * something, and it is the reason the labels can be one word at all.
 *
 * Keyed by member key, so tool slugs and domain slugs share one namespace -
 * which they already do on the rail, where a row is one or the other and the
 * user cannot tell which. Copy rather than catalog data: the backend names the
 * surface it routes, and this says what the surface is to the person reading.
 *
 * A slug with no entry gets no sentence rather than a placeholder.
 */
export const MEMBER_DESCRIPTIONS: Readonly<Record<string, string>> = {
  anthropic: "The Claude desktop app's model calls, on api.anthropic.com.",
  // Both clients, because the row covers both and the heading above it now
  // says "Claude Desktop". This line used to read "Your Claude chats, in the
  // browser tab", which was wrong in the direction that cost a support thread:
  // the entry is `claude.ai`, matched on host, so it covers the desktop app's
  // chat turns AND a browser tab - and it covers the app MORE fully, since
  // `rules_for_client` narrows only the browser to the completion path.
  "claude-web": "Your Claude chats on claude.ai - in the desktop app, and in a browser tab.",
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

/**
 * What a browser that was open across the *first* enable cannot know yet.
 *
 * A different fact from [`PROXY_REOPEN_ADVICE`], and the two must not be merged:
 * that one is about the proxy pointer, which a browser on the desktop's own
 * settings re-reads live, so it would tell a Firefox user to restart for no
 * reason. This is about the trust store, which is read once at process start.
 * `ca_linux.rs` installs the CA into the per-user NSS databases Chromium reads
 * and into the system bundle Firefox picks up through p11-kit, and neither is
 * consulted again by a running process - so a browser that was open when the CA
 * landed fails every intercepted host with `ERR_CERT_AUTHORITY_INVALID` and
 * looks like Gate breaking the web rather than like a step left to do.
 *
 * Linux-only for the same reason as the advice above, and the same kind of
 * reason: macOS trusts in the login keychain and Windows in the per-user root
 * store, both of which the system trust evaluation re-reads for a running
 * process. Naming browsers there would be caution rather than mechanism. This
 * is mechanism.
 *
 * Said once, on the transition, not standing: it is only true of processes older
 * than the trust change, and a note that stayed up would keep telling a user who
 * has already reopened everything to reopen it. The shell drives it off
 * `ca_trusted` going false to true.
 *
 * **Which sentence it says is a reading, not a guess.** `nss` is
 * `ProxyState.ca_nss_trust`, recorded by the write itself, and each of its
 * values wants something different from the user - which is why it is not a
 * boolean. `tools_missing`: nothing was written anywhere, and installing a
 * package is the fix. `write_failed`: `certutil` is there and a store refused,
 * so a package install changes nothing and the report is what names the store.
 * `not_written`: the stores answered and nobody has put the CA in them, so a
 * retry is the fix and there is nothing to read in the report. `trusted`, or no
 * reading at all, leaves the reopen - the one thing Gate genuinely cannot
 * check, because it cannot see a browser process.
 *
 * The first version of this said "false means install a package". That is wrong
 * for a locked or unwritable store, and it withheld the reopen sentence from a
 * user whose other stores took the CA and only needed exactly that - a loop
 * with no exit, prescribed at the moment they were reading closely.
 *
 * **What it asks for has to be reachable.** Not "switch routing on again":
 * `manager_linux::enable` returns early when the client is already connected,
 * *before* `ca::ensure_trusted`, and this note is raised from inside the enable
 * that just succeeded - so routing is already on while the user reads it, and
 * re-enabling runs nothing. Off and on again does reach the write, because
 * `ensure_trusted` deliberately calls `ensure_trusted_nss` outside its
 * `is_trusted` short-circuit.
 *
 * The failure cases are still raised on the transition rather than standing,
 * even though they describe conditions that persist. That is a deliberate
 * limit: a standing notice needs somewhere to live and something to retire it,
 * and the honest home for "Chromium cannot see the CA" is the certificate
 * section in Settings rather than a banner nobody can clear. Until it has one,
 * the diagnostics report is where the state is legible.
 *
 * What *does* retire one of these is the reading changing: the shell clears the
 * note when `ca_nss_trust` leaves the variant that raised it, so a user who
 * installs the package and cycles routing sees the sentence go rather than
 * being left reading a claim they have just falsified.
 */
export function browserTrustRestartAdvice(
  platform: Platform,
  nss: ProxyState["ca_nss_trust"],
): { title: string; body: string } | undefined {
  if (platform !== "linux") return undefined;
  const store = trustStoreName(platform);
  // Named one by one rather than as "Chromium-based", which is a fact about
  // engines and not a thing anybody has in their dock.
  const family = "Chrome, Chromium, Brave, Edge and Vivaldi";
  if (nss === "tools_missing") {
    return {
      title: "Chromium-based browsers can’t see the certificate",
      body: `Gate added its certificate to your ${store}, but ${family} each keep a separate one, and Gate needs certutil to write it. Install it (Debian/Ubuntu: libnss3-tools, Fedora/RHEL: nss-tools), then turn routing off and on again. Firefox and command-line tools are unaffected.`,
    };
  }
  if (nss === "write_failed") {
    return {
      // "A store", not "one store": the refusals are a list and the report
      // prints a line per entry, so the title must not undercount what the
      // body and the report both say.
      title: "A browser certificate store refused the certificate",
      body: `Gate added its certificate to your ${store}, but at least one of the separate stores ${family} keep would not take it. The diagnostics report names which one and why. Once it is unlocked, turn routing off and on again to retry. Any browser that did take it still needs a full quit and reopen; Firefox and command-line tools are unaffected.`,
    };
  }
  if (nss === "not_written") {
    // Nothing refused and nothing is missing: the stores answered and the CA
    // is not in them, which is what `trust-ca --system-trust` leaves behind.
    // Neither of the sentences above fits - there is no store to go and read
    // about in the report, and no package to install.
    return {
      title: "Chromium-based browsers don’t have the certificate yet",
      body: `Gate added its certificate to your ${store}, but ${family} each keep a separate one and Gate has not written those, so turn routing off and on again to add it. Firefox and command-line tools are unaffected.`,
    };
  }
  return {
    title: "Browsers already open need reopening",
    body: `Gate has added its certificate to your ${store}. A browser reads that when it starts, so one that was already open will reject Gate’s traffic until you quit it completely and open it again.`,
  };
}

/**
 * What a chat row's switch covers, for the pane that opens on it.
 *
 * The window shell draws a row's copy as one sentence (`describeMember`, through
 * `AppPane`'s `description`), which is enough for "App" and "CLI" and not enough
 * for these: the switch inspects a credential the user is already signed in with
 * and it matches on HOST rather than on the app that made the request, so it
 * covers clients the row does not name. The popover said this on the row itself
 * (`screens/GroupMembers.tsx`' `explain`) and the window said it nowhere, which
 * left the fact in the shell that is no longer the default - and the browser
 * half, which is the part a user most needs before flipping a switch that reads
 * a session cookie, reaching nobody.
 *
 * Not shared with that `explain`: its wording turns on `routed` and ends with a
 * sentence about the family switch above it, and there is no family switch above
 * anything on a pane. The one thing both shells must not say differently is the
 * browser claim, and both take that from `browserScopeNote`.
 *
 * Undefined for every other row, including a proxy row that is not a chat
 * surface: those are already covered by their own description and the host is
 * not a claim about someone's browser.
 */
/**
 * What a row's blast radius is, in one sentence, for every row that has one
 * worth saying.
 *
 * The rendered half of {@link GroupMember.scope}, and the reason that field
 * exists rather than being inferred from `kind`. A `host` row is named for one
 * client and covers every client on its hosts, because interception is decided
 * at CONNECT from the host alone and no header has been read yet. That is the
 * sentence a user needed before flipping the switch, and the ledger had nowhere
 * to put it: the surface label said "App" or "Web" and quietly implied the
 * opposite.
 *
 * Undefined for a `client` row, where the row's own name already says it - a
 * config tool covers that tool, which is not news. `machine` says its piece
 * because "every program you start afterwards" is genuinely wider than the row
 * it sits on.
 *
 * Distinct from {@link credentialScopeNote}, which answers "whose key is on
 * this" and is only worth saying where Gate is not supplying it. A brokered
 * host row - the API entries, OpenRouter - gets this note and not that one, and
 * used to get neither.
 */
export function scopeNote(member: GroupMember): string | undefined {
  if (member.scope === "machine") {
    return "Covers every program started after your next login, not only AI tools.";
  }
  if (member.scope !== "host") return undefined;
  const hosts = member.domain?.hosts.join(", ");
  if (!hosts) return undefined;
  // Browser-neutral for the same reason `credentialScopeNote` is: what Gate
  // reaches in a browser depends on the platform's proxy channel, and this
  // note has no reading of it.
  return `Matched on host, so this covers everything on ${hosts} - whatever on this machine sends there, not only ${member.name}.`;
}

export function credentialScopeNote(
  member: GroupMember,
  platform: Platform,
  browserChannel: boolean,
): { title: string; body: string } | undefined {
  if (member.credential === "brokered") return undefined;
  const hosts = member.domain?.hosts.join(", ");
  if (!hosts) return undefined;
  // Two sentences from two fields, which is why they are two fields. The first
  // is the credential: Gate is not supplying a key here. The second is the
  // scope: the row is named for one client and reaches every client on the
  // host. Saying only the first is what left a user reading "Web" and
  // concluding their desktop app was not covered.
  return {
    title: "What this switch covers",
    body: [
      `${member.name} carries the credential you’re already signed in with, not an API key Gate brokers, so Gate records and inspects this traffic rather than supplying a key for it.`,
      member.scope === "host"
        // Deliberately browser-neutral. Whether Gate reaches a browser at all
        // is the platform's answer and `browserScopeNote`'s to give - naming
        // one here would claim it on a Linux session where Gate writes only
        // environment variables a running browser never re-reads.
        ? `It is matched on host, so it covers everything on ${hosts}, whatever sends it.`
        : `It covers ${member.name} alone.`,
      browserScopeNote(platform, browserChannel),
    ]
      .filter(Boolean)
      .join(" "),
  };
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
  /** Which program this row is aimed at. The group's key, carried on the
   * member too so a flat list of members can still say where each belongs. */
  client: ClientId;
  /** How much of the machine the row reaches when it is on.
   *
   * The field the old ledger had no room for, and the one that would have
   * answered the question this taxonomy came from. A `host` row covers every
   * client on its hosts, whatever the row is named for, because interception
   * is decided at CONNECT from the host alone. */
  scope: Scope;
  /** Whose credential rides the traffic. */
  credential: Credential;
  /** Whether a group switch may flip this row.
   *
   * Derived from {@link GroupMember.credential}, never set by hand: only a
   * brokered row cascades, because the others carry a credential the user is
   * already signed in with and routing that is a deliberate per-row act. This
   * replaces the `chat` boolean, which meant this and also meant "session
   * surface" - two facts in one field, with a name that named neither. */
  cascade: boolean;
}

export interface Group {
  id: string;
  name: string;
  /** The group's name inside a sentence. Family names are proper nouns and
   * stay capitalised; "other tools" is a common noun and must not. */
  switchLabel: string;
  /** The vendor whose models this client talks to natively, or null where
   * there isn't one. The rail's caption, and the ledger's sort within a
   * vendor. */
  vendor: string | null;
  /** What the group covers, shown under its name on the family panel, and
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
  /** The group whose heading does not name its own contents, so the row has
   * to list them.
   *
   * Exactly one group: "Any app on this machine". It used to be whichever
   * heading was named by exclusion - "Other tools", then "Experimental" - and
   * the roster rode `multiProvider`, which happened to coincide. It no longer
   * does: OpenCode, OpenClaw and Hermes have no vendor either, and a roster
   * reading "OpenCode" under a heading reading "OpenCode" is the same fact
   * twice. Two facts, two fields.
   */
  namedByExclusion: boolean;
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
    client: tool.client,
    scope: tool.scope,
    credential: tool.credential,
    cascade: tool.credential === "brokered",
    // A tool whose client has no vendor routes whatever providers the user
    // configured in it, so `default_upstream_url` is a placeholder constant and
    // naming it would claim a host the tool may never touch.
    //
    // Derived from the client now. It used to be set by the leftover pass -
    // "whatever no provider claimed" - which happened to be the same set and
    // said nothing about why. Domains never carry it, whatever their client:
    // a domain names real hosts, and `any-app`'s entries are exactly the rows
    // whose host IS the point.
    coversAllProviders: vendorOf(tool.client) === null ? true : undefined,
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
    client: domain.client,
    scope: domain.scope,
    credential: domain.credential,
    cascade: domain.credential === "brokered",
  };
}

/**
 * The clients the ledger can draw, in the order it draws them.
 *
 * Mirrors `taxonomy::Client::ALL`, and the display copy is deliberately on this
 * side of the wire for the same reason {@link MEMBER_DESCRIPTIONS} is: the
 * backend names what it routes, the UI says what that is to the person reading.
 * The backend keeps its own `display_name` for the CLI and the logs, which have
 * no ledger to sit in.
 *
 * `vendor` is null where there genuinely isn't one. OpenCode, OpenClaw and
 * Hermes route whatever providers the user configured in them, and `any-app` is
 * not a program at all - which is exactly the case vendor-grouping could not
 * represent, and the reason those three used to need headings invented for them
 * one at a time.
 */
const CLIENTS: readonly { id: ClientId; name: string; vendor: string | null }[] = [
  { id: "claude-code", name: "Claude Code", vendor: "Anthropic" },
  { id: "claude-desktop", name: "Claude Desktop", vendor: "Anthropic" },
  { id: "codex", name: "Codex", vendor: "OpenAI" },
  { id: "chatgpt", name: "ChatGPT", vendor: "OpenAI" },
  { id: "opencode", name: "OpenCode", vendor: null },
  { id: "openclaw", name: "OpenClaw", vendor: null },
  { id: "hermes", name: "Hermes", vendor: null },
  {
    id: "any-app",
    name: "Any app on this machine",
    vendor: null,
  },
];

/** The vendor behind a client, or null where there isn't one. */
function vendorOf(client: ClientId): string | null {
  return CLIENTS.find((c) => c.id === client)?.vendor ?? null;
}

/** What the widest group covers, said in the group rather than left to its
 *  rows. The other seven headings are a program the user installed and need no
 *  definition; this one is the only group named by what it is not. */
const ANY_APP_BLURB =
  "Routes that cover whatever on this machine reaches them, rather than one app Gate configures. Anything else, including a local model, keeps going where it always did.";

/**
 * Build the Home ledger. Not-installed tools and unsupported domains are left
 * out: the ledger lists what could actually route today.
 *
 * Membership is each row's own `client`, so this is a bucketing rather than the
 * two-pass claim-and-sweep it replaced. There is no leftover pass and no
 * catch-all: a row whose client has no entry in {@link CLIENTS} would vanish,
 * which `every_client_is_drawn` in the test file pins, and the backend's own
 * `all_holds_every_client` pins the other end.
 *
 * The order within a group is tools first, then domains, which is the order the
 * old families drew and the order that reads best: the thing Gate configures,
 * then the hosts it intercepts for it.
 */
export function buildGroups(
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

  const groups: Group[] = CLIENTS.map((client) => {
    const members: GroupMember[] = [
      ...installed.filter((t) => t.client === client.id).map((t) => memberFromTool(t, opts)),
      ...routable.filter((d) => d.client === client.id).map((d) => memberFromDomain(d, opts)),
    ];
    return {
      id: client.id,
      name: client.name,
      vendor: client.vendor,
      switchLabel: `Route ${client.name} through Gate`,
      blurb: client.id === "any-app" ? ANY_APP_BLURB : undefined,
      // The rail captions a group with its vendor where there is one. Without
      // a vendor the group's own name has to stand in, which is what this
      // flag has always meant - it is just derivable now instead of being set
      // per heading.
      multiProvider: client.vendor === null,
      namedByExclusion: client.id === "any-app",
      members,
      routed: members.filter((m) => m.routed).length,
      desired: members.filter((m) => m.desired).length,
      // Only the rows a group switch can flip. An additive row is on screen
      // under this heading and answers to its own switch alone.
      cascadeDesired: members.filter((m) => m.desired && m.cascade).length,
    };
  });

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
 *   which derives the same exclusion from the same credential.
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
    // The credential decides, and it decides here for the same reason the
    // backend's `cascade_domains` decides it there: an additive row carries
    // the user's own session, and a group switch must never route that.
    if (!m.cascade) return false;
    // An overridden member is left out for the same reason a drifted one is:
    // the family switch writes Gate's config, and here that config is already
    // written and already losing. Turning it on again is a no-op the user would
    // read as a fix.
    if (on) return !m.desired && m.attention !== "drifted" && m.attention !== "overridden";
    return m.desired;
  });
}
