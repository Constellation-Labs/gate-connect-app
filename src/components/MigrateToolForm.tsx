import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type { MigrateBlocker, MigrateDiscover, MigrateOptions, MigrateReport, Tool } from "../lib/api";
import { ErrorBlock } from "./ErrorBlock";
import { allMigrateOptions, migrateDiscover, migrateExecute, migratePreview } from "../lib/api";

interface Props {
  tool: Tool;
  onClose: () => void;
  onDone: () => void;
}

type Phase = "discovering" | "ready" | "blocked" | "previewing" | "running" | "done" | "error";

interface CategoryDef {
  key: keyof MigrateOptions;
  label: string;
  describe: (d: MigrateDiscover) => string;
  available: (d: MigrateDiscover) => boolean;
}

const CATEGORIES: CategoryDef[] = [
  {
    key: "include_plugins",
    label: "Org plugins & skills",
    describe: (d) => `${d.plugins} bundle${d.plugins === 1 ? "" : "s"} (org plugins + standalone skills)`,
    available: (d) => d.plugins > 0,
  },
  {
    key: "include_scheduled",
    label: "Scheduled tasks",
    describe: (d) => `${d.scheduled} task${d.scheduled === 1 ? "" : "s"}`,
    available: (d) => d.scheduled > 0,
  },
  {
    key: "include_conversations",
    label: "Conversations",
    describe: (d) => `${d.conversations} conversation${d.conversations === 1 ? "" : "s"}`,
    available: (d) => d.conversations > 0,
  },
  {
    key: "include_memory",
    label: "Memory & spaces",
    describe: (d) => (d.has_memory ? "Available" : "None on disk"),
    available: (d) => d.has_memory,
  },
  {
    key: "include_enabled_plugins",
    label: "Enabled-plugin state",
    describe: (d) => (d.has_enabled_plugins ? "cowork_settings.json" : "None on disk"),
    available: (d) => d.has_enabled_plugins,
  },
  {
    key: "include_preferences",
    label: "Preferences",
    describe: (d) => (d.has_preferences ? "Trusted folders, feature flags" : "None on disk"),
    available: (d) => d.has_preferences,
  },
  {
    key: "include_artifacts",
    label: "Artifacts",
    describe: (d) => `${d.artifacts} file${d.artifacts === 1 ? "" : "s"}`,
    available: (d) => d.artifacts > 0,
  },
];

/**
 * Modal-on-modal overlay that drives the standard→3P data migration.
 * Pre-flight runs on mount; the user picks categories; clicking Run
 * shells through to `migrate_execute`. Single-shot report on completion.
 */
export function MigrateToolForm({ tool, onClose, onDone }: Props) {
  const [phase, setPhase] = useState<Phase>("discovering");
  const [discover, setDiscover] = useState<MigrateDiscover | null>(null);
  const [options, setOptions] = useState<MigrateOptions>(allMigrateOptions());
  const [report, setReport] = useState<MigrateReport | null>(null);
  const [preview, setPreview] = useState<MigrateReport | null>(null);
  const [error, setError] = useState<unknown | null>(null);

  // Unmount guard — migrate_execute runs a long admin password prompt;
  // the user can close the popover mid-flight.
  const mounted = useRef(true);
  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
    };
  }, []);

  const refreshDiscover = useCallback(async () => {
    setPhase("discovering");
    setError(null);
    try {
      const d = await migrateDiscover(tool.slug);
      if (!mounted.current) return;
      setDiscover(d);
      // Default every category to its availability — no point asking the
      // user to confirm copying categories that don't exist.
      setOptions((prev) => ({
        ...prev,
        include_plugins: d.plugins > 0,
        include_scheduled: d.scheduled > 0,
        include_conversations: d.conversations > 0,
        include_memory: d.has_memory,
        include_enabled_plugins: d.has_enabled_plugins,
        include_preferences: d.has_preferences,
        include_artifacts: d.artifacts > 0,
      }));
      setPhase(d.ready ? "ready" : "blocked");
    } catch (err) {
      if (!mounted.current) return;
      setError(err);
      setPhase("error");
    }
  }, [tool.slug]);

  useEffect(() => {
    refreshDiscover();
  }, [refreshDiscover]);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape" && phase !== "running") onClose();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose, phase]);

  const anySelected = useMemo(
    () =>
      options.include_plugins ||
      options.include_scheduled ||
      options.include_conversations ||
      options.include_memory ||
      options.include_enabled_plugins ||
      options.include_preferences ||
      options.include_artifacts,
    [options],
  );

  // Any change to the selected categories invalidates a prior preview —
  // force the user to re-preview before they can run.
  useEffect(() => {
    setPreview(null);
  }, [options]);

  const runPreview = async () => {
    setPhase("previewing");
    setError(null);
    try {
      const r = await migratePreview(tool.slug, { ...options, dry_run: true });
      if (!mounted.current) return;
      setPreview(r);
      setPhase("ready");
    } catch (err) {
      if (!mounted.current) return;
      setError(err);
      setPhase("ready");
    }
  };

  const run = async () => {
    if (!preview) return;
    if (
      !window.confirm(
        `This will copy your ${tool.name} data into gateway mode. This can't be undone automatically. Continue?`,
      )
    ) {
      return;
    }
    setPhase("running");
    setError(null);
    try {
      const r = await migrateExecute(tool.slug, { ...options, dry_run: false });
      if (!mounted.current) return;
      setReport(r);
      setPhase("done");
    } catch (err) {
      if (!mounted.current) return;
      setError(err);
      setPhase("ready");
    }
  };

  return (
    <div className="absolute inset-0 z-20 flex flex-col bg-white">
      <header className="shrink-0 flex items-center gap-2 border-b border-ink-100 px-3.5 py-3">
        <button
          type="button"
          onClick={onClose}
          disabled={phase === "running"}
          className="rounded-[4px] p-1 text-ink-500 transition-colors hover:bg-ink-100 hover:text-ink-900 disabled:opacity-50"
          aria-label="Back"
          title="Back to tool"
        >
          <svg
            viewBox="0 0 24 24"
            className="h-3.5 w-3.5"
            fill="none"
            stroke="currentColor"
            strokeWidth="2"
            strokeLinecap="round"
            strokeLinejoin="round"
          >
            <path d="M15 18l-6-6 6-6" />
          </svg>
        </button>
        <div className="min-w-0">
          <div className="truncate text-[13px] font-semibold leading-tight tracking-[-0.01em] text-ink-900">
            Bring over your {tool.name} data
          </div>
          <div className="text-[11px] leading-tight text-ink-500">Copies once into the new gateway-mode app data</div>
        </div>
      </header>

      <div className="flex-1 overflow-y-auto px-3.5 py-3 space-y-3">
        <PreflightBanner phase={phase} discover={discover} onRetry={refreshDiscover} />

        {discover && discover.roots && phase !== "done" && <PathsRow discover={discover} />}

        {phase === "done" && report ? (
          <ReportView report={report} />
        ) : phase === "discovering" ? (
          <div className="rounded-md bg-ink-50 px-3 py-2 text-[12px] text-ink-700 shadow-[inset_0_0_0_1px_oklch(0.96_0_0)]">
            Scanning the standard-mode app data…
          </div>
        ) : discover ? (
          <CategoryChecklist
            discover={discover}
            options={options}
            disabled={phase === "running" || phase === "blocked"}
            onToggle={(key) => setOptions((prev) => ({ ...prev, [key]: !prev[key] }))}
          />
        ) : null}

        {preview && phase !== "done" && (
          <div className="space-y-1.5">
            <div className="text-[11px] font-semibold uppercase tracking-wide text-ink-500">
              Preview — what will be copied
            </div>
            <ReportView report={preview} />
            <p className="text-[11px] leading-relaxed text-ink-500">
              Nothing has been changed yet. Review the counts above, then click Confirm &amp; run to migrate.
            </p>
          </div>
        )}

        {error !== null && <ErrorBlock error={error} context="migrate" />}

        {phase !== "done" && (
          <p className="text-[11px] leading-relaxed text-ink-500">
            Plugins land in{" "}
            <code className="inline-flex items-center rounded-[4px] bg-ink-100 px-1.5 py-0.5 font-mono text-[10.5px] tracking-[-0.01em] text-ink-900">
              /Library/Application Support/Claude/org-plugins
            </code>
            , which is system-owned — macOS will ask for your password once. Everything else copies into your home
            folder without a prompt. Safe to re-run: existing gateway-mode entries are never overwritten.
          </p>
        )}
      </div>

      <footer className="shrink-0 flex justify-end gap-2 border-t border-ink-100 px-3.5 py-2.5">
        {phase === "done" ? (
          <button
            type="button"
            onClick={() => onDone()}
            className="inline-flex h-8 items-center justify-center rounded-md bg-ink-900 px-3.5 text-[12px] font-medium tracking-[-0.005em] text-white transition-colors hover:bg-ink-800"
          >
            Done
          </button>
        ) : (
          <>
            <button
              type="button"
              onClick={onClose}
              disabled={phase === "running"}
              className="inline-flex h-8 items-center justify-center rounded-md px-3 text-[12px] text-ink-700 transition-colors hover:text-ink-900 disabled:opacity-50"
            >
              Cancel
            </button>
            <button
              type="button"
              onClick={runPreview}
              disabled={phase !== "ready" || !anySelected}
              className="inline-flex h-8 items-center justify-center rounded-md px-3.5 text-[12px] font-medium tracking-[-0.005em] text-ink-900 shadow-[inset_0_0_0_1px_oklch(0.9_0_0)] transition-colors hover:bg-ink-50 disabled:cursor-not-allowed disabled:opacity-50"
            >
              {phase === "previewing" ? "Previewing…" : preview ? "Re-preview" : "Preview"}
            </button>
            <button
              type="button"
              onClick={run}
              disabled={phase !== "ready" || !anySelected || !preview}
              className="inline-flex h-8 items-center justify-center rounded-md bg-ink-900 px-3.5 text-[12px] font-medium tracking-[-0.005em] text-white transition-colors hover:bg-ink-800 disabled:cursor-not-allowed disabled:opacity-50"
            >
              {phase === "running" ? "Running…" : "Confirm & run"}
            </button>
          </>
        )}
      </footer>
    </div>
  );
}

function PreflightBanner({
  phase,
  discover,
  onRetry,
}: {
  phase: Phase;
  discover: MigrateDiscover | null;
  onRetry: () => void;
}) {
  if (phase === "discovering" || phase === "error") return null;

  if (phase === "done") {
    return (
      <div className="rounded-md bg-success-50 p-3 text-[12px] text-success-800 shadow-[inset_0_0_0_1px_oklch(0.925_0.084_155.995)]">
        <div className="font-medium">Migration complete</div>
        <p className="mt-1 text-[11px] text-success-700">Quit and relaunch Cowork to see the imported data.</p>
      </div>
    );
  }

  const blockers = discover?.blockers ?? [];
  if (blockers.length === 0) {
    return (
      <div className="rounded-md bg-success-50 p-3 text-[12px] text-success-800 shadow-[inset_0_0_0_1px_oklch(0.925_0.084_155.995)]">
        <div className="font-medium">Ready to migrate</div>
        <p className="mt-1 text-[11px] text-success-700">{sizeLabel(discover?.bytes_estimated ?? 0)} to copy.</p>
      </div>
    );
  }

  return (
    <div className="rounded-md bg-warning-50 p-3 text-[12px] text-warning-800 shadow-[inset_0_0_0_1px_oklch(0.924_0.12_95.746)]">
      <div className="font-medium">Can't migrate yet</div>
      <ul className="mt-1 space-y-1 text-[11px] text-warning-700">
        {blockers.map((b, i) => (
          <li key={i}>{describeBlocker(b)}</li>
        ))}
      </ul>
      <button
        type="button"
        onClick={onRetry}
        className="mt-2 inline-flex h-7 items-center rounded-md bg-white px-3 text-[11px] text-ink-900 shadow-border transition-all hover:bg-ink-50 hover:shadow-border-hover"
      >
        Retry
      </button>
    </div>
  );
}

function describeBlocker(b: MigrateBlocker): string {
  switch (b.kind) {
    case "cowork_running":
      return "Cowork is running. Press ⌘Q in Cowork to fully quit it, then click Retry.";
    case "source_missing":
      return "No standard-mode Cowork data found on this Mac. Sign in to Cowork in standard mode first, then come back.";
    case "dest_missing":
      return "No gateway-mode app data yet. Launch Cowork in gateway mode at least once, then come back.";
    case "insufficient_disk_space":
      return `Not enough free disk space (~${sizeLabel(b.needed_bytes)} needed, ${sizeLabel(b.available_bytes)} free).`;
  }
}

function PathsRow({ discover }: { discover: MigrateDiscover }) {
  if (!discover.roots) return null;
  return (
    <details className="group rounded-md bg-ink-50 px-3 py-2 text-[11px] leading-relaxed text-ink-700 shadow-[inset_0_0_0_1px_oklch(0.96_0_0)]">
      <summary className="flex cursor-pointer items-center justify-between select-none [&::-webkit-details-marker]:hidden">
        <span className="font-medium text-ink-900">Where data moves</span>
        <svg
          viewBox="0 0 24 24"
          className="h-3 w-3 shrink-0 text-ink-500 transition-transform group-open:rotate-90"
          fill="none"
          stroke="currentColor"
          strokeWidth="2"
          strokeLinecap="round"
          strokeLinejoin="round"
          aria-hidden
        >
          <path d="M9 18l6-6-6-6" />
        </svg>
      </summary>
      <div className="mt-1.5 break-all font-mono text-[10.5px] text-ink-700">{discover.roots.source_dir}</div>
      <div className="mt-0.5 break-all font-mono text-[10.5px] text-ink-700">→ {discover.roots.dest_dir}</div>
    </details>
  );
}

// Categories split into two semantic groups so the user isn't faced with
// a 7-item flat list. "Content" = stuff the user made (conversations,
// scheduled tasks, memory, artifacts). "Configuration" = system/plugin
// state. Order within each group goes from "expected to copy" to
// "less common" so the eye lands on the typical wins first.
const CONTENT_KEYS: Array<keyof MigrateOptions> = [
  "include_conversations",
  "include_scheduled",
  "include_memory",
  "include_artifacts",
];
const CONFIG_KEYS: Array<keyof MigrateOptions> = ["include_plugins", "include_enabled_plugins", "include_preferences"];

function CategoryChecklist({
  discover,
  options,
  disabled,
  onToggle,
}: {
  discover: MigrateDiscover;
  options: MigrateOptions;
  disabled: boolean;
  onToggle: (key: keyof MigrateOptions) => void;
}) {
  const contentCats = CONTENT_KEYS.map((k) => CATEGORIES.find((c) => c.key === k)!).filter(Boolean);
  const configCats = CONFIG_KEYS.map((k) => CATEGORIES.find((c) => c.key === k)!).filter(Boolean);

  return (
    <div className="space-y-3">
      <CategoryGroup
        label="Content"
        cats={contentCats}
        discover={discover}
        options={options}
        disabled={disabled}
        onToggle={onToggle}
      />
      <CategoryGroup
        label="Configuration"
        cats={configCats}
        discover={discover}
        options={options}
        disabled={disabled}
        onToggle={onToggle}
      />
    </div>
  );
}

function CategoryGroup({
  label,
  cats,
  discover,
  options,
  disabled,
  onToggle,
}: {
  label: string;
  cats: CategoryDef[];
  discover: MigrateDiscover;
  options: MigrateOptions;
  disabled: boolean;
  onToggle: (key: keyof MigrateOptions) => void;
}) {
  return (
    <section>
      <div className="mb-1.5 px-0.5 font-mono text-[10px] font-medium uppercase tracking-[0.12em] text-ink-500">
        {label}
      </div>
      <div className="rounded-md bg-white shadow-border">
        <ul className="divide-y divide-ink-100">
          {cats.map((cat) => {
            const available = cat.available(discover);
            const checked = options[cat.key] && available;
            return (
              <li key={cat.key as string}>
                <label
                  className={`flex items-start gap-2.5 px-3 py-2.5 ${
                    available ? "cursor-pointer" : "cursor-not-allowed opacity-50"
                  }`}
                >
                  <input
                    type="checkbox"
                    checked={checked}
                    disabled={disabled || !available}
                    onChange={() => onToggle(cat.key)}
                    className="mt-0.5 h-3.5 w-3.5 accent-ink-900"
                  />
                  <div className="min-w-0 flex-1">
                    <div className="text-[12px] font-medium text-ink-900">{cat.label}</div>
                    <div className="font-mono text-[11px] text-ink-500">{cat.describe(discover)}</div>
                  </div>
                </label>
              </li>
            );
          })}
        </ul>
      </div>
    </section>
  );
}

function ReportView({ report }: { report: MigrateReport }) {
  const entries = Object.entries(report.per_category).sort(([a], [b]) => a.localeCompare(b));
  return (
    <div className="rounded-md bg-white shadow-border">
      <ul className="divide-y divide-ink-100">
        {entries.map(([cat, r]) => (
          <li key={cat} className="px-3 py-2.5">
            <div className="flex items-center justify-between gap-2">
              <span className="text-[12px] text-ink-900">{prettyCategory(cat)}</span>
              <span className="font-mono text-[11px] tabular-nums text-ink-500">
                copied {r.copied}
                {r.skipped > 0 && <>, skipped {r.skipped}</>}
                {r.failed > 0 && <span className="text-danger-700"> · failed {r.failed}</span>}
              </span>
            </div>
            {r.errors.length > 0 && (
              <ul className="mt-1 space-y-0.5">
                {r.errors.slice(0, 5).map((e, i) => (
                  <li key={i} className="break-all font-mono text-[10.5px] text-danger-700">
                    ! {e}
                  </li>
                ))}
                {r.errors.length > 5 && (
                  <li className="font-mono text-[10.5px] text-danger-700/60">…and {r.errors.length - 5} more</li>
                )}
              </ul>
            )}
          </li>
        ))}
      </ul>
    </div>
  );
}

function prettyCategory(key: string): string {
  switch (key) {
    case "plugins":
      return "Org plugins & skills";
    case "scheduled":
      return "Scheduled tasks";
    case "conversations":
      return "Conversations";
    case "memory":
      return "Memory & spaces";
    case "enabled_plugins":
      return "Enabled-plugin state";
    case "preferences":
      return "Preferences";
    case "artifacts":
      return "Artifacts";
    default:
      return key;
  }
}

function sizeLabel(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  if (bytes < 1024 * 1024 * 1024) return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
  return `${(bytes / (1024 * 1024 * 1024)).toFixed(2)} GB`;
}
