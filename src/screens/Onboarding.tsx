import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { emit } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { proxyStatus, proxyTrustCa } from "../lib/api";
import { track, trackError } from "../lib/analytics";
import { classifyError } from "../lib/errors";
import { setTourSeen } from "../lib/tour";
import { Button } from "../components/gc/ui";
import { Icon } from "../components/gc/Icon";
import {
  secretStoreName,
  trustPromptHint,
  trustPromptWaiting,
  trustStoreName,
  usePlatform,
  type Platform,
} from "../lib/platform";
import appIcon from "../assets/app-icon.png";
import routingScreen from "../assets/app-integrations.png";
import whereMacos from "../assets/where-is-gate-connect-macos.png";
import whereLinux from "../assets/where-is-gate-connect-linux.png";
import whereWindows from "../assets/where-is-gate-connect-windows.png";

/** Broadcast when the intro is completed, so the popover window can record
 * the seen-flag in its own storage without waiting for a restart. */
export const TOUR_SEEN_EVENT = "gc:tour-seen";

/** Broadcast when the intro trusts the CA, so the popover re-reads proxy state
 * instead of rendering an untrusted certificate until something else refreshes
 * it. Not the backend's `proxy-state-changed`: that one means "routing came up
 * behind your back" and the popover announces it (banner + analytics), which
 * would be a lie about a trust that started nothing. */
export const CA_TRUSTED_EVENT = "gc:ca-trusted";

/** Step 2 hero - the popover's Routing screen shown as a framed screenshot,
 * sized to sit above the title. */
function RoutingHero() {
  return (
    <figure>
      <img
        src={routingScreen}
        alt="The Gate Connect popover: a Routing switch above one row per model family"
        // Intrinsic size of the capture, so the box is reserved before the PNG
        // decodes; `w-full` still drives the rendered width.
        width={402}
        height={442}
        className="mx-auto block h-auto w-full max-w-[256px]"
        // The capture is cropped mid-card, so a square bottom edge reads as a
        // rendering bug. Same fade the platform mockups below already use.
        style={{
          maskImage: "linear-gradient(180deg,#000 92%,transparent 100%)",
          WebkitMaskImage: "linear-gradient(180deg,#000 92%,transparent 100%)",
        }}
      />
      <figcaption className="mt-1.5 text-gc-label text-gc-ink-3">
        The Gate Connect popover
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

/** The shape of the system dialog each platform raises for the CA trust, as
 * the buttons the user has to pick between. Windows is the one that prompted
 * this screen: `certutil -user -addstore Root` raises a red "Security Warning"
 * quoting the certificate's name, which reads as something having gone wrong
 * unless the user was told to expect it. */
const TRUST_DIALOG: Record<
  Platform,
  { title: string; confirm: string; dismiss: string; password: boolean }
> = {
  windows: { title: "Security Warning", confirm: "Yes", dismiss: "No", password: false },
  macos: { title: "Gate Connect", confirm: "OK", dismiss: "Cancel", password: true },
  linux: {
    title: "Authentication required",
    confirm: "Authenticate",
    dismiss: "Cancel",
    password: true,
  },
  unknown: { title: "Confirm", confirm: "OK", dismiss: "Cancel", password: true },
};

/** Step 3 hero: the system dialog, drawn rather than screenshotted.
 *
 * A capture would be wrong on half the machines that see it (the warning is
 * worded and framed differently across Windows versions), and the point is not
 * pixel fidelity - it is that a window of roughly this shape is about to
 * appear, and which button ends it. Drawn in the app's own tokens and
 * `aria-hidden`, with the instruction carried in the step's real copy.
 *
 * `trusted` replaces it with the settled state: previewing a dialog nobody is
 * going to see would be the same lie the copy above it stopped telling. */
function TrustHero({ platform, trusted }: { platform: Platform; trusted: boolean | null }) {
  const dialog = TRUST_DIALOG[platform];
  if (trusted) {
    return (
      <div
        aria-hidden
        className="mx-auto flex h-[76px] w-[76px] items-center justify-center rounded-[20px] bg-gc-accent-wash text-gc-accent"
      >
        <Icon name="shieldCheck" size={36} />
      </div>
    );
  }
  return (
    <figure>
      <div
        aria-hidden
        className="mx-auto w-full max-w-[300px] rounded-[10px] bg-gc-surface p-3.5 text-left shadow-border"
      >
        <div className="flex items-center gap-2">
          <span className="flex h-6 w-6 shrink-0 items-center justify-center rounded-[7px] bg-gc-warning-wash text-gc-warning-deep">
            <Icon name="info" size={13} />
          </span>
          <span className="text-gc-body-sm font-medium text-gc-ink">{dialog.title}</span>
        </div>
        {/* Skeleton lines, not lorem text: inventing sentences the OS does not
            say would teach the user to look for words that never appear. */}
        <div className="mt-2.5 flex-col gap-1.5">
          <span className="block h-[6px] w-full rounded-full bg-gc-line" />
          <span className="block h-[6px] w-4/5 rounded-full bg-gc-line" />
        </div>
        {dialog.password && <div className="mt-2.5 h-[26px] rounded bg-gc-sunken" />}
        <div className="mt-3 flex items-center justify-end gap-2">
          <span className="rounded bg-gc-sunken px-2.5 py-1 text-gc-label text-gc-ink-3">
            {dialog.dismiss}
          </span>
          <span className="rounded bg-gc-accent px-2.5 py-1 text-gc-label font-medium text-white">
            {dialog.confirm}
          </span>
        </div>
      </div>
      <figcaption className="mt-1.5 text-gc-label text-gc-ink-3">
        Roughly what your system will show
      </figcaption>
    </figure>
  );
}

/** Whether the proxy CA is already trusted on this machine. `null` while the
 * read is in flight, and on a platform with no proxy subsystem - either way
 * there is nothing to say yet, so the certificate step renders no controls.
 *
 * Lifted out of the step's own component because the step's *copy* turns on it
 * too: promising a system prompt to someone whose certificate is already
 * installed describes a dialog that will never appear. */
function useCaTrusted(): [boolean | null, (trusted: boolean) => void] {
  const [trusted, setTrusted] = useState<boolean | null>(null);
  useEffect(() => {
    let alive = true;
    proxyStatus()
      .then((s) => {
        if (alive) setTrusted(s.ca_trusted);
      })
      .catch(() => {});
    return () => {
      alive = false;
    };
  }, []);
  return [trusted, setTrusted];
}

/** The certificate step's controls: install now, or move on and meet the
 * prompt later from the popover. */
function TrustStep({
  platform,
  trusted,
  onTrusted,
}: {
  platform: Platform;
  trusted: boolean | null;
  onTrusted: (trusted: boolean) => void;
}) {
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function install() {
    setError(null);
    setPending(true);
    try {
      const state = await proxyTrustCa();
      onTrusted(state.ca_trusted);
      track("ca_trusted", { source: "tour" });
      // The popover is a separate webview holding its own copy of proxy state,
      // so without this it renders an untrusted certificate until the next
      // thing that happens to re-read it.
      void emit(CA_TRUSTED_EVENT);
    } catch (e) {
      trackError(e, "trust_ca");
      // The classified hint names the button to press again, which here is the
      // one directly below it.
      setError(classifyError(e, "trust_ca").title);
    } finally {
      setPending(false);
    }
  }

  if (trusted === null) return null;

  if (trusted) {
    return (
      <p className="mt-4 flex items-center justify-center gap-1.5 text-gc-title-sm text-gc-ink-2">
        <Icon name="check" size={15} className="text-gc-accent" />
        Installed. Nothing to do here.
      </p>
    );
  }

  return (
    <div className="mt-4 flex-col items-center gap-2">
      <Button variant="accent" size="sm" disabled={pending} onClick={() => void install()}>
        {pending ? "Waiting…" : "Install certificate"}
      </Button>
      {/* Present tense while the dialog is up, because at that point it may be
          covering the window this sentence is written in. `aria-live` because
          a screen-reader user is not told the dialog opened at all. */}
      <p aria-live="polite" className="text-gc-caption text-gc-ink-3">
        {pending
          ? trustPromptWaiting(platform)
          : "Skipping is fine: the popover offers it again the first time you route one of those apps."}
      </p>
      {error && <p className="text-gc-caption text-gc-error">{error}</p>}
    </div>
  );
}

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
  /** Renders the install-the-certificate controls under the copy. */
  trust?: boolean;
};

function buildSteps(platform: Platform, caTrusted: boolean | null): Step[] {
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
      sub: "Created by Constellation Gate AI",
      body: [
        "Gate Connect routes the AI apps you already use through Gate. Every request is checked on the way: prompt-injection attempts are stopped, sensitive values are redacted, and compression trims token spend.",
        "You sign in once, choose the apps to cover, and keep working as usual. Setup only takes a few short steps. Click Next to get started.",
      ],
    },
    {
      hero: <RoutingHero />,
      title: "How to turn it on",
      sub: "Routing is a per-app choice: turn on the apps you want Gate to cover.",
      body: [
        "For Claude Code and Codex, Gate Connect points the app’s own config at your gateway and restores it when you disconnect. For apps like Claude Desktop or ChatGPT, it routes the provider’s domain through a local proxy.",
        `Connected apps route through Gate; unselected apps stay unchanged. Your Gate key stays in ${secretStoreName(platform)}, not a plain file.`,
      ],
    },
    // Third, not last: it belongs with "how to turn it on" (it is the rest of
    // that answer), and the tour still has to end on the step that tells the
    // user where the app lives. Putting the OS dialog on its own full-width
    // screen is the whole point - the popover is 360px and has room for one
    // sentence about it, which is how the warning kept losing to the dialog.
    {
      hero: <TrustHero platform={platform} trusted={caTrusted} />,
      // Already-trusted machines (a replay from Settings, a reinstall the
      // certificate survived) must not be promised a dialog that will not
      // arrive: the copy reports rather than warns.
      title: caTrusted ? "The certificate is in place" : "One prompt to expect",
      sub: caTrusted
        ? "Already installed on this machine, so nothing will interrupt you."
        : trustPromptHint(platform),
      body: [
        `Apps like Claude Desktop have no gateway setting to point anywhere, so Gate Connect routes them through a proxy running on this machine. That proxy needs a certificate your ${trustStoreName(platform)} trusts${caTrusted ? "" : ", and installing it is the one step where your system asks you to confirm"}.`,
        "The certificate is generated on this machine and never leaves it. Nothing but the local proxy uses it, and you can remove it from Settings whenever routing is off.",
      ],
      trust: true,
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
        "That’s all there is to it. Sign in and your first app is one toggle away.",
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
  const [caTrusted, setCaTrusted] = useCaTrusted();
  const steps = buildSteps(platform, caTrusted);
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
          <h1 className="text-balance text-gc-display font-semibold leading-tight">{step.title}</h1>
          <p className="mt-[7px] text-gc-body text-gc-ink-3">{step.sub}</p>
          <div className="mb-[14px] mt-[17px] h-px w-full bg-gc-line" aria-hidden />
          <div className="mx-auto max-w-[620px] space-y-3 text-left text-gc-title-sm leading-[1.62] text-pretty text-gc-ink-2">
            {step.body.map((p) => (
              <p key={p.slice(0, 24)}>{p}</p>
            ))}
          </div>
          {step.trust && (
            <TrustStep platform={platform} trusted={caTrusted} onTrusted={setCaTrusted} />
          )}
          {/* The button kit, not a fourth skin. This was accent-wash fill
              with accent text and a seam - a shape DESIGN.md's vocabulary
              does not contain, and the Provisional Indigo rule says not to
              invent new indigo surfaces. */}
          {step.locate && (
            <Button
              variant="secondary"
              size="sm"
              className="mt-4"
              onClick={() => void invoke("reveal_popover")}
            >
              Show me where Gate Connect lives
            </Button>
          )}
        </div>
      </div>

      <footer className="grid h-[52px] shrink-0 grid-cols-[1fr_auto_1fr] items-center gap-3 border-t border-gc-line bg-gc-subtle px-[18px]">
        <label className="flex w-max cursor-pointer items-center gap-[7px] text-gc-caption text-gc-ink-3">
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
        {/* Decorative dots, but the position they encode is not: without the
            label the tour never tells a screen-reader user how far along they
            are. */}
        <div className="flex items-center gap-[7px]" role="img" aria-label={`Step ${index + 1} of ${steps.length}`}>
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
