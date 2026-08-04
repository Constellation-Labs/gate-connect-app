import { useEffect, useRef, useState } from "react";
import { openExternal } from "../lib/openExternal";
import { GATE_DASHBOARD_URL } from "../lib/config";
import type { Org } from "../lib/api";
import { oauthListOrgs, setOrg } from "../lib/api";
import { trackError } from "../lib/analytics";
import { classifyError, type ClassifiedError } from "../lib/errors";
import { ConstellationHexMark } from "../components/gc/ConstellationHexMark";
import { Button, SubHeader, ErrorNote } from "../components/gc/ui";
import { Icon } from "../components/gc/Icon";

/** Org-selection step, shown right after an OAuth sign-in (and reused by the
 *  Settings "switch organization" action). Fetches the user's orgs; if there's
 *  exactly one it auto-selects and advances, otherwise it renders a picker.
 *  Selecting persists the org (the backend also pushes X-Gate-Org-Id into the
 *  running engine/relay) and calls `onDone`. `onBack`, when provided, shows a
 *  header back button (the Settings switch flow); the post-login flow omits it
 *  since an org must be chosen to route. */
export function OrgPicker({
  onDone,
  onBack,
  onReauth,
  onUseApiKey,
}: {
  onDone: () => void;
  onBack?: () => void;
  onReauth: () => void;
  /** Fall back to the API-key path. The only route forward for a user with no
   * organization and no admin to ask. */
  onUseApiKey?: () => void;
}) {
  const [orgs, setOrgs] = useState<Org[] | null>(null);
  const [error, setError] = useState<ClassifiedError | null>(null);
  const [choosing, setChoosing] = useState<string | null>(null);
  // Guard so the single-org auto-select fires at most once.
  const autoSelected = useRef(false);

  async function choose(org: Org) {
    if (choosing) return;
    setChoosing(org.orgId);
    setError(null);
    try {
      await setOrg(org.orgId, org.name);
      onDone();
    } catch (err) {
      setError(classifyError(err, "generic"));
      trackError(err, "generic");
      setChoosing(null);
    }
  }

  useEffect(() => {
    let alive = true;
    oauthListOrgs()
      .then((list) => {
        if (!alive) return;
        setOrgs(list);
        // Auto-select the only org so the common single-org case is one tap
        // shorter (skipped entirely).
        if (list.length === 1 && !autoSelected.current) {
          autoSelected.current = true;
          void choose(list[0]);
        }
      })
      .catch((err) => {
        if (!alive) return;
        setError(classifyError(err, "generic"));
        trackError(err, "generic");
      });
    return () => {
      alive = false;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // While loading, or while auto-selecting the lone org, show the brand lockup
  // rather than flashing an empty list.
  const loading = orgs === null || (orgs.length === 1 && error === null);

  return (
    <div className="flex grow flex-col">
      {onBack ? (
        <SubHeader title="Choose organization" onBack={onBack} />
      ) : (
        <div className="flex flex-col items-center px-5 pt-7 text-center">
          <ConstellationHexMark size={40} fill="#002a5f" />
          {/* A real, focusable heading: App moves focus to
              [data-screen-focus] on every screen change, and this screen had
              neither PopHeader nor SubHeader, so focus fell to body and the
              screen announced nothing. */}
          <h1
            tabIndex={-1}
            data-screen-focus
            className="mt-3 text-[17px] font-semibold tracking-[-0.02em] text-gc-navy outline-none"
          >
            Choose an organization
          </h1>
          <p className="mt-1.5 max-w-[290px] text-[12.5px] leading-[1.45] text-gc-ink-3">
            Pick which organization to route your Gate traffic through.
          </p>
        </div>
      )}

      <div className="flex flex-col gap-2 px-5 pb-5 pt-4">
        {loading && !error && (
          <div
            role="status"
            className="flex items-center justify-center gap-2 py-8 text-[12.5px] text-gc-ink-3"
          >
            <Icon name="refresh" size={14} className="animate-spin" />
            Loading organizations…
          </div>
        )}

        {error && (
          <div className="flex flex-col gap-2 py-2">
            <ErrorNote error={error} />
            <Button variant="accent" full onClick={onReauth}>
              Sign in again
            </Button>
          </div>
        )}

        {!loading &&
          !error &&
          orgs?.map((org) => (
            <button
              key={org.orgId}
              type="button"
              onClick={() => choose(org)}
              disabled={choosing !== null}
              className="flex items-center gap-3 rounded-[10px] bg-gc-surface p-3.5 text-left shadow-border transition hover:shadow-border-hover disabled:opacity-45"
            >
              <div className="flex h-9 w-9 shrink-0 items-center justify-center rounded-[9px] bg-gc-accent-wash text-gc-accent">
                <Icon name="cube" size={16} />
              </div>
              <div className="min-w-0 flex-1">
                <div className="truncate text-[13px] font-medium text-gc-ink">{org.name}</div>
                <div className="truncate font-mono text-[10.5px] text-gc-ink-3">
                  {[org.slug, org.role].filter(Boolean).join(" · ")}
                </div>
              </div>
              {choosing === org.orgId && (
                <Icon name="check" size={15} className="shrink-0 text-gc-accent" />
              )}
            </button>
          ))}

        {/* PRODUCT.md: "No provisioned org can be assumed; the app must
            self-explain without organizational hand-holding." This state used
            to say "Ask an admin" and offer only sign-out, which for a solo
            developer is no admin and no way forward. Both real options are
            now on the screen. */}
        {!loading && !error && orgs?.length === 0 && (
          <div className="flex flex-col items-center gap-3 py-5 text-center">
            <p className="max-w-[280px] text-[12.5px] leading-snug text-gc-ink-3">
              This account isn&rsquo;t in an organization yet. Create one in the
              Gate dashboard, or use a Gate API key instead.
            </p>
            <div className="flex w-full flex-col gap-2">
              <Button
                variant="accent"
                full
                onClick={() => {
                  void openExternal(GATE_DASHBOARD_URL);
                }}
              >
                Create an organization
              </Button>
              {onUseApiKey && (
                <Button variant="secondary" full onClick={onUseApiKey}>
                  Use a Gate API key instead
                </Button>
              )}
            </div>
          </div>
        )}

        {/* The post-login flow has no back button (an org must be chosen to
            route), but wrong-account is a real dead end without an exit. */}
        {!onBack && !loading && !error && (
          <button
            type="button"
            onClick={onReauth}
            className="mx-auto mt-1 text-[12px] font-medium text-gc-ink-3 transition hover:text-gc-ink"
          >
            Wrong account? Sign out
          </button>
        )}
      </div>
    </div>
  );
}
