import { useEffect, useRef, useState } from "react";
import type { Org } from "../lib/api";
import { oauthListOrgs, setOrg } from "../lib/api";
import { trackError } from "../lib/analytics";
import { ConstellationHexMark } from "../components/gc/ConstellationHexMark";
import { SubHeader } from "../components/gc/ui";
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
}: {
  onDone: () => void;
  onBack?: () => void;
}) {
  const [orgs, setOrgs] = useState<Org[] | null>(null);
  const [error, setError] = useState<string | null>(null);
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
      setError(err instanceof Error ? err.message : String(err));
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
        setError(err instanceof Error ? err.message : String(err));
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
          <div className="mt-3 text-[19px] font-semibold tracking-[-0.025em] text-gc-navy">
            Choose an organization
          </div>
          <p className="mt-1.5 max-w-[290px] text-[12.5px] leading-[1.45] text-gc-ink-3">
            Pick which organization to route your Gate traffic through.
          </p>
        </div>
      )}

      <div className="flex flex-col gap-2 px-5 pb-5 pt-4">
        {loading && !error && (
          <div className="flex items-center justify-center gap-2 py-8 text-[12.5px] text-gc-ink-4">
            <Icon name="refresh" size={14} />
            Loading organizations…
          </div>
        )}

        {error && (
          <p className="py-2 text-[11.5px] leading-snug text-gc-error">{error}</p>
        )}

        {!loading &&
          !error &&
          orgs?.map((org) => (
            <button
              key={org.orgId}
              type="button"
              onClick={() => choose(org)}
              disabled={choosing !== null}
              className="flex items-center gap-3 rounded-[10px] bg-gc-surface p-3.5 text-left shadow-border transition hover:shadow-border-hover disabled:opacity-60"
            >
              <div className="flex h-[30px] w-[30px] shrink-0 items-center justify-center rounded-lg bg-gc-accent-wash text-gc-accent">
                <Icon name="cube" size={16} />
              </div>
              <div className="min-w-0 flex-1">
                <div className="truncate text-[13px] font-medium text-gc-ink">{org.name}</div>
                <div className="truncate font-mono text-[10.5px] text-gc-ink-4">
                  {org.slug} · {org.role}
                </div>
              </div>
              {choosing === org.orgId && (
                <Icon name="check" size={15} className="shrink-0 text-gc-accent" />
              )}
            </button>
          ))}

        {!loading && !error && orgs?.length === 0 && (
          <p className="py-6 text-center text-[12.5px] leading-snug text-gc-ink-3">
            No organizations are available for your account. Ask an admin to add
            you to one, then try again.
          </p>
        )}
      </div>
    </div>
  );
}
