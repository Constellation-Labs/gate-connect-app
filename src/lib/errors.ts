/**
 * Maps a raw Tauri-side error string (Rust core → JS via `invoke`) to a
 * user-facing title + hint. The raw payload stays available in `raw` so
 * the UI can tuck it inside a `<details>` for power users.
 *
 * We pattern-match the macOS / keychain / osascript / network failure
 * modes we see in practice - if nothing matches, the fallback names the
 * action that failed and tells the user the details below may help.
 */
export type ErrorContext =
  | "sign_in"
  | "sign_out"
  | "connect"
  | "forget"
  | "save_api_key"
  | "update"
  | "close_agents"
  | "quit_disable"
  | "proxy_toggle"
  | "provider_toggle"
  | "trust_ca"
  | "untrust_ca"
  | "startup"
  | "account_reconcile"
  | "provider_restore"
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

export function classifyError(rawInput: unknown, context: ErrorContext): ClassifiedError {
  const raw = rawToString(rawInput);
  const lc = raw.toLowerCase();

  // Tauri command not registered / unavailable on this platform.
  if (
    (lc.includes("command") &&
      (lc.includes("not found") || lc.includes("not registered") || lc.includes("not allowed"))) ||
    lc.includes("not available on this platform") ||
    lc.includes("unknown command")
  ) {
    return {
      title: "This action isn't available on your platform.",
      hint: "This tool or action isn't supported here yet. The details below help when reporting it.",
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
    // The verb has to name the button the user actually pressed. A cancelled
    // certificate prompt used to say "Click Connect again" next to a button
    // labelled Trust certificate.
    // The two toggle contexts fire from a role=switch, not a button, and they
    // are the paths a user actually hits: the enable path prompts for admin
    // every time the system proxy changes. They fell through to "Connect",
    // which names no control on Home. Switches get "Flip", buttons get
    // "Click".
    const switchNames: Partial<Record<ErrorContext, string>> = {
      proxy_toggle: "the Routing switch",
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
          : context === "trust_ca"
            ? "Trust certificate"
            : context === "untrust_ca"
              ? "Remove"
              : context === "close_agents" || context === "quit_disable"
                ? "Close everything"
                : "Connect";
    return {
      title: "The system prompt was cancelled",
      hint: `Click ${verb} again and approve your system password prompt.`,
      raw,
    };
  }

  // macOS denied keychain access entirely (rare - system-level block).
  if (lc.includes("user interaction is not allowed") || lc.includes("errsecinteraction")) {
    return {
      title: "The system blocked keychain access",
      hint: "Allow Gate Connect to use your system credential store in your OS privacy settings, then try again.",
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
      title: "Couldn't reach the gateway",
      hint: "Check that you're online and that the gateway URL in Settings is right, then try again.",
      raw,
    };
  }

  // Gateway 401 - wrong API key.
  if (lc.includes("401") || lc.includes("unauthorized")) {
    return {
      title: "Gateway rejected the API key",
      hint: "Replace your Gate API key in Settings.",
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

  // Fallback - tell the user *what* failed at least.
  const titles: Record<ErrorContext, string> = {
    sign_in: "Couldn't save your account",
    sign_out: "Couldn't sign out",
    connect: "Couldn't connect this tool",
    forget: "Couldn't reset Gate Connect",
    save_api_key: "Couldn't save the API key",
    update: "Couldn't install the update",
    close_agents: "Couldn't close the running tools and apps",
    quit_disable: "Couldn't disconnect the tools",
    proxy_toggle: "Couldn't toggle routing",
    provider_toggle: "Couldn't change that app's routing",
    trust_ca: "Couldn't trust the certificate",
    untrust_ca: "Couldn't remove the certificate trust",
    startup: "Couldn't load state at startup",
    account_reconcile: "Couldn't reconcile the saved account",
    provider_restore: "Couldn't restore provider routing",
    provider_disable: "Couldn't disconnect provider routing",
    provider_reconcile: "Couldn't refresh tool configs",
    routing_intent: "Couldn't save the routing preference",
    restore_routing: "Couldn't restore routing at startup",
    launch_at_login: "Couldn't set launch at login",
    generic: "Something went wrong",
  };
  return {
    title: titles[context],
    hint: "Try again. If it keeps failing, the details below help when reporting it.",
    raw,
  };
}
