import type {
  ClientId,
  Credential,
  ProxyDomain,
  ProxyState,
  Scope,
  Tool,
  Verdict,
} from "./api";
import { trustStoreName, type Platform } from "./platform";

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


/*
 * There used to be a `MEMBER_DESCRIPTIONS` table here: one sentence per row
 * ("Claude Code in your terminal.", "Anything on this machine that calls
 * api.openai.com directly ..."), drawn under the App pane's title. Design
 * asked for that line to go on 2026-09-21 - the frame's header (`app-info`,
 * 408:25099) is title and status only - and nothing else read the table, so it
 * went with the line rather than staying as copy with no reader. It is in the
 * history at the commit that removed it. The window now has no per-app
 * statement of what a switch covers; AG-892, AG-889/AG-897 and the
 * session-routing default all want one, and where it goes is design's to draw.
 */

/**
 * The programs behind a row, named, for the hover on its label.
 *
 * Shorter and more concrete than the pane sentence that used to exist (see the
 * note above `SECTIONS`), and it exists because the two answer different
 * questions. A description says what the surface IS. A rail row has room for one
 * word - "API", "Chat", "CLI" - and the question a user actually arrives with
 * is "which of the things on my machine is that", which a surface kind cannot
 * answer however well it is chosen.
 *
 * The desktop apps are the reason this is worth its own table. They have no
 * config file, so they never appear in `list_tools`, and the ledger names them
 * nowhere: the row that routes Cowork is labelled "API" under a heading reading
 * "Claude Desktop", and a user looking for Cowork by name finds nothing. Every
 * entry here names a product the user could go and open.
 *
 * Cowork and Work are modes inside the Claude and ChatGPT desktop apps rather
 * than apps of their own, so they are named as part of their host app rather
 * than beside it. Confirmed with the product on 2026-09-11, after two earlier
 * readings - that the name varied by OS, then that each was a separate
 * product - both turned out to be wrong. `AGENT_PROCESSES` in
 * `src-tauri/src/lib.rs` carries what that cost.
 *
 * Keyed by member key like the descriptions, so a slug with no entry gets no
 * hover rather than a placeholder - which is the right default for the rows
 * whose subject is a host rather than a product (`openai`, `openrouter`) and
 * for the terminal tools, whose label is already their name.
 */
export const MEMBER_HINTS: Readonly<Record<string, string>> = {
  "claude-code": "Claude Code CLI and IDE plugins",
  // One app, named with the mode people look for. Claude Code reaches this host
  // too, but through its own route selector rather than this switch, so naming
  // it here would promise something this row does not govern - see
  // `claude_code_route_domain`.
  anthropic: "The Claude desktop app, its Code tab and Cowork included",
  // "Chats in", not "The": this row is one surface of that app, not the app.
  // The desktop app's model calls are the `anthropic` row sitting directly
  // above it, and a hover naming the whole product on both would say the two
  // switches do the same thing.
  "claude-web": "Chats in the Claude desktop app, and on claude.ai in a browser",
  codex: "Codex CLI and IDE extension",
  // Scoped to the surface for the same reason as `claude-web` above: the
  // Subscription row beside this one routes that same app's model calls, so a
  // hover naming the whole product on both would say the two switches do the
  // same thing.
  "chatgpt-apps": "Chats in the ChatGPT desktop app, and on chatgpt.com in a browser",
  // Work is a mode in the ChatGPT app rather than an app, so it is named inside
  // it. Codex is the entry's other client, reaching the same endpoint through
  // the relay on the same subscription bearer.
  chatgpt: "Work in the ChatGPT app, and Codex, on your ChatGPT subscription",
};

/** Clients with no single upstream vendor: they route whatever providers the
 *  user configured in them, so `default_upstream_url` is a placeholder constant
 *  and naming it would claim a host the tool may never touch. */
const MULTI_PROVIDER_CLIENTS: ReadonlySet<ClientId> = new Set<ClientId>([
  "opencode",
  "openclaw",
  "hermes",
  "any-app",
]);

/** The programs behind one row, or nothing where none are named. */
export function hintForMember(key: string): string | undefined {
  return MEMBER_HINTS[key];
}

/**
 * The programs behind a whole section, for the hover on a row that is an app.
 *
 * Every member's hint, not the first one's. Both shells took
 * `members.map(m => m.hint).find(Boolean)`, and members are drawn tools first,
 * so the Claude row hovered "Claude Code CLI and IDE plugins" and the ChatGPT
 * row "Codex CLI and IDE extension" - the four entries in {@link MEMBER_HINTS}
 * that exist to name Cowork and Work reached nobody, which was their whole
 * stated purpose. A row that says "Claude" and hovers only its CLI is the
 * surface-vs-app confusion the sections were drawn to end.
 *
 * Sentence-joined and lower-cased after the first, because these are noun
 * phrases rather than sentences and a tooltip is one line.
 */
export function sectionHint(group: Group): string | undefined {
  const hints = [...new Set(group.members.map((m) => m.hint).filter(Boolean))] as string[];
  if (hints.length === 0) return undefined;
  return hints
    .map((h, i) => (i === 0 ? h : h.charAt(0).toLowerCase() + h.slice(1)))
    .join("; ");
}

/**
 * What a browser that was open across the *first* enable cannot know yet.
 *
 * Distinct from the proxy pointer, which a browser on the desktop's own
 * settings re-reads live - advice about that would tell a Firefox user to
 * restart for no reason. It had its own note, `PROXY_REOPEN_ADVICE`, removed
 * with the other advisory banners on 2026-09-24. This one is about the trust
 * store, which is read once at process start.
 * `ca_linux.rs` installs the CA into each browser's own NSS store - Chromium's
 * per-user database and every Firefox profile's - and no running process reads
 * its store again, so a browser that was open when the CA landed fails every
 * intercepted host with a certificate error and looks like Gate breaking the
 * web rather than like a step left to do.
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
 * and the honest home for "the browsers cannot see the CA" is the certificate
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
  // Named rather than "Chromium-based", which is a fact about engines and not
  // a thing anybody has in their dock. Firefox is in it: outside the distros
  // that wire p11-kit in (Fedora, Arch), it reads its own per-profile store
  // and never the system one - Ubuntu's snap Firefox refused claude.ai beside
  // Chrome on the machine this was measured on.
  const family = "Chrome, Firefox and other browsers";
  if (nss === "tools_missing") {
    return {
      title: "Your browsers can’t see the certificate",
      body: `Gate added its certificate to your ${store}, but ${family} each keep a separate one, and Gate needs certutil to write it. Install it (Debian/Ubuntu: libnss3-tools, Fedora/RHEL: nss-tools), then turn routing off and on again. Command-line tools are unaffected.`,
    };
  }
  if (nss === "write_failed") {
    return {
      // "A store", not "one store": the refusals are a list and the report
      // prints a line per entry, so the title must not undercount what the
      // body and the report both say.
      title: "A browser certificate store refused the certificate",
      body: `Gate added its certificate to your ${store}, but at least one of the separate stores ${family} keep would not take it. The diagnostics report names which one and why; a Firefox profile with a Primary Password needs the certificate imported in Firefox’s own settings. Once it is fixed, turn routing off and on again to retry. ${BROWSER_RESTART}`,
    };
  }
  if (nss === "not_written") {
    // Nothing refused and nothing is missing: the stores answered and the CA
    // is not in them, which is what `trust-ca --system-trust` leaves behind.
    // Neither of the sentences above fits - there is no store to go and read
    // about in the report, and no package to install.
    return {
      title: "Your browsers don’t have the certificate yet",
      body: `Gate added its certificate to your ${store}, but ${family} each keep a separate one and Gate has not written those, so turn routing off and on again to add it. Command-line tools are unaffected.`,
    };
  }
  // `trusted` is a write that landed in the browser stores; no reading at all
  // is only `ca_trusted`, so it claims the system store and nothing more.
  return {
    title: "Browsers already open need reopening",
    body:
      nss === "trusted"
        ? `Gate has added its certificate to your ${store} and your browsers. A browser reads its certificates when it starts. ${BROWSER_RESTART}`
        : `Gate has added its certificate to your ${store}. A browser reads its certificates when it starts. ${BROWSER_RESTART}`,
  };
}

/**
 * The sentence the notes that ask for a reopen end on, so the window, the CLI
 * and those variants say the same thing. "Quit and reopen" rather than
 * "restart": closing the last window leaves some browsers running, and that
 * process keeps the certificates it loaded at launch. The CLI carries a copy
 * (`crates/cli/src/main.rs`); the two move together.
 */
export const BROWSER_RESTART = "Quit and reopen any open browser so it trusts the certificate.";

/** The removal's counterpart of {@link BROWSER_RESTART}, mirrored the same way. */
export const BROWSER_REMOVED_RESTART =
  "Quit and reopen any open browser so it stops trusting the certificate.";

/**
 * The note raised when the user removes the certificate, from what the removal
 * itself recorded - never from `ca_trusted` going false, which a regenerated
 * CA or a system anchor removed outside Gate does too, with nothing taken out
 * of any browser.
 *
 * Linux only, for the reason `browserTrustRestartAdvice` is: a running browser
 * there keeps the stores it read at launch, so one that is still open goes on
 * trusting a root that has been taken out of them. macOS and Windows
 * re-evaluate trust for a running process.
 *
 * `nss` is the removal's reading. A failure is the one outcome with a security
 * edge - a Gate root still trusted in a browser while everything else says it
 * is gone - so it gets its own sentence, and where to remove it by hand.
 */
export function browserTrustRemovedAdvice(
  platform: Platform,
  nss: ProxyState["ca_nss_trust"],
): { title: string; body: string } | undefined {
  if (platform !== "linux") return undefined;
  if (nss === "tools_missing") {
    return {
      title: "A browser may still trust Gate’s certificate",
      body: "Gate removed its certificate from your certificate store, but needs certutil to remove it from the browsers’ own stores and it is not installed. Remove the Gate Connect certificate in each browser’s certificate settings.",
    };
  }
  if (nss === "write_failed" || nss === "not_written") {
    return {
      title: "A browser still trusts Gate’s certificate",
      body: "Gate removed its certificate from your certificate store, but at least one browser’s own store would not let go of it. Remove the Gate Connect certificate in that browser’s certificate settings.",
    };
  }
  return {
    title: "Certificate removed",
    body: `Gate removed its certificate from your browsers. ${BROWSER_REMOVED_RESTART}`,
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
  /**
   * Switched on, and the engine is not running.
   *
   * This used to be `master-off` and used to be ordinary: the user had left the
   * master switch off, and the remedy was to turn it on. There is no such
   * switch any more - routing is on for exactly as long as the app is open - so
   * reaching this state means the launch enable did not complete: no account
   * yet, a certificate prompt that was declined, an engine that could not bind.
   * Rarer than it was, and worse when it happens, which is why it still
   * outranks the sweep's own vocabulary below.
   */
  | "not-routing"
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
   * not be routing, because the engine never came up or the certificate is not
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
  /** The programs behind this row, for the hover on a label with no room to
   * say them. See {@link MEMBER_HINTS}. Absent where none are named. */
  hint?: string;
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
   * surface" - two facts in one field, with a name that named neither.
   *
   * One caller breaks it on purpose and is the reason to read this as "may a
   * FAMILY switch flip this" rather than as an invariant: an app switch routes
   * every surface its app uses, additive rows included, and since AG-934 it
   * does so without asking. See {@link cascadeTargets}. */
  cascade: boolean;
}

export interface Group {
  id: string;
  name: string;
  /** The group's name inside a sentence. Family names are proper nouns and
   * stay capitalised; "other tools" is a common noun and must not. */
  switchLabel: string;
  /** Which band the rail draws this section under. */
  band: Band;
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
  /** Whether the APP switch renders on - the window shell's rail and the tray,
   * as opposed to the popover's family switch above.
   *
   * ANY governing member, like `cascadeDesired` before it, and the change is
   * which members govern rather than how they are counted. Any, because the
   * switch states an intent and the next click's meaning: the person has asked
   * for some of this app, so the click that follows means stop. Reading it as
   * ALL was tried and is wrong - it renders off over a routed tool, which is
   * observation leaking into a switch, and it does not even buy reachability:
   * from a partly-on section both readings need the same two clicks to get
   * everywhere, just in the opposite order.
   *
   * What was actually broken is {@link governingMembers}' empty case. A section
   * with no brokered member - ChatGPT / Codex on any machine without the Codex
   * CLI, where both chatgpt.com rows are additive - counted nothing, so the
   * switch read off however the two hosts were set, every click asked for "on",
   * and once they were on {@link cascadeTargets} returned nothing, which made
   * the off-cascade unreachable from the new shell entirely. The fallback is
   * what gives that section members to be described by. */
  switchOn: boolean;
  /** How many governing members the person has asked for, which separates
   * "asked for and blocked" from "off". Drives the status line's qualifier. */
  switchDesired: number;
}

/**
 * The members that define a section's switch and its status line.
 *
 * The brokered half, as it always was - a section is not "off" because its
 * session surface is off, and a section whose session surface alone is on does
 * not get to claim it is routing. The fallback is the new part: a section with
 * NO brokered member has to be described by the members it does have, or it
 * describes nothing at all and the switch that flips them can never move.
 *
 * Exported because {@link sectionStatus} answers the same question about the
 * same rows, and the switch and the line beside it disagreeing is the whole
 * class of bug this file exists to prevent.
 */
export function governingMembers(members: GroupMember[]): GroupMember[] {
  const brokered = members.filter((m) => m.cascade);
  return brokered.length > 0 ? brokered : members;
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
    hint: hintForMember(tool.slug),
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
            ? // Above not-routing and unverified on purpose: those describe a
              // route that would carry this tool's traffic once something is
              // switched on, and this one says the traffic is not on our route
              // at all.
              "overridden"
            : // `not-routing` outranks the sweep's own vocabulary because it
            // is the better sentence for the same fact: the sweep would report
            // a dead relay as a connection problem, and "routing did not
            // start" is what the user needs to hear.
            connected && !proxyOn
            ? "not-routing"
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
    // Derived from the client, which is the fact rather than a proxy for it:
    // these are the clients that belong to no vendor, because what they route
    // is whatever the user configured in them. Domains never carry it - a
    // domain names real hosts, and the `any-app` entries are exactly the rows
    // whose host IS the point.
    coversAllProviders: MULTI_PROVIDER_CLIENTS.has(tool.client) ? true : undefined,
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
    hint: hintForMember(domain.slug),
    // An enabled domain behind an untrusted certificate is not carrying
    // traffic, so it does not count as routed - same rule as the header's
    // "Partly routed".
    routed: domain.enabled && proxyOn && caTrusted,
    desired: domain.enabled,
    attention: domain.enabled && proxyOn && !caTrusted
      ? "needs-trust"
      : domain.enabled && !proxyOn
        ? "not-routing"
        : null,
    domain,
    client: domain.client,
    scope: domain.scope,
    credential: domain.credential,
    cascade: domain.credential === "brokered",
  };
}

/**
 * The ledger's sections, in the order it draws them.
 *
 * **One row per app the user has, not one per routable surface.** A section's
 * switch routes everything that app does, across whatever mechanisms its
 * surfaces need - the ChatGPT / Codex switch writes Codex's config file AND
 * intercepts two chatgpt.com surfaces, and the user is told "ChatGPT / Codex
 * routes through Gate" rather than being handed three switches and the job of
 * knowing which is which. Claude Desktop and Claude Code are two apps, so two
 * rows (2026-10-08).
 *
 * This replaced grouping by `client`, which replaced grouping by vendor. The
 * taxonomy that bucketing exposed is still the model - it derives the cascade,
 * the scope copy, the diagnostics triple and the CLI table - it is just no
 * longer the shape of the screen. `docs/ui-app-switches-plan.md` has the
 * argument.
 *
 * Membership is by member key rather than by `client`, because two sections cut
 * across the client axis on purpose: `Terminal` and `OpenAI API` are both
 * `Client::AnyApp` and are separate switches, since they are different
 * mechanisms with different blast radii and nothing depends on both.
 *
 * Order within a section is the order written here, and it is tools first then
 * hosts: the thing Gate configures, then the hosts it intercepts for it.
 */
/**
 * What the shell-environment channel reaches, in one sentence, drawn wherever
 * the channel is described in our own words: the window's Settings row and the
 * popover's Terminal blurb. One string so the two cannot drift apart, which is
 * what AG-893 reported: "Terminal" and "Also set shell environment variables"
 * described one control two ways, and the first fix described it a third.
 *
 * "From now on", not "after your next login". On macOS `launchctl setenv` is
 * the parent of everything the session starts afterwards, new Terminal windows
 * included (`system_proxy.rs`); Windows writes `HKCU\Environment`, which every
 * new process inherits; Linux pushes the variables into the running session
 * over D-Bus and only falls back to the next login where there is no session
 * bus (`system_proxy_linux.rs`). Already-running programs keep their old
 * environment everywhere, which "you start from now on" also says.
 *
 * The certificate clause is the larger of the two facts and the harder one to
 * discover: `NODE_EXTRA_CA_CERTS` puts Gate's interception CA in the trust
 * roots of every Node process started afterwards.
 *
 * The tray's card keeps its own drawn copy (`735:37341`) and is not this
 * string; the file wins there.
 */
export const SHELL_CHANNEL_COVERAGE =
  "Routes every program you start from now on, not only AI tools, and tells Node to trust Gate's certificate.";

/**
 * **Order matters, and not for looks.** `NewUiApp` and `TrayApp` both emit a
 * group header on every *change* of band while walking these in order, so a
 * band that appears, stops and appears again draws two headers with the same
 * name and two counters that each count half its rows. Keep every band's
 * sections contiguous; `groups.test.ts` fails if they are not.
 */
export const SECTIONS: readonly {
  id: string;
  name: string;
  /** Which band the rail draws it under. */
  band: Band;
  /** Member keys, in draw order. A key with nothing behind it contributes no
   *  member, and a section with no members is dropped. */
  members: readonly string[];
  blurb?: string;
  /** A destination the user points other programs at, rather than a program
   *  they launch.
   *
   *  Explicit because no existing field means this. `band` is the closest
   *  thing and is not it - it is a layout choice, it has held these rows
   *  alongside OpenClaw, Hermes, OpenCode and the environment channel, and
   *  #324 regrouped it by vendor underneath. One row today - see
   *  `isProviderEndpoint`. */
  providerEndpoint?: true;
}[] = [
  // Two rows, as the Anthropic band's frame draws them: the desktop app (with
  // its Code tab, Cowork and claude.ai chat) and the terminal CLI. They route
  // apart and take Gate models apart - one model for the app, a set for the
  // CLI - so one row could not say which of the two it meant.
  {
    id: "claude-desktop",
    name: "Claude Desktop",
    band: "anthropic",
    members: ["anthropic", "claude-web"],
  },
  {
    id: "claude-code",
    name: "Claude Code",
    band: "anthropic",
    members: ["claude-code"],
  },
  {
    id: "chatgpt",
    name: "ChatGPT / Codex",
    band: "openai",
    // Both names, because the switch covers both and neither alone is the
    // whole of it: Codex is a terminal tool with its own config file, and the
    // other two surfaces are the ChatGPT app's chat turn and the endpoint Work
    // mode calls.
    members: ["codex", "chatgpt-apps", "chatgpt"],
  },
  {
    id: "openai-api",
    providerEndpoint: true,
    name: "OpenAI API",
    band: "openai",
    // Its own switch rather than part of ChatGPT / Codex, because nothing
    // OpenAI ships rides it: Codex routes through the relay whatever this
    // says, and the desktop app is on chatgpt.com. What depends on it is
    // whatever else on the machine calls api.openai.com - in practice the two
    // harnesses above, which blind-tunnel anything outside the enabled
    // catalog. Folding it into the app switch would mean routing every script
    // on the machine as a side effect of routing ChatGPT.
    members: ["openai"],
  },
  {
    id: "openclaw",
    name: "OpenClaw",
    band: "other",
    members: ["openclaw"],
  },
  {
    id: "hermes",
    name: "Hermes",
    band: "other",
    members: ["hermes"],
  },
  {
    id: "opencode",
    name: "OpenCode",
    band: "other",
    members: ["opencode"],
  },
  {
    // Still here although the window's rail no longer draws it - see
    // {@link SETTINGS_MANAGED_MEMBERS}. The popover reads this section, and
    // deleting it would not remove the row anyway: an unclaimed member key gets
    // a section of its own, so the row would come back under its raw name with
    // none of the copy below.
    id: "terminal",
    name: "Terminal",
    band: "other",
    members: ["env-proxy"],
    // The same sentence the window's Settings row draws, then the one fact the
    // popover has room for that the row does not. One coverage statement for
    // one control is what AG-893 asked for; this used to say "after your next
    // login" where Settings said "afterwards", which was the ticket's defect
    // restated across two surfaces.
    //
    // "Anything else, including a local model, keeps going where it always did"
    // was not true and is gone. `NO_PROXY_VALUE` is `localhost,127.0.0.1,::1` -
    // loopback only - so a model served from another machine on the LAN or over
    // Tailscale DOES traverse the engine, which is exactly the case AG-899
    // reports breaking when routing is switched off. What is true is that Gate
    // inspects only the provider hosts it knows and blind-tunnels the rest
    // (`proxy/mod.rs`: "A CONNECT to any other host is blind-tunnelled").
    //
    // Widening `no_proxy` to private, link-local and Tailscale addresses is
    // AG-911's scope, not this ticket's. When it lands, the stronger sentence
    // becomes true and can come back.
    blurb: `${SHELL_CHANNEL_COVERAGE} Gate inspects traffic to the AI providers it knows and passes everything else through untouched.`,
  },
];

/**
 * Which group a section draws under in the rail.
 *
 * **By vendor, not by kind.** This was `"apps" | "tools"` - a split on what the
 * user was being asked, an app they use versus a mechanism they opt into - and
 * the Figma sidebar (`440:1593`) draws vendor groups instead: `Anthropic`,
 * `open ai`, and a catch-all. Design confirmed the grouping on 2026-09-22, so
 * the file wins, per CLAUDE.md.
 *
 * Three, not the frame's four. The frame draws a fourth group labelled
 * `OPENCode` containing **OpenRouter**, while OpenCode itself sits under the
 * catch-all - a self-contradiction no rule settles, and guessing wrong files a
 * row under a competitor's name. Design's answer on 2026-09-22 was "no
 * OpenRouter group", so OpenRouter joins the catch-all and the frame's third
 * group is dropped rather than interpreted.
 *
 * The catch-all is "Other apps", design's wording rather than the frame's
 * "Other tools" - same answer, same day.
 *
 * **The frame also splits the bundles, and this has split one of them.** It
 * draws `Claude Desktop` + `Claude Code` under Anthropic (hence its `1 of 2`),
 * which is what the sections are since 2026-10-08, and `Codex` + `OpenAI apps`
 * + `ChatGPT` under OpenAI, which is still one `chatgpt` row. Design chose to
 * regroup first and split later, 2026-09-22.
 */
export type Band = "anthropic" | "openai" | "other";

/**
 * Member keys the window's APP LIST does not draw, because they are not apps.
 *
 * One entry: `env-proxy`, the shell-environment channel. It sat in the rail
 * under a heading reading "Terminal", described as "command line tools that
 * follow your proxy settings", which undersells it badly. `launchctl setenv`
 * hands seven variables to every process started afterwards in the login
 * session - the four `http(s)_proxy` spellings, the two `no_proxy` ones, and
 * `NODE_EXTRA_CA_CERTS` - GUI apps opened from Finder included. That last one
 * puts Gate's interception CA in the trust roots of every Node process started
 * afterwards.
 *
 * A machine-wide setting is not an app, and the rail is a list of apps. It is a
 * setting, so it lives in Settings, where the rest of this window's machine
 * state does. The tray mirrors it as a status card rather than a second switch,
 * which is the rule the tray already follows for the engine ("the engine's own
 * control stays in the full app", `Tray.tsx`).
 *
 * That also answers AG-893, which reported two controls describing the same
 * coverage in different words. There is one control now, in the place a setting
 * belongs, and the surfaces that are not the window report it rather than
 * offering it again.
 *
 * NOT applied by {@link buildGroups}, deliberately: the popover still draws this
 * row and the ledger is shared, so the window filters its own inputs instead -
 * see `NewUiApp`'s `groups` and `apps`. Filtering the TOOL LIST rather than the
 * built ledger is what keeps the row from reappearing under "unclaimed", which
 * is where a member no section claims lands.
 */
export const SETTINGS_MANAGED_MEMBERS: readonly string[] = ["env-proxy"];

/** Whether this member's control lives in Settings rather than the app list.
 *  See {@link SETTINGS_MANAGED_MEMBERS}. */
export function isSettingsManaged(key: string): boolean {
  return SETTINGS_MANAGED_MEMBERS.includes(key);
}

/**
 * Domains the app draws only while they are on, by slug. Turning one on is the
 * CLI's job (`proxy domain <slug> on`); the row appears once it is on so that
 * turning it off never needs the CLI too.
 *
 * Two things can switch one on besides the CLI: an older build, where the row
 * was always drawn, and the Hermes provider ask, which offers any host a
 * Hermes provider points at and tells the user they can turn it off again from
 * its own row. Hiding the row while it is on would strand both.
 *
 * Filtered before grouping rather than left out of {@link SECTIONS}, because a
 * member no section names lands in a catch-all row of its own - which is the
 * point of that fallback, and exactly what these must not get while off.
 * Applied here rather than by the callers, unlike
 * {@link SETTINGS_MANAGED_MEMBERS}, because every surface that lists rows
 * hides these the same way; there is no Settings row to hand them to.
 *
 * `openai` is the api.openai.com host. Nothing OpenAI ships rides it, so the
 * row is a switch for "every other program on this machine that calls
 * OpenAI" - a CLI user's decision rather than something to offer in the app.
 *
 * `opencode` is Zen / Go on `opencode.ai`, which the OpenCode row used to carry
 * as a second member whether or not it was on. Off, it made that row read
 * "Partly protected - 1 of 2" and kept it on screen on a machine without
 * OpenCode. Nothing OpenCode does needs the host: its config already sends the
 * `opencode` and `opencode-go` providers through the relay, which routes them
 * whether or not the domain is enabled (`KNOWN_PROVIDERS` in
 * `integrations/opencode.rs`). Here rather than in {@link TOOL_MANAGED_DOMAINS}
 * because the OpenCode switch of an older build turned it on and nothing
 * recorded that, so hiding it while on would leave `opencode.ai` intercepted
 * with no way off but the CLI. While it is on, the row is two members again and
 * the OpenCode switch turns it off with the tool.
 */
export const CLI_ONLY_DOMAINS: readonly string[] = ["openai", "opencode"];

/**
 * Domains the app never draws, because a tool's own switch owns them.
 *
 * `openrouter` is Hermes' provider. Its row sat under "Other apps" inviting a
 * reader to manage it, and managing it was never really theirs to do: Hermes
 * routed to a provider Gate did not inspect was the state the whole coverage
 * path exists to prevent, so the app asked about it every time
 * (`HermesProviderDialog`) rather than leaving it to them. Product removed
 * both the row and the question on 2026-09-23 - Hermes' switch turns the
 * domain on, and turning Hermes off turns off what it turned on.
 *
 * **Unlike {@link CLI_ONLY_DOMAINS} this hides the row while it is ON too**,
 * which is the whole point and is also what makes it dangerous: there is no
 * row to switch off from. What keeps that honest is
 * `preferences::auto_enabled_domains` - the record of what Gate enabled, and
 * the thing Hermes' off reads. A domain the person turned on themselves is
 * not in that record, so it is never switched off for them; it is also never
 * drawn, so the CLI is their only way back. That trade was accepted with the
 * decision.
 */
export const TOOL_MANAGED_DOMAINS: readonly string[] = ["openrouter"];

/** The rail's eyebrow per band. The eyebrow is the band rather than the
 *  section, because a section is a row now and labelling each with its own
 *  name would print every name twice. */
export const BAND_LABELS: Readonly<Record<Band, string>> = {
  anthropic: "Anthropic",
  openai: "OpenAI",
  other: "Other apps",
};

/**
 * Build the ledger: one row per app, not one per routable surface.
 *
 * Not-installed tools and unsupported domains are left out, so the ledger lists
 * what could actually route today, and a section left with no members is
 * dropped rather than drawn empty.
 *
 * **Nothing can fall off.** A member key {@link SECTIONS} does not name gets a
 * section of its own rather than vanishing - a catalog entry added before
 * anyone gives it a home is a row someone has to see, and the alternative is a
 * routable surface that exists in the backend and nowhere on screen. That was
 * the failure mode of every hand-kept grouping this file has had.
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
  const byKey = new Map<string, GroupMember>();
  for (const tool of tools) {
    if (tool.status.kind !== "not_installed") byKey.set(tool.slug, memberFromTool(tool, opts));
  }
  for (const domain of domains) {
    // Tool and domain slugs share one namespace and `opencode` is both, so the
    // domain must not overwrite the tool. While the domain is on, both are
    // named by the same section and both need a member, which is why this is
    // keyed per kind rather than per slug.
    if (TOOL_MANAGED_DOMAINS.includes(domain.slug)) continue;
    if (domain.supported && (domain.enabled || !CLI_ONLY_DOMAINS.includes(domain.slug))) {
      byKey.set(`domain:${domain.slug}`, memberFromDomain(domain, opts));
    }
  }

  const claimed = new Set<string>();
  const membersFor = (keys: readonly string[]): GroupMember[] => {
    const out: GroupMember[] = [];
    for (const key of keys) {
      for (const candidate of [key, `domain:${key}`]) {
        const member = byKey.get(candidate);
        if (!member || claimed.has(candidate)) continue;
        claimed.add(candidate);
        out.push(member);
      }
    }
    return out;
  };

  const groups: Group[] = SECTIONS.map((section) => {
    const members = membersFor(section.members);
    return group(section.id, section.name, section.band, members, section.blurb);
  });

  // Whatever no section named. Empty in a shipped build; a row here is a
  // catalog entry that needs a home, and it is visible until it gets one.
  for (const [key, member] of byKey) {
    if (claimed.has(key)) continue;
    groups.push(group(member.key, member.name, "other", [member]));
  }

  return groups.filter((g) => g.members.length > 0);
}

/** One section, with the counts the rail and the switch read off it. */
function group(
  id: string,
  name: string,
  band: Band,
  members: GroupMember[],
  blurb?: string,
): Group {
  const governs = governingMembers(members);
  return {
    id,
    name,
    band,
    switchLabel: `Route ${name} through Gate`,
    blurb,
    members,
    routed: members.filter((m) => m.routed).length,
    desired: members.filter((m) => m.desired).length,
    // Brokered members only, which is what the switch's *rendered* state reads
    // off. What the switch *flips* is a different question and a wider set -
    // see `cascadeTargets`, which an app switch calls with consent. Keeping the
    // rendered state on the brokered half means a section whose session
    // surface alone is on does not claim to be routing.
    cascadeDesired: members.filter((m) => m.cascade && intended(m)).length,
    switchOn: governs.some(intended),
    switchDesired: governs.filter(intended).length,
  };
}

/**
 * Whether the app switch should read this member as ON - the section's half of
 * the intent-vs-flow split.
 *
 * `desired` alone is not it, and the gap is a drifted tool. `memberFromTool`
 * sets `desired` from the config being Gate's, which a drifted one's is not -
 * something else rewrote it. But the person still asked for this app to route,
 * and principle 2 is explicit about what happens when a switch is driven by
 * anything else: the switch renders off, and clicking it turns off the setting
 * they were trying to turn on. The rail said so in its own words before the rows
 * became sections ("a drifted tool is still one the user asked to route"), and
 * the sections have to carry it now that the row is built from the ledger.
 *
 * Deliberately NOT folded into `GroupMember.desired`, which the popover's
 * `toggleMember` reads for the opposite purpose: there, a drifted member's
 * `desired: false` is what raises the re-adopt confirmation. Two questions, two
 * answers - "did the person ask for this" and "is Gate's config in place".
 */
function intended(m: GroupMember): boolean {
  return m.desired || m.attention === "drifted";
}


/** The member keys a section claims, in draw order, or **`[id]` itself** for
 *  an id no section owns - which is a section `buildGroups` synthesised for an
 *  unplaced member, whose id IS its member key.
 *
 *  **Never empty**, which is easy to misread: this said "or empty" while
 *  returning `[id]`, and a caller testing `.length > 0` to mean "is a declared
 *  section" got true for everything. Use {@link isDeclaredSection} for that
 *  question.
 *
 *  Exported so a caller that has an id but not a built ledger can still resolve
 *  it. `NewUiApp` needs exactly that: the open pane's activity read is set up
 *  before the ledger is built, and reaching for the built groups there is a
 *  use-before-declare rather than a lookup.
 */
export function sectionMemberKeys(id: string): readonly string[] {
  return SECTIONS.find((s) => s.id === id)?.members ?? [id];
}

/**
 * Every sender a section's traffic arrives under, for the sections whose surfaces
 * the engine names apart (`client_tool` in `proxy/mod.rs`).
 *
 * The Claude Desktop pane covers the desktop app and claude.ai - the app's Code
 * tab included, which the engine stamps as the app; the ChatGPT pane covers
 * Codex, the desktop app and chatgpt.com. Claude Code is its own row since the
 * split and reads its tool's name like any other single-tool section. Each is a name the engine
 * stamps into `x-gate-client`, and the activity reads refuse any other
 * (`activity::feed_clients`).
 *
 * Not decided by vendor: the OpenAI API section's traffic carries no app name,
 * so it is absent here and keeps no feed of its own.
 */
export const SECTION_CLIENTS: Readonly<Record<string, readonly string[]>> = {
  "claude-desktop": ["claude-desktop", "claude-web"],
  chatgpt: ["codex", "chatgpt", "chatgpt-web"],
};

/** One key for a set of client names: sorted and comma-joined, so the same set
 *  in any order is one key. It is the backend's cache key too
 *  (`activity_cache::key`), which is what lets the tray find a section's reading
 *  the window's pane stored, and what the hooks key their effects on. */
export function clientScopeKey(names: readonly string[]): string {
  return [...new Set(names)].sort().join(",");
}

/** The client names an app pane reads its activity for: the section's table entry,
 *  else the one config tool it resolved, else none - and none means the pane has
 *  no per-app reading at all. */
export function paneClients(id: string, openTool: string | null): readonly string[] | null {
  return SECTION_CLIENTS[id] ?? (openTool === null ? null : [openTool]);
}

/** Whether {@link SECTIONS} declares this id - as opposed to it being a member
 *  key that `buildGroups` will synthesise a one-row section for.
 *
 *  The distinction `sectionMemberKeys` cannot make, because it answers `[id]`
 *  for both. A section id is not a `ToolId`, so anything that reaches the
 *  per-tool commands with one gets `unknown tool "..."` from Rust. */
export function isDeclaredSection(id: string): boolean {
  return SECTIONS.some((s) => s.id === id);
}

/**
 * The sections with no row because nothing behind them is on this machine,
 * for the rail's "Not installed" group.
 *
 * A section qualifies when at least one of its members is a tool reporting
 * `not_installed` and the ledger drew no row for it. The second half is what
 * keeps ChatGPT / Codex out on a machine without Codex: its domains still give
 * it a row, so the app is there even though one program inside it is not.
 * Claude Code has no domain of its own since the split, so it is listed here
 * like any single-tool row, while Claude Desktop is never a candidate: it has
 * no tool member at all. Detection is the tool's own: for OpenCode
 * without a binary on the path, a config file or a login counts and an empty
 * leftover `~/.config/opencode` does not.
 *
 * Takes the same filtered tool list `buildGroups` was given, so a
 * settings-managed member cannot come back here.
 */
export function notInstalledSections(
  tools: Tool[],
  groups: Group[],
): { id: string; name: string }[] {
  const missing = new Set(
    tools.filter((t) => t.status.kind === "not_installed").map((t) => t.slug),
  );
  const drawn = new Set(groups.map((g) => g.id));
  return SECTIONS.filter(
    (s) => !drawn.has(s.id) && s.members.some((key) => missing.has(key)),
  ).map((s) => ({ id: s.id, name: s.name }));
}

/**
 * Is this section a provider endpoint - a destination other programs are
 * pointed at - rather than an app?
 *
 * Not the same question as `NewUiApp`'s `noPaneReading`, which it looks like:
 * that one is "this pane has no sender to read" - no `SECTION_CLIENTS` entry
 * and no installed config tool. The Claude Desktop pane has no config tool and
 * still reads its section, and Claude Desktop is one app that nothing is
 * pointed at.
 *
 * Unknown ids answer false. A section this table does not name is a catalog
 * entry that has not been placed yet, and claiming it is an endpoint would
 * misdescribe it.
 */
export function isProviderEndpoint(id: string): boolean {
  return SECTIONS.find((s) => s.id === id)?.providerEndpoint === true;
}

/** Which kind of exception `groupSummary` found, so a row can give the sentence
 * its own ink instead of printing every severity in the same grey. Reality is
 * what this ledger is for, and it was losing the row to the switch beside it:
 * intent is one saturated indigo object, so reality has to speak in more than
 * one place to hold its own. */
export type GroupException =
  | "error"
  | "needs-trust"
  | "not-routing"
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
  const notRouting = group.members.filter((m) => m.attention === "not-routing");
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
  if (notRouting.length > 0) {
    return { count, exception: "not routing", kind: "not-routing" };
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
export function cascadeTargets(
  group: Group,
  on: boolean,
  opts: { sessions?: boolean } = {},
): GroupMember[] {
  return group.members.filter((m) => {
    // Opt-in, and the default is the safe one. An additive row carries a
    // credential the person is already signed in with, and only a caller that
    // has ASKED may route it - which today is the window shell's app switch,
    // after its confirmation. The popover has no such dialog, so it passes
    // nothing and keeps the brokered-only cascade it has always had.
    //
    // A parameter rather than a property of the group, because it is a fact
    // about the caller (did you ask?) and not about the section.
    if (!m.cascade && !opts.sessions) return false;
    // With `sessions`, every member - including the additive ones. A section
    // switch is an APP switch: it routes everything that app does, and for
    // Claude and ChatGPT that includes a surface the user is signed in to.
    //
    // This is the one place on the frontend where the invariant the rest of the
    // tree enforces is deliberately broken, so it is worth being exact about
    // what replaces it - and about how much the Rust layer is actually holding.
    // `provider::cascade_domains` refuses these rows to a FAMILY switch, so no
    // cascade in Rust and none from the CLI's `provider enable` can reach them.
    // It is not a check on routing them: `proxy_set_domain` will enable any row
    // it is handed, which is what the CLI's `proxy domain claude-web on` and the
    // popover's own per-surface switch both do, deliberately.
    //
    // Nothing stands in front of THIS path. `SessionConsentDialog` did, and
    // the guarantee was "cannot happen without being told"; the dialog was
    // removed on 2026-09-23 (AG-934) and both shells now pass `sessions`
    // unconditionally, so it is "can happen, silently". That is the decision,
    // not a gap - see docs/ui-app-switches-plan.md, which predates it.
    // An overridden member is left out for the same reason a drifted one is:
    // the family switch writes Gate's config, and here that config is already
    // written and already losing. Turning it on again is a no-op the user would
    // read as a fix.
    //
    // `intended`, not `desired`, on BOTH branches - the same predicate the
    // switch renders from, or the two disagree about one click. Turning a
    // section off has to disconnect its drifted tool: the switch reads on
    // because the person asked for that app, so an off-cascade that skipped the
    // member would leave the switch on after the click that turned it off.
    if (on) return !intended(m) && m.attention !== "overridden";
    return intended(m);
  });
}
