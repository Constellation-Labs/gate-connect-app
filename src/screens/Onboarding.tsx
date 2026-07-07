import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { emit } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { Icon } from "../components/gc/Icon";
import { SectionLabel } from "../components/gc/ui";
import { track } from "../lib/analytics";
import { setTourSeen } from "../lib/tour";
import { usePlatform, type Platform } from "../lib/platform";
import appIcon from "../assets/app-icon.png";
import whereMacos from "../assets/where-is-gate-connect-macos.png";
import whereLinux from "../assets/where-is-gate-connect-linux.png";
import whereWindows from "../assets/where-is-gate-connect-windows.png";

/** Broadcast when the intro is completed, so the popover window can record
 * the seen-flag in its own storage without waiting for a restart. */
export const TOUR_SEEN_EVENT = "gc:tour-seen";

/** Non-interactive switch for the routing-screen facsimile. */
function MockSwitch({ on }: { on: boolean }) {
  return (
    <span
      className={`relative ml-auto inline-flex h-[22px] w-[38px] shrink-0 items-center rounded-full ${
        on ? "bg-gc-accent" : "bg-gc-line-strong"
      }`}
      aria-hidden
    >
      <span
        className={`absolute flex h-[17px] w-[17px] items-center justify-center rounded-full bg-white shadow-sm ${
          on ? "right-[3px] text-gc-accent" : "left-[3px] text-transparent"
        }`}
      >
        {on && <Icon name="check" size={10} />}
      </span>
    </span>
  );
}

function MockProviderRow({ name, sub, on }: { name: string; sub: string; on: boolean }) {
  return (
    <div className="flex items-center gap-3 px-4 py-3">
      <div className="min-w-0 flex-1">
        <div className="text-[13.5px] font-semibold text-gc-ink">{name}</div>
        <div className="mt-0.5 text-[12px] text-gc-ink-4">{sub}</div>
      </div>
      <MockSwitch on={on} />
    </div>
  );
}

/** Step 2 hero - a facsimile of the popover's Routing screen, blown up so the
 * per-app toggles read from across the room. */
function RoutingHero() {
  return (
    <figure>
      <div className="mx-auto w-full max-w-[440px] rounded-xl bg-gc-surface text-left shadow-popover">
        <div className="flex items-center gap-1.5 border-b border-gc-line px-3.5 py-3">
          <Icon name="chevronLeft" size={15} className="text-gc-ink-4" />
          <span className="text-[14px] font-semibold text-gc-ink">Routing</span>
        </div>
        <div className="flex items-center gap-3 px-4 py-3.5">
          <div className="flex h-9 w-9 shrink-0 items-center justify-center rounded-lg bg-gc-accent-wash text-gc-accent">
            <Icon name="shieldCheck" size={18} />
          </div>
          <div className="min-w-0 flex-1">
            <div className="text-[13.5px] font-semibold text-gc-ink">Route through Gate</div>
            <div className="mt-0.5 text-[12px] text-gc-ink-4">
              On · routing your enabled apps through Gate
            </div>
          </div>
          <MockSwitch on />
        </div>
        <SectionLabel>Providers</SectionLabel>
        <div className="divide-y divide-gc-line border-t border-gc-line">
          <MockProviderRow name="Claude Code / Cowork" sub="Claude Code + Claude Desktop" on />
          <MockProviderRow name="OpenAI / Codex" sub="Codex + OpenAI API" on />
          <MockProviderRow name="OpenRouter" sub="OpenRouter API" on />
          <MockProviderRow name="Google / Gemini" sub="Gemini API" on={false} />
        </div>
      </div>
      <figcaption className="mt-3 text-[13px] text-gc-ink-4">
        The Routing screen in Gate Connect
      </figcaption>
    </figure>
  );
}

const WHERE_IMAGE: Record<Platform, string> = {
  macos: whereMacos,
  linux: whereLinux,
  windows: whereWindows,
  unknown: whereMacos,
};

/** Where the tray icon lives, in this OS's own vocabulary. */
function whereItLives(platform: Platform): string {
  switch (platform) {
    case "windows":
      return "Gate Connect lives in the system tray at the bottom right of your screen.";
    case "linux":
      return "Gate Connect lives in the top bar at the top right of your screen.";
    default:
      return "Gate Connect lives in the menu bar at the top right of your screen.";
  }
}

type Step = {
  hero: React.ReactNode;
  title: string;
  sub: string;
  body: string[];
  locate?: boolean;
};

function buildSteps(platform: Platform): Step[] {
  return [
    {
      hero: (
        <img
          src={appIcon}
          alt="The Gate Connect app icon"
          width={96}
          height={96}
          className="mx-auto drop-shadow-[0_16px_28px_rgba(15,18,34,0.22)]"
        />
      ),
      title: "Welcome to Gate Connect",
      sub: "Created by Constellation Gate AI",
      body: [
        "Gate Connect routes the AI apps you already use through Gate. Every request is checked on the way: prompt-injection attempts are stopped, sensitive values are redacted, and compression trims token spend.",
        "You sign in once, choose the apps to cover, and keep working as usual. Setup only takes a few short steps. Click Next to get started.",
      ],
    },
    {
      hero: <RoutingHero />,
      title: "How to turn it on",
      sub: "Protection is a per-app choice: turn on the apps you want Gate to cover.",
      body: [
        "For Claude Code, Codex, and opencode, Gate Connect points the app's own config at your gateway and restores it when you disconnect. For apps like Claude Desktop or ChatGPT, it routes the provider's domain through a local proxy.",
        "Connected apps route through Gate; unselected apps stay unchanged. Your Gate key stays in the operating system keychain, not a plain file.",
      ],
    },
    {
      hero: (
        <img
          src={WHERE_IMAGE[platform]}
          alt="The Gate Connect icon sits in the system status area; clicking it opens the status popover"
          className="mx-auto w-full max-w-[720px]"
        />
      ),
      title: "Where is Gate Connect?",
      sub: whereItLives(platform),
      body: [
        "Click the Gate icon to see whether Gate is active, which apps are connected, and which need attention.",
        "That's all there is to it. Sign in and your first app is one toggle away.",
      ],
      locate: true,
    },
  ];
}

/** Window-sized first-launch intro. Lives in its own Tauri window (label
 * "onboarding"); the popover never renders this. Completing it (or checking
 * "do not show again") records the seen-flag, and closing the window hands
 * the user back to the tray popover (see the Rust CloseRequested handler). */
export function Onboarding() {
  const platform = usePlatform();
  const steps = buildSteps(platform);
  const [index, setIndex] = useState(0);
  const [dir, setDir] = useState<"fwd" | "back">("fwd");
  const [dontShow, setDontShow] = useState(true);

  // First launch vs a replay from Settings, threaded through the window URL
  // by `open_onboarding_window`.
  const source =
    new URLSearchParams(window.location.search).get("source") === "settings"
      ? "settings"
      : "firstrun";

  const step = steps[index];
  const last = index === steps.length - 1;

  const finish = () => {
    setTourSeen(dontShow);
    track("tour_completed", { source });
    // Tell the popover window to record the flag in its own storage too, in
    // case the platform doesn't share localStorage between webviews.
    if (dontShow) void emit(TOUR_SEEN_EVENT);
    void getCurrentWindow().close();
  };

  return (
    <div className="flex h-full flex-col bg-gc-surface text-gc-ink">
      <div className="flex-1 overflow-y-auto px-10 pb-8 pt-12">
        <div
          key={index}
          className={`mx-auto w-full max-w-[860px] text-center ${
            dir === "fwd" ? "ob-slide-in-fwd" : "ob-slide-in-back"
          }`}
        >
          <div className="mb-8">{step.hero}</div>
          <h1 className="text-[30px] font-bold leading-tight tracking-[-0.02em]">{step.title}</h1>
          <p className="mt-2 text-[15px] text-gc-ink-4">{step.sub}</p>
          <div className="my-7 h-px w-full bg-gc-line" aria-hidden />
          <div className="mx-auto max-w-[640px] space-y-4 text-left text-[15.5px] leading-[1.65] text-gc-ink-2">
            {step.body.map((p) => (
              <p key={p.slice(0, 24)}>{p}</p>
            ))}
          </div>
          {step.locate && (
            <button
              type="button"
              onClick={() => void invoke("reveal_popover")}
              className="mt-8 inline-flex h-10 items-center rounded-lg bg-gc-accent-wash px-5 text-[14px] font-semibold text-gc-accent shadow-border transition-colors hover:bg-gc-accent-wash-2"
            >
              Click Here to Locate Gate Connect
            </button>
          )}
        </div>
      </div>

      <footer className="grid h-[56px] shrink-0 grid-cols-[1fr_auto_1fr] items-center gap-3 border-t border-gc-line bg-gc-subtle px-4">
        <label className="flex w-max cursor-pointer items-center gap-2 text-[13px] text-gc-ink-3">
          <input
            type="checkbox"
            checked={dontShow}
            onChange={(e) => {
              setDontShow(e.target.checked);
              // Persist immediately so closing the window mid-flow honors it.
              setTourSeen(e.target.checked);
            }}
            className="h-4 w-4 accent-gc-accent"
          />
          Do not show this intro again
        </label>
        <div className="flex items-center gap-1.5" aria-hidden>
          {steps.map((_, i) => (
            <span
              key={i}
              className={`h-1.5 w-1.5 rounded-full ${i === index ? "bg-gc-accent" : "bg-gc-line-strong"}`}
            />
          ))}
        </div>
        <div className="flex items-center justify-end gap-2">
          <button
            type="button"
            disabled={index === 0}
            onClick={() => {
              setDir("back");
              setIndex((i) => Math.max(0, i - 1));
            }}
            className="h-9 rounded-lg bg-gc-sunken px-4 text-[13.5px] font-semibold text-gc-ink-2 transition-colors enabled:hover:bg-gc-line disabled:opacity-55"
          >
            Previous
          </button>
          <button
            type="button"
            onClick={() => {
              if (last) {
                finish();
              } else {
                setDir("fwd");
                setIndex((i) => i + 1);
              }
            }}
            className="h-9 rounded-lg bg-gc-ink px-4 text-[13.5px] font-semibold text-white transition-colors hover:bg-gc-ink-2"
          >
            {last ? "Get started" : "Next"}
          </button>
        </div>
      </footer>
    </div>
  );
}
