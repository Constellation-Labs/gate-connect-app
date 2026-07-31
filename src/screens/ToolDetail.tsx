import { useState } from "react";
import type { Tool } from "../lib/api";
import { classifyError, type ClassifiedError } from "../lib/errors";
import { trackError } from "../lib/analytics";
import { SubHeader, SectionLabel, Switch, ErrorNote, IconButton } from "../components/gc/ui";
import { ToolPill } from "../components/ToolPill";
import { Icon } from "../components/gc/Icon";

/** Per-tool detail: the pill is a door, not a verdict. The switch connects or
 * disconnects this one tool (writing/reverting its own config); below it, the
 * full untruncated status (an error's whole message, a drifted setup's
 * reason) so "why isn't this tool connecting?" is answerable without leaving
 * the popover. */
export function ToolDetail({
  tool,
  busy,
  onSetRouted,
  onBack,
}: {
  tool: Tool;
  busy: boolean;
  /** Connect (true) or disconnect (false) this tool; rejects on failure. */
  onSetRouted: (routed: boolean) => Promise<void>;
  onBack: () => void;
}) {
  const { status } = tool;
  const routed = status.kind === "connected";
  const [error, setError] = useState<ClassifiedError | null>(null);
  // A change made here should carry its restart advice here too - the user
  // may close the popover before ever seeing Home's banner.
  const [changed, setChanged] = useState(false);
  // Adopting a drifted (hand-written) Gate setup replaces someone's config;
  // that gets an inline confirm step, armed by the first flip.
  const [confirmingAdopt, setConfirmingAdopt] = useState(false);
  // Copy feedback on the upstream URL (icon flashes to a check), matching
  // the gateway URL in Settings: identifiers are never retyped by hand.
  const [copiedUrl, setCopiedUrl] = useState(false);

  async function copyUpstreamUrl() {
    try {
      await navigator.clipboard.writeText(tool.default_upstream_url);
      setCopiedUrl(true);
      setTimeout(() => setCopiedUrl(false), 1500);
    } catch (e) {
      trackError(e, "generic");
    }
  }

  async function toggle() {
    setError(null);
    if (!routed && status.kind === "drifted" && !confirmingAdopt) {
      setConfirmingAdopt(true);
      return;
    }
    setConfirmingAdopt(false);
    try {
      await onSetRouted(!routed);
      setChanged(true);
    } catch (e) {
      setError(classifyError(e, "connect"));
    }
  }

  return (
    <div className="flex flex-col">
      <SubHeader title={tool.name} onBack={onBack} />

      <div className="flex items-start gap-3 px-3.5 py-3.5">
        <div className="min-w-0 flex-1">
          <div className="text-[14px] font-semibold text-gc-ink">Route through Gate</div>
          <div className="mt-0.5 text-[11.5px] leading-snug text-gc-ink-3">
            Writes {tool.name}&rsquo;s own config so its requests go through
            your gateway.
          </div>
        </div>
        <Switch
          on={routed}
          label={`Route ${tool.name} through Gate`}
          busy={busy}
          onClick={() => void toggle()}
        />
      </div>

      {confirmingAdopt && (
        <div className="mx-3.5 mb-2 rounded bg-gc-subtle p-3 shadow-border">
          <div className="text-[11.5px] leading-snug text-gc-ink-2">
            Replace {tool.name}&rsquo;s existing Gate setup? Gate Connect
            rewrites the config and manages the key from your keychain;
            turning the switch off later restores {tool.name}&rsquo;s own
            settings.
          </div>
          <div className="mt-2.5 flex items-center gap-2">
            <button
              type="button"
              onClick={() => void toggle()}
              disabled={busy}
              className="text-[12.5px] font-medium text-gc-accent disabled:opacity-50"
            >
              Replace setup
            </button>
            <button
              type="button"
              onClick={() => setConfirmingAdopt(false)}
              disabled={busy}
              className="ml-auto text-[12.5px] font-medium text-gc-ink-3"
            >
              Cancel
            </button>
          </div>
        </div>
      )}

      {changed && !error && (
        <div role="status" className="mx-3.5 mb-2 flex items-center gap-2.5 rounded bg-gc-highlight px-3 py-2.5 shadow-border">
          <Icon name="refresh" size={15} className="shrink-0 text-gc-ink" />
          <div className="min-w-0 flex-1 text-[12px] font-medium leading-snug text-gc-ink">
            <span className="font-semibold">Close {tool.name}</span> to apply
            the change; it picks this up the next time you open it.
          </div>
        </div>
      )}

      {error && <ErrorNote error={error} className="mx-3.5 mb-2" />}

      <div className="flex items-center gap-3 px-3.5 pt-1">
        <div className="min-w-0 flex-1 text-[13px] font-medium text-gc-ink">Status</div>
        <ToolPill status={status} />
      </div>

      <div className="px-3.5 pt-2">
        {status.kind === "connected" && (
          <p className="text-[12px] leading-snug text-gc-ink-2">
            {tool.name}&rsquo;s own config points at your Gate gateway. Requests
            carry the key from your keychain; the key itself never lands in the
            config file.
          </p>
        )}
        {status.kind === "detected" && (
          <p className="text-[12px] leading-snug text-gc-ink-2">
            {tool.name} is installed, but its config doesn&rsquo;t point at
            Gate. Turn the switch on and Gate Connect will write the config
            for you.
          </p>
        )}
        {status.kind === "drifted" && (
          <>
            <p className="text-[12px] leading-snug text-gc-ink-2">
              {tool.name} has a Gate setup written outside this app. Turning
              the switch on replaces that configuration and manages the key
              from your keychain.
            </p>
            {status.reason && (
              <div className="mt-2 rounded bg-gc-sunken px-3 py-2.5 font-mono text-[10.5px] leading-snug text-gc-ink-3 [overflow-wrap:anywhere]">
                {status.reason}
              </div>
            )}
          </>
        )}
        {status.kind === "error" && (
          <>
            <p className="text-[12px] leading-snug text-gc-ink-2">
              Gate Connect couldn&rsquo;t read {tool.name}&rsquo;s routing
              state. Try again after restarting Gate Connect; the details
              below help when reporting the problem.
            </p>
            <div
              role="alert"
              className="mt-2 rounded bg-gc-sunken px-3 py-2.5 font-mono text-[10.5px] leading-snug text-gc-ink-2 [overflow-wrap:anywhere]"
            >
              {status.message}
            </div>
          </>
        )}
      </div>

      <SectionLabel>Details</SectionLabel>
      <div className="flex flex-col border-t border-gc-line">
        <div className="flex items-center gap-3 border-b border-gc-line px-3.5 py-2.5">
          <div className="min-w-0 flex-1 text-[12px] text-gc-ink-3">Upstream provider</div>
          <div className="text-[12px] font-medium text-gc-ink">
            {tool.upstream_provider_name}
          </div>
        </div>
        <div className="flex items-center gap-2 border-b border-gc-line px-3.5 py-2.5">
          <div className="min-w-0 flex-1 text-[12px] text-gc-ink-3">Upstream URL</div>
          <div className="truncate font-mono text-[10.5px] text-gc-ink-3">
            {tool.default_upstream_url}
          </div>
          <IconButton
            icon={copiedUrl ? "check" : "copy"}
            size={13}
            onClick={() => void copyUpstreamUrl()}
            aria-label="Copy upstream URL"
          />
        </div>
      </div>
    </div>
  );
}
