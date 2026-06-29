import type { Tool } from "../lib/api";
import { StatusBadge } from "./StatusBadge";

interface Props {
  tool: Tool;
  busy: boolean;
  onOpen: () => void;
}

/**
 * Tool list row - cg `.cg-card`-shaped surface: white background,
 * shadow-as-border (no solid 1px), 6px radius. The vendor mark on the
 * left is a per-tool colored square hinting at the upstream provider
 * family; the right edge holds the StatusBadge + chevron.
 *
 * `min-w-0 flex-1` on the meta column + `truncate` on the subtitle is
 * the truncation contract - without it, the mono upstream host pushes
 * into the status pill on long values .
 */
export function ToolRow({ tool, busy, onOpen }: Props) {
  const installed = tool.status.kind !== "not_installed";
  const disabled = !installed || busy;
  const subtitle = upstreamSubtitle(tool);

  return (
    <li>
      <button
        type="button"
        onClick={onOpen}
        disabled={disabled}
        className="group flex w-full items-center gap-3 rounded-md bg-white px-3 py-2.5 text-left shadow-border transition-all hover:bg-ink-50 hover:shadow-border-hover focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ink-400 disabled:cursor-not-allowed disabled:opacity-60 disabled:hover:bg-white disabled:hover:shadow-border"
      >
        <VendorMark tool={tool} />

        <div className="min-w-0 flex-1">
          <div className="truncate text-[13px] font-medium leading-tight tracking-[-0.005em] text-ink-900">
            {tool.name}
          </div>
          <div className="mt-0.5 truncate font-mono text-[11px] leading-tight text-ink-500">
            {subtitle}
          </div>
        </div>

        <StatusBadge status={tool.status} />

        <svg
          className="shrink-0 text-ink-400 transition-colors group-hover:text-ink-600"
          viewBox="0 0 24 24"
          width="12"
          height="12"
          fill="none"
          stroke="currentColor"
          strokeWidth="2"
          strokeLinecap="round"
          strokeLinejoin="round"
          aria-hidden
        >
          <path d="M9 6l6 6-6 6" />
        </svg>
      </button>
    </li>
  );
}

/**
 * Per-tool vendor mark - small colored tile with the tool's initials.
 * Color hints at the upstream family without committing to vendor logos:
 *   - Anthropic-family (Cowork, Claude Code) → warm orange
 *   - OpenAI-family (Codex) → ink (neutral, since their brand is black)
 *   - Multi-provider (opencode) → green
 *   - default → ink-700
 */
function VendorMark({ tool }: { tool: Tool }) {
  const initials = vendorInitials(tool.slug);
  const bg = vendorColor(tool.upstream_provider_name);
  return (
    <div
      className={`flex h-7 w-7 shrink-0 items-center justify-center rounded-md font-sans text-[12px] font-semibold tracking-[-0.02em] text-white ${bg}`}
      aria-hidden
    >
      {initials}
    </div>
  );
}

function vendorInitials(slug: string): string {
  switch (slug) {
    case "cowork":
      return "C";
    case "claude-code":
      return "CC";
    case "codex":
      return "cx";
    case "opencode":
      return "oc";
    case "openclaw":
      return "ow";
    case "hermes":
      return "hm";
    default:
      return slug.slice(0, 2).toUpperCase();
  }
}

function vendorColor(provider: string): string {
  // Soft warm for Anthropic shapes, neutral ink for OpenAI, green for
  // opencode (which is multi-provider). Class names are inlined so
  // Tailwind picks them up during scan.
  switch (provider.toLowerCase()) {
    case "anthropic":
      return "bg-[oklch(0.62_0.18_28)]";
    case "openai":
      return "bg-[oklch(0.32_0.02_264)]";
    case "opencode":
      return "bg-[oklch(0.45_0.16_145)]";
    default:
      return "bg-ink-700";
  }
}

function upstreamSubtitle(tool: Tool): string {
  // Show the upstream host (no scheme, no path) for a compact
  // technically-informative subtitle that matches the gateway-host
  // styling in the header.
  try {
    const url = new URL(tool.default_upstream_url);
    return url.host + (url.pathname && url.pathname !== "/" ? url.pathname : "");
  } catch {
    return tool.upstream_provider_name;
  }
}
