import { openUrl } from "@tauri-apps/plugin-opener";
import { trackError } from "./analytics";

/** Open a URL in the user's browser, reporting failures instead of dropping
 * them.
 *
 * Every call site used to be `void openUrl(...)`, so a rejection became an
 * unhandled promise and the button appeared to do nothing. That is exactly how
 * the dashboard link stayed broken: the opener ACL rejected its unslashed URL
 * and nothing said so. */
export async function openExternal(url: string): Promise<void> {
  try {
    await openUrl(url);
  } catch (err) {
    // Nowhere useful to render this - the footer link has no error surface -
    // but it must reach the log and analytics rather than vanish.
    console.error("[gate] failed to open", url, err);
    trackError(err, "generic");
  }
}
