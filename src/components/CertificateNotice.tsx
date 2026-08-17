import { useRef } from "react";
import { Takeover, TAKEOVER_Z } from "./Takeover";
import { Button } from "./gc/ui";
import { Icon } from "./gc/Icon";
import { trustPromptHint, trustPromptWaiting, trustStoreName, type Platform } from "../lib/platform";

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

/** The system dialog, drawn rather than screenshotted.
 *
 * A capture would be wrong on half the machines that see it (the warning is
 * worded and framed differently across Windows versions), and the point is not
 * pixel fidelity - it is that a window of roughly this shape is about to
 * appear, and which button ends it. Drawn in the app's own tokens and
 * `aria-hidden`, with the instruction carried in the panel's real copy. */
function DialogSketch({ platform }: { platform: Platform }) {
  const dialog = TRUST_DIALOG[platform];
  return (
    <figure>
      <div
        aria-hidden
        className="mx-auto w-full max-w-[248px] rounded-[10px] bg-gc-surface p-3 text-left shadow-border"
      >
        <div className="flex items-center gap-2">
          <span className="flex h-5 w-5 shrink-0 items-center justify-center rounded-md bg-gc-warning-wash text-gc-warning-deep">
            <Icon name="info" size={12} />
          </span>
          <span className="text-gc-body-sm font-medium text-gc-ink">{dialog.title}</span>
        </div>
        {/* Skeleton lines, not lorem text: inventing sentences the OS does not
            say would teach the user to look for words that never appear. */}
        <div className="mt-2 flex-col gap-1">
          <span className="block h-[5px] w-full rounded-full bg-gc-line" />
          <span className="block h-[5px] w-4/5 rounded-full bg-gc-line" />
        </div>
        {dialog.password && <div className="mt-2 h-[22px] rounded bg-gc-sunken" />}
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
          worth more than a shield. Its own glyph is `info` in warning wash, this
          system's mark for "read before you confirm" (the routing notice and
          Home's certificate card both pick it) - a shieldCheck would promise the
          protection this step has not granted yet. */}
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
