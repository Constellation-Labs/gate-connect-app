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
  anthropic: "The Claude desktop app, Cowork included",
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
 * The rows a browser tab can arrive on.
 *
 * Keyed by member key, like {@link MEMBER_HINTS} and for the same reason: no
 * catalog field carries this. `chatgpt-apps` and `chatgpt` are both
 * `Client::ChatGpt`, both `Credential::Additive` and both `Scope::Host`
 * (`proxy/catalog.rs`), so the taxonomy cannot tell them apart, and the slug is
 * the only thing that can. A field on `ProxyDomain` is the real answer and this
 * table is a stand-in for it; adding one is a catalog change and wants the
 * backend, so it is raised rather than done here.
 *
 * The distinction is worth having because the REMEDY turns on it. A page keeps
 * the connection it opened before the PAC named the host, so it bypasses Gate
 * until it is reloaded; a program reads its route at launch, so it has to be
 * closed. Telling one user the other's remedy names something they cannot do.
 *
 * Every entry here is `additive`. That is not the rule - `anthropic` is a host
 * row too and nobody browses `api.anthropic.com` - it is what happens to be
 * true of the two surfaces a person visits in a browser.
 */
const BROWSER_SURFACE_ROWS: ReadonlySet<string> = new Set([
  // claude.ai, and the Claude desktop app's chat sharing that host.
  "claude-web",
  // chatgpt.com, and the ChatGPT desktop app's chat turn.
  "chatgpt-apps",
]);

/**
 * Whether a browser tab can be sitting on this row's hosts right now.
 *
 * The one row this exists to exclude is `chatgpt` ("Subscription"), whose
 * clients are Codex through the relay and the ChatGPT app's Work mode - two
 * programs, no tab. It reads as a chat surface on every field the ledger
 * carries, which is exactly why the answer is written down rather than derived.
 */
export function hasBrowserSurface(member: GroupMember): boolean {
  return BROWSER_SURFACE_ROWS.has(member.key);
}

/**
 * What an open page cannot know yet, for the shell that just routed its host.
 *
 * The same fact `RestartHint` puts on a popover row, for the surfaces that have
 * no row: a section switch routes up to three members at once, and the window
 * and tray report the config half of that through the close-and-reopen dialog
 * and the host half nowhere. A person who turns Claude on with claude.ai open
 * keeps browsing on the connection they had, past Gate, with nothing on screen
 * saying so.
 *
 * ON only, and the caller enforces it. The off direction was assumed symmetric
 * and is not: measured in
 * `crates/core/tests/proxy_e2e.rs::a_row_switched_off_stops_rewriting_a_connection_already_open`,
 * one socket carrying two requests with the row switched off between them. The
 * engine re-reads its rule set per REQUEST rather than per connection, so the
 * very next request on a connection that was already open is no longer routed -
 * it goes to the provider. The connection survives the switch; the routing does
 * not, and there is nothing to tell the user to reload.
 *
 * Undefined when nothing that moved has a browser surface, which is every
 * section whose members are config rows and the `chatgpt` row on its own.
 */
export function hostReloadAdvice(
  moved: GroupMember[],
): { title: string; body: string } | undefined {
  // Deduplicated and in draw order: two members can name one host, and a
  // banner that says "chatgpt.com, chatgpt.com" reads as a bug in the sentence.
  const hosts = [...new Set(moved.filter(hasBrowserSurface).flatMap((m) => m.domain?.hosts ?? []))];
  if (hosts.length === 0) return undefined;
  return {
    title: "Pages already open need reloading",
    body: `Gate now routes ${hosts.join(", ")}. A page that was already open keeps the connection it opened before, so please reload it.`,
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
 * surfaces need - the Claude switch writes Claude Code's config file AND
 * intercepts two hosts, and the user is told "Claude routes through Gate"
 * rather than being handed three switches and the job of knowing which is
 * which.
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
   *  #324 regrouped it by vendor underneath. Two rows today, and the pane's
   *  copy turns on it - see `isProviderEndpoint`. */
  providerEndpoint?: true;
}[] = [
  {
    id: "claude",
    name: "Claude",
    band: "anthropic",
    members: ["claude-code", "anthropic", "claude-web"],
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
 * **The frame also splits the bundles and this does not, yet.** It draws
 * `Claude Desktop` + `Claude Code` under Anthropic (hence its `1 of 2`) and
 * `Codex` + `OpenAI apps` + `ChatGPT` under OpenAI, where the sections here are
 * still one row each for `claude` and `chatgpt`. Design chose to regroup first
 * and split later, 2026-09-22.
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
 */
export const CLI_ONLY_DOMAINS: readonly string[] = ["openai"];

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
    // domain must not overwrite the tool. Both are named by the same section
    // and both need a member, which is why this is keyed per kind rather than
    // per slug.
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


/**
 * The app a routable surface belongs to, or null when no section claims it.
 *
 * The inverse of {@link sectionMemberKeys}, and exported for the same reason:
 * a caller holding a slug but no built ledger still needs to know which app the
 * user thinks it is. The reopen flow is that caller - it is handed running
 * processes, one per surface, and AG-898 asks it to report one outcome per app
 * rather than listing Codex and ChatGPT as two.
 *
 * Reads {@link SECTIONS} directly rather than a built `Group[]`, because the
 * mapping from surface to app is static: what `buildGroups` adds is which
 * members exist on this machine, which is not a question this answers.
 */
export function appForMember(key: string): { id: string; name: string } | null {
  const section = SECTIONS.find((s) => s.members.includes(key));
  return section ? { id: section.id, name: section.name } : null;
}

/** The member keys a section claims, in draw order, or empty for an id no
 *  section owns - which is a section `buildGroups` synthesised for an unplaced
 *  member, whose id IS its member key.
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
 * Is this section a provider endpoint - a destination other programs are
 * pointed at - rather than an app?
 *
 * The pane's unattributed note turns on this and must not turn on
 * `openDomain`, which is the question it looks like and is not: that one is
 * `openTool === null`, "this section has no INSTALLED config tool". A section
 * stays alive on its `domain:` members, so a Claude pane on a machine without
 * Claude Code is `openDomain` too - and "any app on this machine can be
 * pointed here" is false of Claude Desktop, which is one app that nothing is
 * pointed at.
 *
 * Unknown ids answer false. A section this table does not name is a catalog
 * entry that has not been placed yet, and claiming it is an endpoint would put
 * the wrong sentence on it.
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
