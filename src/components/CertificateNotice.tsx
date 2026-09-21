import { useRef } from "react";
import { Takeover, TAKEOVER_Z } from "./Takeover";
import { Button } from "./gc/ui";
import { Icon } from "./gc/Icon";
import { trustPromptHint, trustPromptWaiting, trustStoreName, type Platform } from "../lib/platform";
import windowsTrustDialog from "../assets/windows-certificate-warning.png";

/** How each platform's trust prompt is depicted, and what it is.
 *
 * Two shapes, because the platforms split in two. Windows raises a "Security
 * Warning" asking the user to weigh something, and we have a capture of the
 * real one - which is worth more than a drawing, because it quotes
 * `CA_COMMON_NAME` ("Gate Connect Local CA") back at the user four times, and
 * that is the detail that lets them match the window in front of them to this
 * app. The rest raise a password entry: macOS through `osascript … with
 * administrator privileges` (primitives.rs:169), Linux through `pkexec`
 * (ca_linux.rs:225). Neither asks the user to weigh anything, so they are
 * drawn as a field rather than as a warning, in that platform's own words.
 *
 * Keyed on the whole of `Platform` so a new OS has to answer this question, and
 * so the day one of the drawn platforms gets a capture it is a one-row change
 * with nothing to touch in the renderer. */
type TrustDialog =
  | { kind: "capture"; src: string; width: number; height: number }
  | { kind: "drawn"; title: string; confirm: string; dismiss: string };

const TRUST_DIALOG: Record<Platform, TrustDialog> = {
  windows: { kind: "capture", src: windowsTrustDialog, width: 516, height: 475 },
  macos: { kind: "drawn", title: "Gate Connect", confirm: "OK", dismiss: "Cancel" },
  linux: {
    kind: "drawn",
    title: "Authentication required",
    confirm: "Authenticate",
    dismiss: "Cancel",
  },
  unknown: { kind: "drawn", title: "Confirm", confirm: "OK", dismiss: "Cancel" },
};

/** The system prompt, drawn or captured per `TRUST_DIALOG`.
 *
 * The point of the drawn ones is not pixel fidelity - it is that a window of
 * roughly this shape is about to appear, that it wants a password rather than a
 * decision, and which button ends it. The captured one carries the certificate
 * name on top of that; its one per-machine line is the thumbprint, which
 * nothing asks the user to check, and the caption is hedged for it. Either way
 * the depiction is `aria-hidden`, with the instruction carried in the panel's
 * real copy. */
function DialogSketch({ platform }: { platform: Platform }) {
  const dialog = TRUST_DIALOG[platform];
  if (dialog.kind === "capture") {
    return (
      <figure>
        <img
          aria-hidden
          alt=""
          src={dialog.src}
          // Intrinsic size of the capture, so the box is reserved before the PNG
          // decodes; `w-full` still drives the rendered width. 280px rather than
          // the drawing's 248 - wider reads better and the panel does not
          // scroll, so the ceiling is what is left of the 620px window: 258px of
          // dialog plus the title, the two paragraphs and the two buttons is
          // ~560. The full 324px of content width would be ~600 and too close.
          width={dialog.width}
          height={dialog.height}
          className="mx-auto block h-auto w-full max-w-[280px] rounded-[10px] shadow-border"
        />
        <figcaption className="mt-1.5 text-gc-label text-gc-ink-3">
          Roughly what your system will show
        </figcaption>
      </figure>
    );
  }
  return (
    <figure>
      <div
        aria-hidden
        className="mx-auto w-full max-w-[248px] rounded-[10px] bg-gc-surface p-3 text-left shadow-border"
      >
        <div className="flex items-center gap-2">
          <span className="flex h-5 w-5 shrink-0 items-center justify-center rounded-md bg-gc-accent-wash text-gc-accent">
            <Icon name="key" size={12} />
          </span>
          <span className="text-gc-body-sm font-medium text-gc-ink">{dialog.title}</span>
        </div>
        {/* Skeleton lines, not lorem text: inventing sentences the OS does not
            say would teach the user to look for words that never appear. */}
        <div className="mt-2 flex-col gap-1">
          <span className="block h-[5px] w-full rounded-full bg-gc-line" />
          <span className="block h-[5px] w-4/5 rounded-full bg-gc-line" />
        </div>
        {/* The password entry, which is the whole point of this drawing: both
            platforms ask for a password rather than for a judgement, and a bare
            sunken bar read as one more skeleton line. "Password" is real text
            because both of them really do write that word; the value is drawn
            as dots, since the OS never shows characters either. The caret sits
            after them so the field reads as focused and waiting. */}
        <div className="mt-2.5">
          <span className="block text-gc-label text-gc-ink-3">Password</span>
          <div className="mt-1 flex h-[22px] items-center gap-[3px] rounded bg-gc-sunken px-1.5 shadow-border">
            {[0, 1, 2, 3, 4, 5, 6].map((i) => (
              <span key={i} className="h-[4px] w-[4px] rounded-full bg-gc-ink-3" />
            ))}
            <span className="ml-[1px] h-[11px] w-px bg-gc-ink" />
          </div>
        </div>
        <div className="mt-2.5 flex items-center justify-end gap-1.5">
          <span className="rounded bg-gc-sunken px-2 py-0.5 text-gc-label text-gc-ink-3">
            {dialog.dismiss}
          </span>
          <span className="rounded bg-gc-accent px-2 py-0.5 text-gc-label font-medium text-white">
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

/** Full-popover pre-flight for the certificate trust, shown when something the
 *  user just turned on needs the CA and the CA is not trusted yet.
 *
 *  `enable()` trusts the CA itself (manager*.rs), so the master switch and
 *  every connect that auto-enables the engine used to spring the OS dialog with
 *  nothing on screen naming it: on Windows a red "Security Warning" quoting a
 *  certificate name, over a popover that hides itself the moment the dialog
 *  steals focus. Home's certificate card explains all this, but it is gated on
 *  routing already being on, so on a fresh machine the first dialog always beat
 *  the card written to prepare the user for it.
 *
 *  A takeover rather than a line on Home: this owns the whole room for the one
 *  moment it matters, which is the room the inline card never had. And it lands
 *  at the moment of need - the user has just asked for something that requires
 *  the certificate - rather than in the tour, where the same warning arrived
 *  before there was any action to explain it. */
export function CertificateNotice({
  platform,
  pending,
  onInstall,
  onDecline,
}: {
  platform: Platform;
  /** The OS dialog is up and we're blocked on it: the copy switches to present
   * tense, because at that point the dialog may be covering this panel. */
  pending: boolean;
  onInstall: () => void;
  onDecline: () => void;
}) {
  const safeRef = useRef<HTMLButtonElement>(null);
  return (
    <Takeover
      z={TAKEOVER_Z.trust}
      labelledBy="certificate-notice-title"
      initialFocus={safeRef}
      // Escape is a decline, but not while we're blocked: the OS dialog owns
      // the decision from that point, and dismissing this panel would leave the
      // user with a system warning and no app on screen - the exact failure the
      // pin exists to prevent.
      onEscape={pending ? undefined : onDecline}
      resetKey={pending}
    >
      {/* The sketch is this panel's tile. The other takeovers open on a 56px
          glyph; here the thing being warned about has a shape, and drawing it is
          worth more than a shield. The drawn platforms carry `key` in accent
          wash rather than a status colour: what they raise is a password entry,
          and warning wash would promise an alarm that only Windows actually
          rings. A shieldCheck would be wrong on all of them - it would promise
          the protection this step has not granted yet. */}
      <DialogSketch platform={platform} />

      <div className="flex flex-col gap-1.5">
        <h1
          id="certificate-notice-title"
          className="text-gc-panel-title font-semibold tracking-[-0.01em] text-gc-ink"
        >
          One prompt to expect
        </h1>
        <p className="text-gc-body-sm leading-snug text-gc-ink-3">
          Apps with no gateway setting of their own route through a proxy on this
          machine, and your {trustStoreName(platform)} has to trust its certificate.
          Generated here, never leaves this machine, removable from Settings
          whenever routing is off.
        </p>
        {/* The handoff sentence, in the same words Home's card and the family
            panel's banner use, so the three surfaces do not describe one
            certificate three ways. `aria-live` because a screen-reader user is
            not told the system dialog opened at all. */}
        <p
          aria-live="polite"
          className={`text-gc-caption leading-snug ${pending ? "text-gc-ink-2" : "text-gc-ink-3"}`}
        >
          {pending ? trustPromptWaiting(platform) : trustPromptHint(platform)}
        </p>
      </div>

      {/* Install first in DOM order, because it is the path the user's own
          switch asked for - but focus goes to Not now, per DESIGN.md: Enter on
          an unread panel must not decide the outcome, and this is the panel
          whose entire job is to be read before the most security-sensitive act
          in the app. Not now is a full secondary button of equal size for the
          same reason, not a text link. */}
      <div className="mt-1 flex w-full flex-col gap-2">
        <Button variant="accent" full disabled={pending} onClick={onInstall}>
          {pending ? "Waiting…" : "Install certificate"}
        </Button>
        <Button ref={safeRef} variant="secondary" full disabled={pending} onClick={onDecline}>
          Not now
        </Button>
      </div>
    </Takeover>
  );
}
