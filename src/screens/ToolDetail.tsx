import type { Tool } from "../lib/api";
import { SubHeader, SectionLabel, Button } from "../components/gc/ui";
import { ToolPill } from "../components/ToolPill";

/** Per-tool detail: the pill is a door, not a verdict. Shows the full,
 * untruncated status (an error's whole message, a drifted setup's reason)
 * and the next step, so "why isn't this tool connecting?" is answerable
 * without leaving the popover. */
export function ToolDetail({
  tool,
  onBack,
  onOpenRouting,
}: {
  tool: Tool;
  onBack: () => void;
  onOpenRouting: () => void;
}) {
  const { status } = tool;
  return (
    <div className="flex flex-col">
      <SubHeader title={tool.name} onBack={onBack} />

      <div className="flex items-center gap-3 px-3.5 pt-3.5">
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
            Gate. Turn on its provider under Routing and Gate Connect will
            write the config for you.
          </p>
        )}
        {status.kind === "drifted" && (
          <>
            <p className="text-[12px] leading-snug text-gc-ink-2">
              {tool.name} has a Gate setup written outside this app. Turning
              its provider on under Routing replaces that configuration and
              manages the key from your keychain.
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

      {(status.kind === "detected" || status.kind === "drifted") && (
        <div className="px-3.5 pt-3">
          <Button variant="accent" full onClick={onOpenRouting}>
            Open Routing
          </Button>
        </div>
      )}

      <SectionLabel>Details</SectionLabel>
      <div className="flex flex-col border-t border-gc-line">
        <div className="flex items-center gap-3 border-b border-gc-line px-3.5 py-2.5">
          <div className="min-w-0 flex-1 text-[12px] text-gc-ink-3">Provider</div>
          <div className="text-[12px] font-medium text-gc-ink">
            {tool.upstream_provider_name}
          </div>
        </div>
        <div className="flex items-center gap-3 border-b border-gc-line px-3.5 py-2.5">
          <div className="min-w-0 flex-1 text-[12px] text-gc-ink-3">Upstream</div>
          <div className="truncate font-mono text-[10.5px] text-gc-ink-3">
            {tool.default_upstream_url}
          </div>
        </div>
      </div>
    </div>
  );
}
