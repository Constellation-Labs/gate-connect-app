/**
 * Maps a raw Tauri-side error string (Rust core → JS via `invoke`) to a
 * user-facing title + hint. The raw payload stays available in `raw` so
 * the UI can tuck it inside a `<details>` for power users.
 *
 * We pattern-match the macOS / keychain / osascript / network failure
 * modes we see in practice - if nothing matches, the fallback names the
 * action that failed and tells the user the details below may help.
 */
import type { AuthMode } from "./api";
import { currentPlatform, secretStoreName } from "./platform";

export type ErrorContext =
  | "sign_in"
  | "sign_out"
  | "connect"
  | "forget"
  | "save_api_key"
  | "update"
  | "close_agents"
  | "proxy_toggle"
  | "provider_toggle"
  | "trust_ca"
  | "env_export"
  | "untrust_ca"
  | "startup"
  | "account_reconcile"
  | "provider_restore"
  /** Opening a link in the user's browser failed: an opener-ACL miss, no
   *  browser, a sandbox refusal. Its own context because the remedy is nothing
   *  to do with Gate - the user can still copy the address - and because it used
   *  to vanish into a console nobody reads. */
  | "open_external"
  /** The re-read that follows a routing write. Its own context because a failed
   *  resync is not a failed write - the write landed, and only the view of it is
   *  stale. It used to be invisible: thrown from a `finally` into a `void` call
   *  site, where it took every switch in the app down with it. */
  | "resync"
  | "provider_disable"
  | "provider_reconcile"
  | "routing_intent"
  | "restore_routing"
  | "launch_at_login"
  | "generic";

/** Contexts the Rust side is allowed to report through `drain_backend_errors`.
 * An unknown string (a backend site added without a frontend counterpart)
 * degrades to "generic" rather than sending an unvetted label. */
const BACKEND_CONTEXTS = new Set<ErrorContext>([
  "account_reconcile",
  "provider_restore",
  "provider_disable",
  "provider_reconcile",
  "routing_intent",
  "restore_routing",
  "launch_at_login",
]);

export function backendErrorContext(context: string): ErrorContext {
  return BACKEND_CONTEXTS.has(context as ErrorContext) ? (context as ErrorContext) : "generic";
}

export interface ClassifiedError {
  title: string;
  hint: string;
  raw: string;
}

/**
 * Normalize an unknown error payload into a searchable string. Errors come
 * across the Tauri boundary as plain strings, but JS-side throws and
 * structured Rust errors can arrive as Error instances or objects, where
 * `String(x)` would collapse to "[object Object]" and match nothing.
 */
function rawToString(rawInput: unknown): string {
  if (typeof rawInput === "string") return rawInput;
  if (rawInput instanceof Error) return rawInput.message;
  if (rawInput && typeof rawInput === "object") {
    const obj = rawInput as Record<string, unknown>;
    const field = obj.message ?? obj.error ?? obj.body;
    if (typeof field === "string") return field;
    try {
      return JSON.stringify(rawInput);
    } catch {
      return String(rawInput);
    }
  }
  return String(rawInput);
}

/**
 * Whether an error names a loopback bind that failed because the port is taken.
 *
 * Prose, because invoke rejections cross the IPC as strings; where the backend
 * still holds the error value it decides this by type instead
 * (`core::analytics::failure_reason`, the `reason` on a drained backend error).
 * The phrases are the ones that reach the webview: the relay's own sentence
 * ("the relay port N is already in use"), the engine's synthetic `AddrInUse`
 * ("address in use"), and the OS's own on each platform - "Address already in
 * use (os error 48)" on macOS, 98 on Linux, and Windows' sentence about socket
 * addresses with error 10048.
 */
function isPortInUse(lc: string): boolean {
  return (
    lc.includes("already in use") ||
    lc.includes("address in use") ||
    lc.includes("only one usage of each socket address") ||
    /os error (48|98|10048)\b/.test(lc)
  );
}

/**
 * Why a step of connecting failed, as the closed set the `connection_failed`
 * analytics event carries (AG-960). Ordered from the most specific cause to the
 * least, and never the raw text: the reason is a label from this list, so a
 * host or a path in the message cannot reach the event.
 *
 * - `port_in_use`: a loopback port Gate binds is held by another process.
 * - `cowork_setting_missing`: Claude Desktop keeps local Cowork off, so routing
 *   the Claude row cannot reach a Cowork task. Never produced from an error
 *   string; the connect path reads Claude's settings and reports it directly.
 * - `routing_off`: a proxy-routed tool refused because the engine is not running.
 * - `ca_trust_declined`: the OS certificate prompt was declined or dismissed.
 * - `prompt_declined`: another OS prompt (the admin prompt for the system proxy)
 *   was cancelled.
 * - `offline`: the gateway could not be reached.
 * - `auth_rejected`: the gateway refused the session or the key (401, or a
 *   typed `rejected` code).
 * - `sign_in_not_completed`: the browser sign-in was declined or abandoned.
 * - `unknown`: none of the above.
 */
export type ConnectionFailureReason =
  | "port_in_use"
  | "cowork_setting_missing"
  | "routing_off"
  | "ca_trust_declined"
  | "prompt_declined"
  | "offline"
  | "auth_rejected"
  | "sign_in_not_completed"
  | "unknown";

export const CONNECTION_FAILURE_REASONS: readonly ConnectionFailureReason[] = [
  "port_in_use",
  "cowork_setting_missing",
  "routing_off",
  "ca_trust_declined",
  "prompt_declined",
  "offline",
  "auth_rejected",
  "sign_in_not_completed",
  "unknown",
];

/** Read a reason the backend sent (`BackendError.reason`), refusing anything
 *  outside the closed set rather than passing an unvetted label through. */
export function knownConnectionFailureReason(value: unknown): ConnectionFailureReason | null {
  return CONNECTION_FAILURE_REASONS.includes(value as ConnectionFailureReason)
    ? (value as ConnectionFailureReason)
    : null;
}

/**
 * Map an error to its `connection_failed` reason. The same pattern families as
 * `classifyError`, in the same order where they overlap, so the reason and the
 * title a user reads can never disagree about what happened.
 */
export function connectionFailureReason(
  rawInput: unknown,
  context: ErrorContext,
): ConnectionFailureReason {
  const text = rawToString(rawInput);
  // A typed `gateway_api::FailureCode` envelope, where the command sent one:
  // the code decides, not the words in its message.
  const code = failureCodeOf(text);
  if (code === "rejected") return "auth_rejected";
  if (code === "offline") return "offline";
  const lc = text.toLowerCase();
  if (isPortInUse(lc)) return "port_in_use";
  if (lc.includes("proxy is not running")) return "routing_off";
  if (lc.includes("certificate trust dialog was cancelled")) return "ca_trust_declined";
  // The browser sign-in's own Cancel and its five-minute give-up, ahead of the
  // prompt and network arms that their words would otherwise match.
  if (
    lc.includes("access_denied") ||
    lc.includes("access denied") ||
    lc.includes("login redirect") ||
    lc.includes("waiting for the login")
  ) {
    return "sign_in_not_completed";
  }
  if (
    lc.includes("user canceled") ||
    lc.includes("user cancelled") ||
    // osascript's cancel code as it prints it - "(-128)" or "error -128" - and
    // never a bare substring, which a port or an id can contain.
    /\(-128\)|\berror -128\b/.test(lc) ||
    (lc.includes("authorization") && lc.includes("denied"))
  ) {
    return context === "trust_ca" ? "ca_trust_declined" : "prompt_declined";
  }
  if (
    lc.includes("connection refused") ||
    lc.includes("dns") ||
    lc.includes("timed out") ||
    lc.includes("timeout") ||
    lc.includes("network is unreachable")
  ) {
    return "offline";
  }
  if (isUnauthorized(lc)) return "auth_rejected";
  return "unknown";
}

/**
 * A 401 as the backend words one: "returned 401 Unauthorized", "status 401",
 * "HTTP 401", or the word "unauthorized" on its own. A bare `401` is not
 * enough: it is a substring of ports (`:40199`) and ids.
 */
function isUnauthorized(lc: string): boolean {
  return (
    /\bunauthori[sz]ed\b/.test(lc) ||
    /\b(?:returned|status|http|code)\b[\s:=]*401\b/.test(lc) ||
    /\b401 unauthori/.test(lc)
  );
}

/** The `code` of a JSON failure envelope (`gateway_api::Failure`), if that is
 *  what the rejection is. */
function failureCodeOf(text: string): string | null {
  if (!text.startsWith("{")) return null;
  try {
    const parsed = JSON.parse(text) as { code?: unknown };
    return typeof parsed.code === "string" ? parsed.code : null;
  } catch {
    return null;
  }
}

/** Prefix of the error a gateway switch reports when the switch landed and
 *  only the relaunch after it failed, so the copy can say the move is done. */
export const SWITCHED_RELAUNCH_FAILED = "gateway switched; relaunch failed after moving to";

export function classifyError(
  rawInput: unknown,
  context: ErrorContext,
  /** The account's auth mode, where the caller knows it. The gateway's 401 is
   * the same string for a revoked OAuth session and a bad API key, but the
   * remedies are different screens - and telling an OAuth user to replace a
   * key points them at a control Settings does not render for them. */
  authMode?: AuthMode,
): ClassifiedError {
  const raw = rawToString(rawInput);
  const lc = raw.toLowerCase();

  // A gateway switch that landed and then could not relaunch. Ahead of every
  // other branch: the relaunch error's own words could match any of them, and
  // none would say the one thing that matters - the switch is done.
  if (raw.startsWith(SWITCHED_RELAUNCH_FAILED)) {
    return {
      title: "Gateway switched, but Gate Connect couldn’t relaunch",
      hint: "Quit Gate Connect and open it again to finish switching.",
      raw,
    };
  }

  // Tauri command not registered / unavailable on this platform.
  if (
    (lc.includes("command") &&
      (lc.includes("not found") || lc.includes("not registered") || lc.includes("not allowed"))) ||
    lc.includes("not available on this platform") ||
    lc.includes("unknown command")
  ) {
    return {
      title: "This action isn’t available on your platform.",
      hint: "Not supported here yet. The details below help when reporting it.",
      raw,
    };
  }

  // The proxy-routed tools (OpenClaw, Hermes, the environment channel) refuse
  // to connect while the engine is down, by design: handing a tool's whole
  // egress to a dead port breaks it outright rather than merely leaving it
  // un-routed. Without this branch the generic fallback answers "try again",
  // which is precisely wrong - retrying cannot help until routing is on, and
  // the one sentence that says so is buried in the details disclosure.
  if (isRoutingOffRefusal(raw)) {
    return {
      title: "Turn on “Route through Gate” first",
      hint: "This tool routes through Gate’s local proxy, so turn routing on first.",
      raw,
    };
  }

  // The Windows certificate trust dialog was declined or dismissed
  // (`ca_windows.rs` bails with this sentence when certutil -addstore exits
  // non-zero). certutil reports no "user cancelled" string of its own, so this
  // fell past the cancelled-prompt branch below all the way to the generic
  // fallback, which answered "Try again. If it keeps failing, the details
  // below help when reporting it." - advice for a bug, on the one failure that
  // is the user's own deliberate No. Names the dialog and the button instead.
  if (lc.includes("certificate trust dialog was cancelled")) {
    return {
      title: "The certificate wasn’t trusted",
      hint: "Click Trust again and choose Yes in the Windows security warning. Until then, some apps can’t route.",
      raw,
    };
  }

  // The sign-in page's own Cancel, which is a browser button and not a system
  // prompt.
  //
  // Ahead of the prompt branch below, and that order is the fix: Cognito
  // answers a declined authorization with `access_denied`, `oauth.rs` wraps it
  // as "authorization failed (access_denied)", and the branch below matches
  // "authorization" AND "denied" - so pressing Cancel in the browser produced
  // "The system prompt was cancelled - approve your system password prompt",
  // naming a dialog the user never saw. It is the same misdiagnosis as the
  // timeout one below, on the likelier path: declining takes a click, walking
  // away takes five minutes.
  if (lc.includes("access_denied") || lc.includes("access denied")) {
    return {
      title: "The sign-in was declined",
      hint: "Try again and approve it in the browser.",
      raw,
    };
  }

  // Auth prompt cancelled (macOS osascript exits -128; the Windows and Linux
  // credential prompts report their own cancels through the same branch).
  if (
    lc.includes("user canceled") ||
    lc.includes("user cancelled") ||
    lc.includes("-128") ||
    (lc.includes("authorization") && lc.includes("denied"))
  ) {
    // The verb has to name the control the user actually touched. A cancelled
    // certificate prompt used to say "Click Connect again" next to a button
    // labelled Trust certificate. Switches get "Flip", buttons get "Click".
    //
    // Starting the engine and trusting the certificate are each reached from
    // more than one control - a notice's switch, setup's button, the dialog in
    // front of a connect - so those two say what to do again rather than name
    // one control and be wrong about the others.
    const retryHints: Partial<Record<ErrorContext, string>> = {
      proxy_toggle: "Turn routing on again and approve your system password prompt.",
      trust_ca: "Try again and approve your system password prompt.",
    };
    const retryHint = retryHints[context];
    if (retryHint) {
      return { title: "The system prompt was cancelled", hint: retryHint, raw };
    }
    const switchNames: Partial<Record<ErrorContext, string>> = {
      provider_toggle: "that switch",
    };
    const switchName = switchNames[context];
    if (switchName) {
      return {
        title: "The system prompt was cancelled",
        hint: `Flip ${switchName} again and approve your system password prompt.`,
        raw,
      };
    }
    const verb =
      context === "forget"
        ? "Reset"
        : context === "sign_out"
          ? "Sign out"
          : context === "untrust_ca"
            ? "Remove"
            : context === "close_agents"
              ? "Close everything"
              : "Connect";
    return {
      title: "The system prompt was cancelled",
      hint: `Click ${verb} again and approve your system password prompt.`,
      raw,
    };
  }

  // The OS denied secret-store access entirely (rare - system-level block).
  if (lc.includes("user interaction is not allowed") || lc.includes("errsecinteraction")) {
    const store = secretStoreName(currentPlatform(), "the");
    return {
      title: `The system blocked access to ${store}`,
      hint: `Allow Gate Connect to use ${store} in your OS privacy settings, then try again.`,
      raw,
    };
  }

  // The browser login was never finished.
  //
  // Ahead of the network branch on purpose: `oauth.rs` gives up after
  // `LOGIN_TIMEOUT_SECS` with "timed out waiting for the login redirect", and
  // that sentence contains "timed out", so the connectivity arm below claimed
  // it and told the user to check that they were online and that the gateway
  // URL was right. Neither was the problem - the gateway was never asked. The
  // most common cause is the one that produced this: the sign-in page opened in
  // a browser profile the person was not signed into, and they walked away from
  // it.
  if (lc.includes("login redirect") || lc.includes("waiting for the login")) {
    return {
      title: "The browser sign-in was not finished",
      hint: "Gate stopped waiting after five minutes. Try again and finish in the browser.",
      raw,
    };
  }

  // One of Gate's loopback ports is held by another process: a second copy of
  // the app, or `gate-connect proxy relay`. The relay refuses to move off its
  // persisted port rather than silently strand every tool config pointing at it
  // (`relay.rs`'s `bind_relay`), so retrying cannot help until the other one
  // stops - which is what the generic fallback's "Try again" never said. Ahead
  // of the network branch because none of these words overlap it, and ahead of
  // nothing else for the same reason.
  if (isPortInUse(lc)) {
    return {
      title: "Gate’s local port is already in use",
      hint: "Another Gate Connect or gate-connect relay is using it. Quit it and try again.",
      raw,
    };
  }

  // Network: gateway unreachable.
  if (
    lc.includes("connection refused") ||
    lc.includes("dns") ||
    lc.includes("timed out") ||
    lc.includes("timeout") ||
    lc.includes("network is unreachable")
  ) {
    return {
      title: "Couldn’t reach the gateway",
      hint: "Check your connection and the gateway URL in Settings.",
      raw,
    };
  }

  // Gateway 401 - the session or the key was rejected.
  if (lc.includes("401") || lc.includes("unauthorized")) {
    return {
      title: authMode === "oauth" ? "Gateway rejected your session" : "Gateway rejected the API key",
      // Setup has no Settings to send anyone to: the sign-in and key panes are
      // the remedy, and they are on screen.
      hint:
        authMode === "oauth"
          ? context === "sign_in"
            ? "Sign in again, or use a different account."
            : "Sign in again from Settings."
          : authMode === "api_key"
            ? context === "sign_in"
              ? "Check that the key is for this gateway, then paste it again."
              : "Replace your Gate API key in Settings."
            : "Open Settings and reconnect: sign in again, or replace your Gate API key.",
      raw,
    };
  }

  // Disk space (writing tool config files).
  if (lc.includes("no space") || lc.includes("disk full")) {
    return {
      title: "Not enough disk space",
      hint: "Free up some disk space and try again.",
      raw,
    };
  }

  // A tool write an integration refused for a reason the user can fix: "No
  // supported OpenCode providers found to route through Gate. Run ...". The
  // body says what to do rather than "Try again", which hid the instruction
  // behind Details (staging QA, 2026-09-30). The title stays the generic one.
  if (context === "connect") {
    const hint = connectRefusalHint(raw);
    if (hint) return { title: "Couldn’t connect this tool", hint, raw };
  }

  // Fallback - tell the user *what* failed at least.
  const titles: Record<ErrorContext, string> = {
    // The write already succeeded; only the re-read of it failed, so this says
    // the rows may be stale rather than implying the change did not land.
    resync: "Couldn’t refresh what’s on screen",
    open_external: "Couldn’t open that link",
    // Three of the four `sign_in` call sites are browser flows, not writes:
    // the OAuth offer, Settings’ switch to a Gate account, and first run’s
    // sign-in. "Couldn’t save your account" described the fourth and read as
    // a non sequitur after a Cognito round-trip that never saved anything.
    sign_in: "Couldn’t complete sign-in",
    sign_out: "Couldn’t sign out",
    connect: "Couldn’t connect this tool",
    forget: "Couldn’t reset Gate Connect",
    save_api_key: "Couldn’t save the API key",
    update: "Couldn’t install the update",
    close_agents: "Couldn’t restart the running tools and apps",
    proxy_toggle: "Couldn’t toggle routing",
    provider_toggle: "Couldn’t change that app’s routing",
    trust_ca: "Couldn’t trust the certificate",
    env_export: "Couldn’t change the command-line tool setting",
    untrust_ca: "Couldn’t remove the certificate trust",
    startup: "Couldn’t load state at startup",
    account_reconcile: "Couldn’t reconcile the saved account",
    provider_restore: "Couldn’t restore provider routing",
    provider_disable: "Couldn’t disconnect provider routing",
    provider_reconcile: "Couldn’t refresh tool configs",
    routing_intent: "Couldn’t save the routing preference",
    restore_routing: "Couldn’t restore routing at startup",
    launch_at_login: "Couldn’t set launch at login",
    generic: "Something went wrong",
  };
  return {
    title: titles[context],
    hint: "Try again, or report it with the details below.",
    raw,
  };
}

/**
 * The window's copy for a connect an integration refused, by the refusal's
 * own wording. `null` for anything not listed, which keeps "Try again".
 *
 * A list, not a test of the message's shape: a backend message reaches the
 * body only once someone has written copy for it. Reading any one-sentence
 * error as an instruction also let through paths ("failed to write
 * ~/.codex/config.toml"), parser output and network errors. And the backend's
 * own sentences are written for the CLI ("then re-run connect"), so the window
 * says it in its own words. The patterns follow the `bail!`s in
 * `crates/core/src/integrations/`; a reworded refusal falls back to "Try
 * again", with the full text still under Details.
 */
const CONNECT_REFUSALS: readonly [RegExp, (m: RegExpMatchArray) => string][] = [
  [
    /^No supported OpenCode providers found to route through Gate\b/,
    () =>
      "OpenCode has no provider Gate can route. Run opencode auth login, then try again.",
  ],
  [
    /^None of the configured OpenCode providers can route through Gate yet \(([^)]*)\)/,
    (m) => `None of OpenCode’s providers can route through Gate yet (${m[1]}).`,
  ],
  [
    /^Codex isn't logged in yet\b/,
    () => "Codex isn’t signed in. Run codex login, then try again.",
  ],
  [
    /^Hermes already has its own proxy settings in ~\/\.hermes\/\.env\b/,
    () =>
      "Hermes has its own proxy settings in ~/.hermes/.env. Remove them to route it through Gate.",
  ],
  [
    /^(Claude Code|Codex|OpenCode|OpenClaw|Hermes) is not installed\b/,
    (m) => `${m[1]} isn’t installed. Install it, then try again.`,
  ],
];

export function connectRefusalHint(raw: string): string | null {
  const text = raw.trim();
  for (const [pattern, hint] of CONNECT_REFUSALS) {
    const m = text.match(pattern);
    if (m) return hint(m);
  }
  return null;
}

/**
 * A tool's connect refused because routing is off. The fix is the whole
 * install's ("Turn on Route through Gate first"), so the window says it rather
 * than one app's pane (review on #390).
 */
export function isRoutingOffRefusal(raw: string): boolean {
  return raw.toLowerCase().includes("proxy is not running");
}
