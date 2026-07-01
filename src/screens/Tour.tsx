import { useState } from "react";
import type { ReactNode } from "react";
import { ConstellationHexMark } from "../components/gc/ConstellationHexMark";
import { Button } from "../components/gc/ui";
import { Icon } from "../components/gc/Icon";
import { usePlatform, type Platform } from "../lib/platform";

/** Sunken "screenshot" frame every step preview sits in - a tinted backdrop so
 * the white app surfaces inside read as a little window. */
function Frame({ children }: { children: ReactNode }) {
  return (
    <div className="flex h-[148px] w-full items-center justify-center rounded-md bg-gc-sunken px-5">
      <div className="w-full max-w-[236px]">{children}</div>
    </div>
  );
}

/** A status pill, miniaturized from StatusBadge. */
function MiniPill({ label }: { label: string }) {
  return (
    <span className="ml-auto inline-flex shrink-0 items-center gap-1 rounded-[4px] bg-success-100 px-1.5 py-0.5 font-mono text-[8.5px] font-medium text-success-800">
      <span className="h-1 w-1 rounded-full bg-current" aria-hidden />
      {label}
    </span>
  );
}

/** Step 1 - the app logo, front and center. */
function HeroPreview() {
  return (
    <Frame>
      <div className="flex flex-col items-center justify-center gap-2.5">
        <div className="flex h-[52px] w-[52px] items-center justify-center rounded-[14px] bg-gc-surface shadow-border">
          <ConstellationHexMark size={30} fill="#002a5f" />
        </div>
        <span className="text-[13px] font-semibold tracking-[-0.02em] text-gc-navy">
          Gate <span className="text-gc-accent">Connect</span>
        </span>
      </div>
    </Frame>
  );
}

/** Step 2 - the OS status area, app icon lit up with its popover peeking out.
 * Lives up top (menu bar / top bar) on macOS and Ubuntu, and down in the
 * taskbar tray on Windows. */
function TrayPreview({ platform }: { platform: Platform }) {
  const bottom = platform === "windows";

  const bar = (
    <div className="flex h-[18px] w-full items-center gap-2 rounded-md bg-gc-ink px-2">
      {bottom ? (
        <>
          <span className="h-2.5 w-2.5 rounded-[2px] bg-white/15" />
          <span className="h-2.5 w-2.5 rounded-[2px] bg-white/15" />
        </>
      ) : (
        <>
          <span className="text-[8px] font-semibold text-white/75">File</span>
          <span className="text-[8px] text-white/50">Edit</span>
          <span className="text-[8px] text-white/50">View</span>
        </>
      )}
      <span className="ml-auto flex items-center gap-1.5">
        <span className="font-mono text-[8px] text-white/50">9:41</span>
        <span className="flex h-[14px] w-[14px] items-center justify-center rounded-[3px] bg-white/15">
          <ConstellationHexMark size={9} fill="#ffffff" />
        </span>
      </span>
    </div>
  );

  const popover = (
    <div className="w-[96px] rounded-md bg-gc-surface p-1.5 shadow-border">
      <div className="flex items-center gap-1">
        <ConstellationHexMark size={8} fill="#002a5f" />
        <div className="h-1.5 w-9 rounded bg-gc-sunken" />
      </div>
      <div className="mt-1 h-1.5 w-full rounded bg-gc-sunken" />
    </div>
  );

  return (
    <Frame>
      <div className="flex flex-col items-end">
        {bottom ? (
          <>
            <div className="mb-1.5">{popover}</div>
            {bar}
          </>
        ) : (
          <>
            {bar}
            <div className="mt-1.5">{popover}</div>
          </>
        )}
      </div>
    </Frame>
  );
}

/** Step 3 - the flow: your tools route through Gate to your providers. */
function FlowPreview() {
  return (
    <Frame>
      <div className="flex items-center justify-between">
        <div className="flex flex-col items-center gap-1.5">
          <div className="flex h-9 w-11 flex-col justify-center gap-1 rounded-md bg-gc-surface px-1.5 shadow-border">
            <div className="h-1.5 w-full rounded bg-gc-sunken" />
            <div className="h-1.5 w-2/3 rounded bg-gc-sunken" />
          </div>
          <span className="text-[8px] font-medium text-gc-ink-4">your tools</span>
        </div>
        <Icon name="chevronRight" size={12} className="text-gc-ink-5" />
        <div className="flex flex-col items-center gap-1.5">
          <div className="flex h-9 w-9 items-center justify-center rounded-lg bg-gc-accent-wash">
            <ConstellationHexMark size={16} fill="#002a5f" />
          </div>
          <span className="text-[8px] font-medium text-gc-navy">Gate</span>
        </div>
        <Icon name="chevronRight" size={12} className="text-gc-ink-5" />
        <div className="flex flex-col items-center gap-1.5">
          <div className="flex h-9 w-11 items-center justify-center rounded-md bg-gc-surface text-gc-ink-3 shadow-border">
            <Icon name="layers" size={14} />
          </div>
          <span className="text-[8px] font-medium text-gc-ink-4">providers</span>
        </div>
      </div>
    </Frame>
  );
}

/** Step 4 - the connect form: key field + Connect button. */
function ConnectPreview() {
  return (
    <Frame>
      <div className="font-mono text-[8.5px] font-medium uppercase tracking-[0.08em] text-gc-ink-4">
        Gate API Key
      </div>
      <div className="mt-1.5 flex h-7 items-center gap-1.5 rounded bg-gc-surface px-2 shadow-border">
        <Icon name="key" size={11} className="text-gc-ink-4" />
        <span className="font-mono text-[10px] text-gc-ink-3">sk-gw-••••••••</span>
      </div>
      <div className="mt-2 flex h-7 items-center justify-center rounded bg-gc-accent text-[10px] font-medium text-white">
        Connect
      </div>
    </Frame>
  );
}

/** Mini switch, locked on, mirroring the real Switch. */
function MiniSwitch() {
  return (
    <span className="relative ml-auto inline-flex h-[16px] w-[27px] shrink-0 items-center rounded-full bg-gc-accent">
      <span className="absolute right-[2px] h-[12px] w-[12px] rounded-full bg-white shadow-sm" />
    </span>
  );
}

/** Step 3 - the routing card with its toggle and live request count. */
function RoutePreview() {
  return (
    <Frame>
      <div className="flex items-center gap-2.5 rounded-md bg-gc-surface p-3 shadow-border">
        <div className="flex h-7 w-7 shrink-0 items-center justify-center rounded-lg bg-gc-accent-wash text-gc-accent">
          <Icon name="shieldCheck" size={14} />
        </div>
        <div className="min-w-0 flex-1">
          <div className="text-[10.5px] font-medium text-gc-ink">Route through Gate</div>
          <div className="font-mono text-[8.5px] text-gc-ink-4">1,248 requests</div>
        </div>
        <MiniSwitch />
      </div>
    </Frame>
  );
}

const TOOLS = ["Cowork", "Codex", "OpenCode"];

/** Step 4 - the tool-integration list, each row a connected pill. */
function ToolsPreview() {
  return (
    <Frame>
      <div className="space-y-1.5">
        {TOOLS.map((name) => (
          <div
            key={name}
            className="flex items-center gap-2 rounded bg-gc-surface px-2 py-1.5 shadow-border"
          >
            <div className="flex h-5 w-5 shrink-0 items-center justify-center rounded bg-gc-sunken text-gc-ink-3">
              <Icon name="cube" size={11} />
            </div>
            <span className="text-[10.5px] font-medium text-gc-ink">{name}</span>
            <MiniPill label="connected" />
          </div>
        ))}
      </div>
    </Frame>
  );
}

type Step = {
  preview: ReactNode;
  title: string;
  body: string;
};

/** Where the app lives, worded for the platform's status area. */
function whereItLives(platform: Platform): string {
  switch (platform) {
    case "windows":
      return "Gate Connect sits in your system tray, down by the clock. Click the icon anytime to open this window and check on your setup.";
    case "linux":
      return "Gate Connect sits up in your top bar, one click away. Click the icon anytime to open this window and check on your setup.";
    case "macos":
      return "Gate Connect sits up in your menu bar, one click away. Click the icon anytime to open this window and check on your setup.";
    default:
      return "Gate Connect sits in your system's status area, one click away. Click the icon anytime to open this window and check on your setup.";
  }
}

function buildSteps(platform: Platform): Step[] {
  return [
    {
      preview: <HeroPreview />,
      title: "Welcome to Gate Connect",
      body: "Gate Connect helps you connect your AI agents to Gate AI. This quick tour shows where it lives and how to get set up - just a few taps.",
    },
    {
      preview: <TrayPreview platform={platform} />,
      title: "Where is Gate Connect?",
      body: whereItLives(platform),
    },
    {
      preview: <FlowPreview />,
      title: "One gateway for every tool",
      body: "Point your tools at Gate once and it routes their traffic to your providers. No per-tool keys, no scattered config to keep in sync.",
    },
    {
      preview: <ConnectPreview />,
      title: "Connect once",
      body: "Sign in with your gateway URL and API key. The key goes straight into your keychain - never a config file on disk.",
    },
    {
      preview: <RoutePreview />,
      title: "Route through Gate",
      body: "Flip routing on to send your desktop agents' traffic through the gateway. The live request count climbs as calls flow through.",
    },
    {
      preview: <ToolsPreview />,
      title: "Point your tools",
      body: "Cowork, Codex, OpenCode and more - Gate Connect sets each one to use your gateway. Toggle them on per tool whenever you like.",
    },
  ];
}

/** First-launch welcome tour. A full-popover slide sequence shown once, ahead
 * of the connect screen. `onDone` is called for both finish and skip; `skipped`
 * lets the caller record which. */
export function Tour({ onDone }: { onDone: (skipped: boolean) => void }) {
  const platform = usePlatform();
  const steps = buildSteps(platform);
  const [index, setIndex] = useState(0);
  const step = steps[index];
  const isLast = index === steps.length - 1;

  return (
    <div className="flex flex-1 flex-col px-5 pb-5 pt-7">
      <div className="flex justify-end">
        <button
          type="button"
          onClick={() => onDone(true)}
          className="font-mono text-[11px] lowercase tracking-[0.02em] text-gc-ink-4 transition hover:text-gc-ink-3"
        >
          skip
        </button>
      </div>

      <div className="flex flex-1 flex-col items-center justify-center text-center">
        {step.preview}
        <div className="mt-4 text-[19px] font-semibold tracking-[-0.025em] text-gc-navy">
          {step.title}
        </div>
        <p className="mt-2 max-w-[290px] text-[12.5px] leading-[1.45] text-gc-ink-3">
          {step.body}
        </p>
      </div>

      <div className="mb-4 flex items-center justify-center gap-1.5">
        {steps.map((_, i) => (
          <span
            key={i}
            className={`h-1.5 rounded-full transition-all ${
              i === index ? "w-4 bg-gc-ink-2" : "w-1.5 bg-gc-line-strong"
            }`}
          />
        ))}
      </div>

      <div className="flex gap-2">
        {index > 0 && (
          <Button variant="secondary" onClick={() => setIndex((i) => i - 1)}>
            Back
          </Button>
        )}
        <Button
          variant="accent"
          full
          onClick={() => (isLast ? onDone(false) : setIndex((i) => i + 1))}
        >
          {isLast ? "Get started" : "Next"}
        </Button>
      </div>
    </div>
  );
}
