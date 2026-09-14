import type {
  Account,
  Diagnostics,
  LaunchAtLoginStatus,
  OAuthStatus,
  ProviderState,
  ProxyState,
  RunningAgent,
  RunningAgents,
  Status,
  Tool,
} from "./api";
import type { Platform } from "./platform";
import type { AnalyticsId } from "./analytics";

/**
 * The copy-pasteable support report.
 *
 * A pure function over state the popover already holds plus one backend
 * snapshot, so what the user pastes is exactly what the panel showed them -
 * PRODUCT.md's transparency rule applies to the report itself, not just to the
 * screens it describes. Being pure is also what makes the format testable
 * without a webview.
 *
 * What it deliberately never prints: the Gate key, its recorded prefix, the
 * OAuth token, and every upstream provider credential. The report is written
 * to leave the machine, so nothing in it may be a secret once it has. Email
 * and org *are* printed - they are the account's identity, they are already on
 * screen in Settings, and without them a support thread cannot find the
 * account it is about. The analytics id is printed on the same grounds: it is
 * a random per-install id, not a credential, and it is what lets a pasted
 * report be joined to the events this install actually sent.
 *
 * The layout is plain text with mono-aligned labels rather than JSON: it gets
 * pasted into chat threads and issues, where a human reads it first.
 */

/** Field label column. Wide enough for the longest label below, narrow enough
 *  that a value still gets room on one line in a 360px popover. */
const LABEL_WIDTH = 16;

/**
 * What to print after a tool's status about which build it is.
 *
 * Three outcomes, three strings, because they send a support thread to three
 * different places. A version is the answer. `version unreadable` means the
 * executable was found and would not say - a broken or unusual install. Silence
 * means no executable was found at all, which for a tool detected through its
 * config directory is itself the finding: Gate can see its config and not its
 * binary.
 */
function versionSuffix(
  slug: string,
  versions: Record<string, string | null> | null,
): string {
  if (!versions || !(slug in versions)) return "";
  const version = versions[slug];
  return version ? ` (v${version})` : " (version unreadable)";
}

export interface DiagnosticsInput {
  /** Stamped into the report. Injected so tests aren't clock-dependent. */
  now: Date;
  /** App version from `getVersion()`; empty while it is still loading. */
  version: string;
  platform: Platform;
  /** Which id this install's events are filed under, so a report and an event
   *  stream can be lined up. `disabled` is itself the answer to "why do I see
   *  no events for this user". */
  analyticsId: AnalyticsId;
  /** Null when the backend snapshot failed. The report still renders - a
   *  missing section is itself a finding. */
  backend: Diagnostics | null;
  /** Each tool's version, or null if the probe was never run. A slug absent
   *  from the map is a tool whose executable was not found; a slug mapped to
   *  null is one that would not say. See `versionSuffix`. */
  versions: Record<string, string | null> | null;
  account: Account | null;
  oauth: OAuthStatus | null;
  proxy: ProxyState | null;
  providers: ProviderState[];
  tools: Tool[];
  launchAtLogin: LaunchAtLoginStatus | null;
  /** Routing came back on a different local port than last session, so
   *  already-running tools may point at a dead one. */
  clientsStale: boolean;
  /** The running AI tools, or null when the probe failed. Named individually
   *  rather than counted: "which tool has been up since before routing" is
   *  the question a count can only hint at. */
  agents: RunningAgents | null;
}

function row(label: string, value: string): string {
  return `${label.padEnd(LABEL_WIDTH)}${value}`;
}

function yesNo(value: boolean): string {
  return value ? "yes" : "no";
}

function onOff(value: boolean): string {
  return value ? "on" : "off";
}

/** A value we could not resolve. Named rather than blank, so a hole in the
 *  report reads as a fact about the machine instead of a formatting bug. */
const UNKNOWN = "unknown";

function orUnknown(value: string | null | undefined): string {
  return value && value.length > 0 ? value : UNKNOWN;
}

/** A duration in whole units, largest two first: "3d 4h", "2h 46m", "9m".
 *  Days matter because an agent process left running over a weekend is
 *  exactly the one that predates routing. */
function span(totalSec: number): string {
  const days = Math.floor(totalSec / 86400);
  const hours = Math.floor((totalSec % 86400) / 3600);
  const minutes = Math.floor((totalSec % 3600) / 60);
  if (days > 0) return `${days}d ${hours}h`;
  return hours > 0 ? `${hours}h ${minutes}m` : `${minutes}m`;
}

/** Time to (or since) the access token's expiry, as a span rather than a
 *  timestamp: "expired 3h ago" is the diagnosis, the wall clock is not. */
export function expiryPhrase(expiresAtUnix: number, now: Date): string {
  if (expiresAtUnix <= 0) return "n/a";
  const deltaSec = expiresAtUnix - Math.floor(now.getTime() / 1000);
  const phrase = span(Math.abs(deltaSec));
  return deltaSec >= 0 ? `in ${phrase}` : `${phrase} ago`;
}

/** Unix seconds as an ISO timestamp trimmed to the second. Absolute, because
 *  the reason to print a start time at all is to line the process up against
 *  a log; the "up 2h 46m" beside it is for the human scanning. */
function isoSecond(unixSeconds: number): string {
  return `${new Date(unixSeconds * 1000).toISOString().slice(0, 19)}Z`;
}

/** One running tool: what it is, how long it has been up, and whether it
 *  predates routing - the last being the whole reason this section exists. */
export function agentLine(agent: RunningAgent, now: Date): string {
  const parts = [`pid ${agent.pid}`];
  if (agent.started_at_unix > 0) {
    const upSec = Math.max(0, Math.floor(now.getTime() / 1000) - agent.started_at_unix);
    parts.push(`up ${span(upSec)}`, `started ${isoSecond(agent.started_at_unix)}`);
  } else {
    parts.push("start time unavailable");
  }
  if (agent.predates_routing) parts.push("predates routing");
  return row(agent.name, parts.join(", "));
}

/** One tool's status, flattened to a line. `drifted` and `error` keep their
 *  reason: that string is the single most useful field in the whole report
 *  when a tool is misbehaving, and the UI already shows it. */
export function toolStatusLine(status: Status): string {
  switch (status.kind) {
    case "not_installed":
      return "not installed";
    case "detected":
      return "installed, not routed";
    case "connected":
      return "routed";
    case "drifted":
      return `drifted: ${status.reason}`;
    case "overridden":
      return `overridden: ${status.source}`;
    case "error":
      return `error: ${status.message}`;
  }
}

/** The analytics id line. `disabled` says analytics never started on this
 *  install; UNKNOWN says it did and the id would not come back. */
function analyticsIdValue(id: AnalyticsId): string {
  switch (id.kind) {
    case "id":
      return id.value;
    case "disabled":
      return "disabled";
    case "unavailable":
      return UNKNOWN;
  }
}

export function buildDiagnosticsReport(input: DiagnosticsInput): string {
  const {
    now,
    version,
    platform,
    analyticsId,
    backend,
    versions,
    account,
    oauth,
    proxy,
    providers,
    tools,
    launchAtLogin,
    clientsStale,
    agents,
  } = input;

  const lines: string[] = [
    "Gate Connect diagnostics",
    // UTC, unambiguously: these get pasted across time zones, and a local
    // string without an offset costs the reader the one thing a timestamp is
    // for - ordering it against the log line they are holding.
    `generated ${now.toISOString()}`,
    "",
    "[app]",
    row("version", version ? `v${version}` : UNKNOWN),
    row("platform", `${platform} ${orUnknown(backend?.arch)}`),
    row("os", orUnknown(backend?.os_name)),
  ];

  // Linux only; folded into the version string on the other two.
  if (backend?.os_kernel) lines.push(row("kernel", backend.os_kernel));
  lines.push(row("data dir", orUnknown(backend?.data_dir)));
  lines.push(row("analytics id", analyticsIdValue(analyticsId)));

  lines.push("", "[account]");
  if (!account) {
    lines.push(row("account", "none configured"));
  } else {
    const isOAuth = account.auth_mode === "oauth";
    lines.push(row("gateway", account.gateway_base_url));
    lines.push(row("auth mode", isOAuth ? "constellation sign-in" : "api key"));
    if (isOAuth) {
      lines.push(row("signed in", yesNo(oauth?.signed_in ?? false)));
      lines.push(row("email", orUnknown(oauth?.email)));
      lines.push(row("session expires", expiryPhrase(oauth?.expires_at_unix ?? 0, now)));
    } else {
      // Never the prefix: the point of this section is that the credential
      // itself has no representation in the report at all.
      lines.push(row("key stored", yesNo(account.has_api_key)));
    }
    lines.push(row("organization", account.org_name ?? "none selected"));
    lines.push(row("org id", orUnknown(account.org_id)));
  }

  lines.push("", "[routing]");
  if (!proxy) {
    lines.push(row("proxy", "unavailable on this platform"));
  } else {
    lines.push(row("routing", onOff(proxy.running)));
    lines.push(row("engine port", proxy.port === null ? "none" : String(proxy.port)));
    if (proxy.pac_port !== null) lines.push(row("pac port", String(proxy.pac_port)));
    lines.push(row("certificate", proxy.ca_trusted ? "trusted" : "not trusted"));
    // Trusted with no cert on disk is a broken state the UI cannot show: the
    // OS trusts a root we can no longer mint leaf certs from.
    if (backend && proxy.ca_trusted && !backend.ca_cert_present) {
      lines.push(row("cert file", "MISSING on disk"));
    }
    // Linux only: Chromium-based browsers read a per-user NSS store and never
    // the system one, so the certificate above reading "trusted" while this
    // line appears is the whole of "Firefox works, Chrome doesn't". Silent when
    // the question does not apply, and silent when the store holds it - that is
    // what the line above already says.
    //
    // `unreadable` is its own line rather than folded into the one above it.
    // "CA MISSING" is a claim about the user's store, and printing it for a
    // database Gate never managed to open manufactures that claim out of the
    // absence of a reading - which is the one thing this report must not do,
    // because it is the sentence a support engineer acts on.
    if (backend?.ca_nss_trusted === "absent") {
      lines.push(row("browser store", "CA MISSING (chromium)"));
    } else if (backend?.ca_nss_trusted === "unreadable") {
      lines.push(row("browser store", "could not be read (chromium)"));
    }
    // What the write itself saw, which is the only place the cause lives. The
    // line above is `all()` over the stores and probed now, so it cannot
    // separate "certutil is not installed" from "one database refused" - and
    // the note the app raises on a failed write tells the reader this report
    // names which store and why. Silent before any write in this process, and
    // silent on a clean one: `trusted` is what the line above already says.
    const nssWrite = backend?.ca_nss_write ?? null;
    // No record is not a clean write, and printing nothing for both made the
    // two identical on the page. Said in words rather than left blank, per
    // principle 6: a section that was never read says so. Only where the
    // question applies at all - `ca_nss_trusted` is null off Linux and on a
    // machine with no Chromium store, and a "no record" line there would be
    // answering a question nobody asked.
    if (!nssWrite && backend?.ca_nss_trusted != null) {
      lines.push(row("browser write", "no record for this certificate"));
    } else if (nssWrite && nssWrite.outcome !== "trusted") {
      lines.push(
        row(
          "browser write",
          nssWrite.outcome === "tools_missing"
            ? "FAILED - certutil not installed"
            : "FAILED - a store refused",
        ),
      );
      // One line per store, because the fix differs per store and a count would
      // send the reader back to guessing. `reason` is certutil's own words.
      for (const refusal of nssWrite.refusals) {
        lines.push(row("  refused", `${refusal.store} (${refusal.reason})`));
      }
    }
    lines.push(
      row(
        "env export",
        proxy.env_export_separable
          ? onOff(proxy.env_export_opted_in)
          : `${onOff(proxy.env_export_opted_in)} (not separable here)`,
      ),
    );
  }
  if (backend) {
    // The persisted port is the identity of our proxy address; a tool pointed
    // at a different loopback port is pointed at a dead session.
    lines.push(row("persisted proxy", orUnknown(backend.persisted_engine_proxy_url)));
    lines.push(row("relay", orUnknown(backend.relay_base_url)));
    // Two channels, two readbacks, both from the OS rather than our record:
    // they disagreeing is what "my terminal routes but the desktop app
    // doesn't" looks like from here.
    lines.push(row("env readback", backend.exported_proxy_url ?? "not set"));
    lines.push(row("system proxy", orUnknown(backend.system_proxy)));
    lines.push(row("restore intent", onOff(backend.routing_intent)));
  }
  lines.push(row("stale clients", yesNo(clientsStale)));
  lines.push(
    row(
      "launch at login",
      launchAtLogin
        ? launchAtLogin.pending_disable
          ? `${onOff(launchAtLogin.enabled)} (deregistration pending)`
          : onOff(launchAtLogin.enabled)
        : UNKNOWN,
    ),
  );

  lines.push("", "[running agents]");
  if (!agents) {
    lines.push(row("scan", UNKNOWN));
  } else {
    // The scanned set is named before the result, because the result is only
    // readable against it: "none" means none of *these*, and the scan doesn't
    // cover every integration Gate routes. Without this line an empty section
    // gets read as proof that nothing was running.
    lines.push(row("scanned for", agents.scanned_names.join(", ")));
    if (agents.agents.length === 0) {
      lines.push(row("running", "none"));
    } else {
      for (const agent of agents.agents) lines.push(agentLine(agent, now));
    }
  }

  lines.push("", "[providers]");
  if (providers.length === 0) {
    lines.push("none");
  } else {
    for (const provider of providers) {
      lines.push(
        row(provider.slug, provider.available ? onOff(provider.enabled) : `${onOff(provider.enabled)} (unavailable)`),
      );
    }
  }

  lines.push("", "[tools]");
  if (tools.length === 0) {
    lines.push("none");
  } else {
    for (const tool of tools) {
      // Same three facts as the domain rows below, in the same order, so the
      // two sections can be read against each other - which is the whole point
      // of a tool row and a domain row sharing a client.
      lines.push(
        row(
          tool.slug,
          `${toolStatusLine(tool.status)}${versionSuffix(tool.slug, versions)} - ${tool.client}, ${tool.scope}, ${tool.credential}`,
        ),
      );
    }
  }

  if (proxy) {
    lines.push("", "[proxy domains]");
    if (proxy.domains.length === 0) {
      lines.push("none");
    } else {
      for (const domain of proxy.domains) {
        const state = domain.supported
          ? onOff(domain.enabled)
          : `${onOff(domain.enabled)} (unsupported)`;
        // The taxonomy beside the flag, because on/off alone is what sent the
        // report that prompted this: a reader saw `claude-web off` and could
        // not tell from the report whether that meant their desktop app was
        // uncovered, whether anything else was covering the host, or what
        // turning it on would touch. Client, scope and credential answer all
        // three, and they cost one line each in a report that is already read
        // by whoever is debugging it.
        lines.push(row(domain.slug, `${state} - ${domain.client}, ${domain.scope}, ${domain.credential}`));
      }
    }
  }

  // Said in the artifact itself, not only in the UI that produced it: by the
  // time this is read it has been pasted somewhere, and whoever reads it there
  // should not have to wonder whether it carried a key.
  lines.push("", "No keys, tokens or passwords are included in this report.");

  return `${lines.join("\n")}\n`;
}
