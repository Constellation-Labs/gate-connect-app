import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { emit } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { track } from "../lib/analytics";
import { setTourSeen } from "../lib/tour";
import { Button } from "../components/gc/ui";
import { usePlatform, type Platform } from "../lib/platform";
import appIcon from "../assets/app-icon.png";
import routingScreen from "../assets/app-integrations.png";
import whereMacos from "../assets/where-is-gate-connect-macos.png";
import whereLinux from "../assets/where-is-gate-connect-linux.png";
import whereWindows from "../assets/where-is-gate-connect-windows.png";

/** Broadcast when the intro is completed, so the popover window can record
 * the seen-flag in its own storage without waiting for a restart. */
export const TOUR_SEEN_EVENT = "gc:tour-seen";

/** Step 2 hero - the popover's Routing screen shown as a framed screenshot,
 * sized to sit above the title. */
function RoutingHero() {
  return (
    <figure>
      <img
        src={routingScreen}
        alt="Gate Connect routing screen: a Route through Gate toggle above per-app toggles"
        className="mx-auto block w-full max-w-[256px]"
      />
      <figcaption className="mt-1.5 text-[10.5px] text-gc-ink-4">
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
          width={128}
          height={128}
          className="mx-auto rounded-[28px] drop-shadow-[0_14px_34px_rgba(20,38,102,0.5)]"
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
          className={`mx-auto block w-full max-w-[540px] object-cover ${
            platform === "windows" ? "object-bottom" : "object-top"
          }`}
          // Crop the 1920x1120 mockup toward the tray icon and fade the cut edge.
          // Windows' tray is at the bottom right, so keep the bottom and cut the
          // top; macOS/Linux menu bars are at the top, so keep the top.
          style={{
            aspectRatio: "1920 / 840",
            maskImage:
              platform === "windows"
                ? "linear-gradient(0deg,#000 88%,transparent 100%)"
                : "linear-gradient(180deg,#000 88%,transparent 100%)",
            WebkitMaskImage:
              platform === "windows"
                ? "linear-gradient(0deg,#000 88%,transparent 100%)"
                : "linear-gradient(180deg,#000 88%,transparent 100%)",
          }}
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
  // by `open_onboarding_window` as a hash fragment (query strings can fail to
  // load the page on Windows).
  const source = window.location.hash === "#settings" ? "settings" : "firstrun";

  const step = steps[index];
  const last = index === steps.length - 1;

  // Closing the window before "Get started" is a skip. `finish` also closes
  // the window, so it sets this first to keep a completed run from
  // double-counting as a skip. The index rides in a ref so the once-mounted
  // close listener reads the step the user actually left from.
  const finishedRef = useRef(false);
  const indexRef = useRef(index);
  indexRef.current = index;
  useEffect(() => {
    // Registering this listener takes over the window's close semantics:
    // Tauri prevents the native close whenever JS listens for close-requested
    // (has_js_listener in manager/window.rs) and the window then closes only
    // because @tauri-apps/api's wrapper calls destroy() once this resolves
    // without preventDefault. Two consequences worth keeping in mind before
    // editing: the capability set must keep `core:window:allow-destroy` (an
    // unpermitted destroy leaves a window nobody can close), and a throw in
    // here strands the user the same way - so nothing may fail loudly.
    const unlisten = getCurrentWindow().onCloseRequested(() => {
      try {
        if (finishedRef.current) return;
        finishedRef.current = true;
        track("tour_skipped", { source, step: indexRef.current + 1 });
      } catch (e) {
        console.warn("[gate] tour skip tracking failed", e);
      }
    });
    return () => {
      void unlisten.then((f) => f());
    };
  }, [source]);

  const finish = () => {
    finishedRef.current = true;
    // Close first, then do the bookkeeping: none of it is worth trapping the
    // user in a window that won't close, and the guard keeps a failed write
    // from escaping the click handler.
    void getCurrentWindow().close();
    try {
      setTourSeen(dontShow);
      track("tour_completed", { source });
      // Tell the popover window to record the flag in its own storage too, in
      // case the platform doesn't share localStorage between webviews.
      if (dontShow) void emit(TOUR_SEEN_EVENT);
    } catch (e) {
      console.warn("[gate] tour completion bookkeeping failed", e);
    }
  };

  return (
    <div className="flex h-full flex-col bg-gc-surface text-gc-ink">
      <div className="flex flex-1 items-center justify-center overflow-y-auto px-[42px] py-8">
        <div
          key={index}
          className={`mx-auto w-full max-w-[860px] text-center ${
            dir === "fwd" ? "ob-slide-in-fwd" : "ob-slide-in-back"
          }`}
        >
          <div className="mb-4">{step.hero}</div>
          <h1 className="text-balance text-[27px] font-semibold leading-tight">{step.title}</h1>
          <p className="mt-[7px] text-[13.5px] text-gc-ink-3">{step.sub}</p>
          <div className="mb-[14px] mt-[17px] h-px w-full bg-gc-line" aria-hidden />
          <div className="mx-auto max-w-[620px] space-y-3 text-left text-[14px] leading-[1.62] text-pretty text-gc-ink-2">
            {step.body.map((p) => (
              <p key={p.slice(0, 24)}>{p}</p>
            ))}
          </div>
          {step.locate && (
            <button
              type="button"
              onClick={() => void invoke("reveal_popover")}
              className="mt-4 inline-flex h-[34px] items-center rounded-lg bg-gc-accent-wash px-[18px] text-[12.5px] font-semibold text-gc-accent shadow-border transition-colors hover:bg-gc-accent-wash-2"
            >
              Show me where Gate Connect lives
            </button>
          )}
        </div>
      </div>

      <footer className="grid h-[52px] shrink-0 grid-cols-[1fr_auto_1fr] items-center gap-3 border-t border-gc-line bg-gc-subtle px-[18px]">
        <label className="flex w-max cursor-pointer items-center gap-[7px] text-[11.5px] text-gc-ink-3">
          <span className="relative inline-flex h-[14px] w-[14px] shrink-0 items-center justify-center">
            <input
              type="checkbox"
              checked={dontShow}
              onChange={(e) => {
                setDontShow(e.target.checked);
                // Persist immediately so closing the window mid-flow honors it.
                setTourSeen(e.target.checked);
              }}
              // WKWebView renders native checkboxes white-on-white and
              // effectively invisible, so we draw the box + check ourselves.
              className="peer h-[14px] w-[14px] cursor-pointer appearance-none rounded-[4px] border border-gc-line-strong bg-white transition-colors checked:border-gc-accent checked:bg-gc-accent"
            />
            <svg
              aria-hidden
              viewBox="0 0 16 16"
              className="pointer-events-none absolute h-[10px] w-[10px] text-white opacity-0 peer-checked:opacity-100"
            >
              <path
                d="M4 8.5l2.5 2.5L12 5"
                fill="none"
                stroke="currentColor"
                strokeWidth={2}
                strokeLinecap="round"
                strokeLinejoin="round"
              />
            </svg>
          </span>
          Do not show this intro again
        </label>
        <div className="flex items-center gap-[7px]" aria-hidden>
          {steps.map((_, i) => (
            <span
              key={i}
              className={`h-[7px] w-[7px] rounded-full ${i === index ? "bg-gc-accent" : "bg-gc-line-strong"}`}
            />
          ))}
        </div>
        <div className="flex items-center justify-end gap-2">
          <Button
            size="sm"
            className="min-w-[74px]"
            disabled={index === 0}
            onClick={() => {
              setDir("back");
              setIndex((i) => Math.max(0, i - 1));
            }}
          >
            Previous
          </Button>
          <Button
            variant="accent"
            size="sm"
            className="min-w-[74px]"
            onClick={() => {
              if (last) {
                finish();
              } else {
                setDir("fwd");
                setIndex((i) => i + 1);
              }
            }}
          >
            {last ? "Get started" : "Next"}
          </Button>
        </div>
      </footer>
    </div>
  );
}
