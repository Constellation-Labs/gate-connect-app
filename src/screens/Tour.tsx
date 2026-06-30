import { useState } from "react";
import type { ReactNode } from "react";
import { ConstellationHexMark } from "../components/gc/ConstellationHexMark";
import { Button } from "../components/gc/ui";
import { Icon } from "../components/gc/Icon";

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

/** Step 1 - the connected popover, header + content skeleton. */
function WelcomePreview() {
  return (
    <Frame>
      <div className="rounded-md bg-gc-surface p-3 shadow-border">
        <div className="flex items-center gap-2">
          <ConstellationHexMark size={16} fill="#002a5f" />
          <span className="text-[11px] font-semibold tracking-[-0.02em] text-gc-navy">
            Gate <span className="text-gc-accent">Connect</span>
          </span>
          <MiniPill label="connected" />
        </div>
        <div className="mt-3 space-y-1.5">
          <div className="h-2 w-3/4 rounded bg-gc-sunken" />
          <div className="h-2 w-1/2 rounded bg-gc-sunken" />
        </div>
      </div>
    </Frame>
  );
}

/** Step 2 - the connect form: key field + Connect button. */
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

const STEPS: Step[] = [
  {
    preview: <WelcomePreview />,
    title: "Welcome to Gate Connect",
    body: "Point your AI dev tools at one gateway and stop juggling credentials. Here's the quick tour - it takes a few taps.",
  },
  {
    preview: <ConnectPreview />,
    title: "Connect once",
    body: "Sign in with your gateway URL and API key. The key goes straight into the macOS keychain - never a config file on disk.",
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

/** First-launch welcome tour. A full-popover slide sequence shown once, ahead
 * of the connect screen. `onDone` is called for both finish and skip; `skipped`
 * lets the caller record which. */
export function Tour({ onDone }: { onDone: (skipped: boolean) => void }) {
  const [index, setIndex] = useState(0);
  const step = STEPS[index];
  const isLast = index === STEPS.length - 1;

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
        {STEPS.map((_, i) => (
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
