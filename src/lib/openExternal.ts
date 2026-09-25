import { openUrl } from "@tauri-apps/plugin-opener";
import { trackError } from "./analytics";
import { classifyError, type ClassifiedError } from "./errors";
import { logWarn } from "./log";

/**
 * Open a URL in the user's browser.
 *
 * Returns `null` when it opened, or a classified error the caller can render.
 *
 * Every call site used to be `void openUrl(...)`, so a rejection became an
 * unhandled promise and the button appeared to do nothing. That is exactly how
 * the dashboard link stayed broken: the opener ACL rejected its unslashed URL
 * and nothing said so.
 *
 * Catching it was the first half of the fix, but the error still only reached
 * `console.error` - a webview console nobody can read after the fact - so a
 * failure was observable in principle and invisible in practice. Now it goes to
 * the diagnostic log, and comes back so a surface can show it. Callers that
 * genuinely have nowhere to render keep working: they ignore the return, and the
 * log still has the line.
 */
export async function openExternal(url: string): Promise<ClassifiedError | null> {
  try {
    await openUrl(url);
    return null;
  } catch (err) {
    logWarn(`failed to open ${url}: ${String(err)}`);
    trackError(err, "open_external");
    return classifyError(err, "open_external");
  }
}
