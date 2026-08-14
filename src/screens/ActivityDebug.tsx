import { useCallback, useEffect, useState } from "react";
import { activityOverview } from "../lib/api";
import { SubHeader, Button } from "../components/gc/ui";

/**
 * TEMPORARY (AG-572) — a raw JSON viewer for the gateway's activity overview.
 *
 * This is scaffolding, not the feature. The real screen is blocked on AG-571's
 * design handoff, and several fields in the response are still unsettled
 * (`tokensSaved`, `notices`, and the per-installation scoping). Rendering the
 * payload verbatim lets us exercise the endpoint through the app's real
 * credential path, on the real popover surface, without pretending to a layout
 * nobody has approved yet.
 *
 * Deliberately outside the design system's normal rules: mono everywhere, a
 * scrolling pre block, no status pills. Those rules apply to the shipped
 * screen, and dressing scaffolding up as product is how scaffolding ships by
 * accident. Delete this file when the real Overview lands.
 */
export function ActivityDebug({ onBack }: { onBack: () => void }) {
  const [json, setJson] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);

  const load = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const raw = await activityOverview();
      // Pretty-print when it parses; fall back to the raw text so a
      // non-JSON error body is still visible rather than swallowed.
      try {
        setJson(JSON.stringify(JSON.parse(raw), null, 2));
      } catch {
        setJson(raw);
      }
    } catch (e) {
      setError(String(e));
      setJson(null);
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void load();
  }, [load]);

  return (
    <div className="flex grow flex-col">
      <SubHeader title="Activity (debug)" onBack={onBack} />
      <div className="flex min-h-0 grow flex-col gap-3 px-3.5 pb-4">
        <div className="flex items-center gap-2">
          <Button onClick={() => void load()} disabled={loading}>
            {loading ? "Loading…" : "Refresh"}
          </Button>
          <span className="font-mono text-gc-caption text-gc-ink-3">GET /v1/me/activity</span>
        </div>

        {error && (
          <pre className="overflow-x-auto whitespace-pre-wrap break-words rounded-gc-md bg-gc-subtle p-2.5 font-mono text-gc-caption text-gc-error-deep">
            {error}
          </pre>
        )}

        {json && (
          <pre className="min-h-0 grow overflow-auto rounded-gc-md bg-gc-sunken p-2.5 font-mono text-gc-caption leading-relaxed text-gc-ink">
            {json}
          </pre>
        )}
      </div>
    </div>
  );
}
