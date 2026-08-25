import { invoke } from "@tauri-apps/api/core";

/**
 * The front end's door to the diagnostic log.
 *
 * Until this existed the UI was the least observable part of the app: its errors
 * went to a webview console that nobody can read after the fact. A routing
 * switch that silently stopped responding produced no error banner, no stdout
 * line and no file - the only way to see it was to have devtools open at the
 * moment it happened, which is not something a user can be asked to have done.
 *
 * Off in production. The Rust side decides (`logging::enabled`), and a call made
 * when it is off does nothing - so call sites need no guard of their own and
 * cannot drift from the policy.
 *
 * **Never pass a credential, a prompt, or a request body.** The backstop there
 * scrubs one shape of key out of a message; it is not a licence to log secrets.
 */
type Level = "info" | "warn" | "error";

/** Fire-and-forget. Logging must never be the reason something fails, and an
 *  unavailable command - an older binary, a browser context in tests - is not
 *  worth a rejection nobody handles. */
function write(level: Level, message: string): void {
    void invoke("log_message", { level, message }).catch(() => {});
}

export const logInfo = (message: string) => write("info", message);
export const logWarn = (message: string) => write("warn", message);
export const logError = (message: string) => write("error", message);

/** Where the log file is, or null when logging is off. */
export const logFilePath = () => invoke<string | null>("log_file_path").catch(() => null);

/**
 * Turn anything thrown into one line worth reading.
 *
 * `String(err)` on an Error gives "Error: message" and drops the stack, which is
 * the half that says *where*. Tauri rejects with plain strings, so both shapes
 * have to be handled.
 */
export function describe(err: unknown): string {
  if (err instanceof Error) {
    return err.stack ? `${err.message}\n${err.stack}` : err.message;
  }
  return typeof err === "string" ? err : JSON.stringify(err);
}
