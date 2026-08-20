import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { emit } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { track } from "../lib/analytics";
import { setTourSeen } from "../lib/tour";
import { ConstellationHexMark } from "../components/gc/ConstellationHexMark";
import { Icon } from "../components/gc/Icon";
import type { IconName } from "../components/gc/Icon";
import { secretStoreName, usePlatform, type Platform } from "../lib/platform";
import appIcon from "../assets/app-icon.png";
import whatIsGateConnect from "../assets/onboarding-what-is-gate-connect.png";
import seeWhatGateIsDoing from "../assets/onboarding-see-what-gate-is-doing.png";
import whereMacos from "../assets/where-is-gate-connect-macos.png";
import whereLinux from "../assets/where-is-gate-connect-linux.png";
import whereWindows from "../assets/where-is-gate-connect-windows.png";

/** Broadcast when the intro is completed, so the popover window can record
 * the seen-flag in its own storage without waiting for a restart. */
export const TOUR_SEEN_EVENT = "gc:tour-seen";

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
  /** The bordered strip the design puts under each tutorial step's art. */
  note?: string;
  noteIcon?: IconName;
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
          className="mx-auto rounded-[28px] drop-shadow-[0_14px_34px_rgba(0,42,95,0.5)]"
        />
      ),
      title: "Welcome to Gate Connect",
      sub: "Created by Constellation Network",
      body: [
        "Gate Connect routes the AI apps you already use through Gate. Every message is checked on the way: prompt-injection attempts are stopped, sensitive values are redacted, and compression trims token spend.",
        "You sign in once, choose the apps to cover, and keep working as usual. Setup is only a few short steps to complete. Click Next to get started.",
      ],
    },
    {
      // Both tutorial illustrations are captures of the Figma's own art
      // (`Flows / Onboarding`, the 590x220 panel inside each step's card),
      // taken at ~1.7x so they hold up on a retina panel. This one is whole;
      // the last step's is cropped mid-chart and carries a fade for it.
      hero: (
        <img
          src={whatIsGateConnect}
          alt="Claude, OpenAI and Gemini connected by lines that meet at the Gate mark, which passes a response on to the app"
          width={1000}
          height={376}
          className="mx-auto block h-auto w-full max-w-[540px]"
        />
      ),
      title: "What is Gate Connect?",
      sub: "Gate Connect is your desktop app to route the AI apps you already use through Gate AI.",
      body: [
        "You sign in once, choose the apps you want covered, and keep working in Claude, Codex, OpenCode, and other supported apps as usual.",
        `For Claude Code and Codex, Gate Connect points the app’s own config at your gateway and restores it when you disconnect. For apps like Claude Desktop or ChatGPT, it routes the provider’s domain through a local proxy. Your Gate key stays in ${secretStoreName(platform)}, not a plain file.`,
      ],
      note: "Your apps keep working normally. Their requests pass through Gate first.",
      noteIcon: "shieldCheck",
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
        "Click the Gate Connect icon to open the compact popover for a quick status check, or expand it to the full desktop app for more details, alerts, and controls.",
      ],
      note: "Open the desktop app for detail. Collapse to a popover for a fast status check.",
      noteIcon: "monitor",
      locate: true,
    },
    {
      hero: (
        <img
          src={seeWhatGateIsDoing}
          alt="The Overview dashboard: messages, blocked and flagged counts, tokens saved, and a bar chart of message volume"
          width={1000}
          height={376}
          className="mx-auto block h-auto w-full max-w-[540px]"
          // The design crops this one mid-chart, so a square bottom edge reads
          // as a rendering bug. Same fade the platform mockups use.
          style={{
            maskImage: "linear-gradient(180deg,#000 92%,transparent 100%)",
            WebkitMaskImage: "linear-gradient(180deg,#000 92%,transparent 100%)",
          }}
        />
      ),
      title: "See what Gate is doing",
      sub: "Once requests pass through Gate, the desktop app shows recent activity, security actions, and compression savings without exposing prompt or response content.",
      body: ["That’s all there is to it. Sign in and your first app is one toggle away."],
      note: "Notifications will alert you when a request has been blocked or flagged.",
      noteIcon: "bell",
    },
  ];
}

/**
 * The window chrome the design draws the intro inside. Window controls belong to
 * the OS, so the bar carries only the brand lockup - the same call the app
 * shell's `Topbar` makes, and the reason this does not reuse it is that that one
 * owns an overflow menu the intro has no use for.
 */
function IntroTopbar() {
  return (
    <header className="flex h-12 shrink-0 items-center justify-center border-b border-base-border bg-base-card">
      <span className="flex items-center gap-2">
        <ConstellationHexMark size={24} />
        <span className="text-base font-semibold leading-6 tracking-[-0.16px]">
          <span className="text-blue-ribbon-800">Gate</span>{" "}
          <span className="text-neutral-600">Connect</span>
        </span>
      </span>
    </header>
  );
}

/** The rail under the topbar. It replaces the footer's step dots and takes over
 *  their job of telling a screen-reader user how far along the tour is. */
function IntroProgress({ step, total }: { step: number; total: number }) {
  return (
    <div
      role="progressbar"
      aria-valuemin={1}
      aria-valuemax={total}
      aria-valuenow={step}
      aria-label={`Step ${step} of ${total}`}
      className="h-1 shrink-0 bg-gray-100"
    >
      <div
        className="h-full bg-gradient-to-r from-blue-ribbon-800 to-blue-ribbon-700 transition-[width] duration-300"
        style={{ width: `${(step / total) * 100}%` }}
      />
    </div>
  );
}

function IntroButton({
  children,
  onClick,
  primary,
  disabled,
}: {
  children: React.ReactNode;
  onClick: () => void;
  primary?: boolean;
  disabled?: boolean;
}) {
  return (
    <button
      type="button"
      onClick={disabled ? undefined : onClick}
      aria-disabled={disabled || undefined}
      className={`flex h-9 min-w-[88px] items-center justify-center rounded-base px-4 text-sm font-medium transition-colors focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-base-primary ${
        primary
          ? "bg-blue-ribbon-700 text-white hover:bg-blue-ribbon-800"
          : "border border-base-border bg-base-card text-neutral-900 shadow-base-2xs hover:bg-gray-50"
      } ${disabled ? "cursor-not-allowed opacity-50" : ""}`}
    >
      {children}
    </button>
  );
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
  // Starts clear: opting out of the intro should be a choice the user makes,
  // not a default they discover after it stops appearing.
  const [dontShow, setDontShow] = useState(false);

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
      // Finishing the tour is itself the "seen" signal, so it records the
      // flag whatever the checkbox says: the checkbox only decides what
      // happens when someone leaves early. (It used to be the sole writer,
      // which is why it had to start checked; now that it doesn't, it can
      // start clear without replaying the intro on every launch.)
      setTourSeen(true);
      track("tour_completed", { source });
      // Tell the popover window to record the flag in its own storage too, in
      // case the platform doesn't share localStorage between webviews.
      void emit(TOUR_SEEN_EVENT);
    } catch (e) {
      console.warn("[gate] tour completion bookkeeping failed", e);
    }
  };

  // Step 0 is the welcome frame, which the design draws without a card. The
  // three that follow are the tutorial proper and number themselves against
  // each other, not against the welcome.
  const tutorialTotal = steps.length - 1;

  return (
    <div className="flex h-full flex-col bg-base-background text-neutral-900">
      <IntroTopbar />
      <IntroProgress step={index + 1} total={steps.length} />

      <div className="flex min-h-0 flex-1 items-center justify-center overflow-y-auto p-6">
        <div
          key={index}
          className={`w-full max-w-[680px] ${
            dir === "fwd" ? "ob-slide-in-fwd" : "ob-slide-in-back"
          }`}
        >
          {index === 0 ? (
            <div className="text-center">
              <div className="mb-4">{step.hero}</div>
              <h1 className="text-balance text-2xl font-semibold leading-8 tracking-heading">
                {step.title}
              </h1>
              <p className="mt-1 text-sm leading-5 text-neutral-600">{step.sub}</p>
              <div className="my-4 h-px w-full bg-base-border" aria-hidden />
              <div className="space-y-3 text-pretty text-left text-sm leading-5 text-neutral-600">
                {step.body.map((p) => (
                  <p key={p.slice(0, 24)}>{p}</p>
                ))}
              </div>
            </div>
          ) : (
            <section className="rounded-lg border border-base-border bg-base-card p-6 shadow-base-sm">
              <div className="flex items-baseline justify-between gap-3">
                <span className="font-mono text-base-xs uppercase leading-4 tracking-eyebrow text-base-muted-foreground">
                  Tutorial
                </span>
                <span className="font-mono text-base-xs uppercase leading-4 tracking-eyebrow text-base-muted-foreground">
                  {index} of {tutorialTotal}
                </span>
              </div>
              <h1 className="mt-2 text-balance text-xl font-semibold leading-6 tracking-heading">
                {step.title}
              </h1>
              <p className="mt-1.5 text-pretty text-sm leading-5 text-neutral-600">{step.sub}</p>
              <div className="mt-3 space-y-3 text-pretty text-sm leading-5 text-neutral-600">
                {step.body.map((p) => (
                  <p key={p.slice(0, 24)}>{p}</p>
                ))}
              </div>

              {/* No panel around this: the captured art *is* the design's
                  590x220 panel, border and gray field included, so wrapping it
                  gives two nested boxes. */}
              <div className="mt-4">{step.hero}</div>

              {step.note && (
                <p className="mt-3 flex items-center gap-2 rounded-lg border border-base-border bg-base-card px-3 py-2 text-left text-base-xs leading-4 text-neutral-600">
                  {step.noteIcon && (
                    <span aria-hidden className="shrink-0 text-base-muted-foreground">
                      <Icon name={step.noteIcon} size={16} />
                    </span>
                  )}
                  {step.note}
                </p>
              )}

              {step.locate && (
                <div className="mt-4 flex justify-center">
                  <IntroButton onClick={() => void invoke("reveal_popover")}>
                    Show me where Gate Connect lives
                  </IntroButton>
                </div>
              )}
            </section>
          )}
        </div>
      </div>

      <footer className="flex h-14 shrink-0 items-center justify-between gap-3 border-t border-base-border bg-base-card px-4">
        <label className="flex w-max cursor-pointer items-center gap-2 text-base-xs leading-4 text-neutral-600">
          <span className="relative inline-flex size-4 shrink-0 items-center justify-center">
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
              className="peer size-4 cursor-pointer appearance-none rounded-[2px] border border-base-input bg-base-card transition-colors checked:border-base-primary checked:bg-base-primary"
            />
            <svg
              aria-hidden
              viewBox="0 0 16 16"
              className="pointer-events-none absolute size-2.5 text-white opacity-0 peer-checked:opacity-100"
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

        <div className="flex items-center gap-2">
          <IntroButton
            disabled={index === 0}
            onClick={() => {
              setDir("back");
              setIndex((i) => Math.max(0, i - 1));
            }}
          >
            Previous
          </IntroButton>
          <IntroButton
            primary
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
          </IntroButton>
        </div>
      </footer>
    </div>
  );
}
