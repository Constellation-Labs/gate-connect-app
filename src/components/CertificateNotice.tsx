import { useRef } from "react";
import type { ReactNode } from "react";
import { Takeover, TAKEOVER_Z } from "./Takeover";
import { Button } from "./gc/ui";
import { Icon } from "./gc/Icon";
import { trustPromptHint, trustPromptWaiting, trustStoreName, type Platform } from "../lib/platform";
import windowsTrustDialog from "../assets/windows-certificate-warning.png";

/** How each platform's trust prompt is depicted, and what it is.
 *
 * Three shapes, because the platforms split three ways. Cited by function
 * rather than by line, because line numbers rot.
 *
 * `capture` - Windows. `certutil -user -addstore Root` raises a "Security
 * Warning" that asks the user to weigh something, and we ship a capture of the
 * real one. What the capture carries is the shape and the two buttons: at the
 * width this popover can give it (280 of the dialog's native 516) the body text
 * is not readable, so the certificate's name is carried as real text by
 * `trustPromptWaiting`, not by the picture.
 *
 * `password` - macOS and Linux, which ask for a password rather than for a
 * judgement. macOS runs `security add-trusted-cert` directly (`ca::ensure_trusted`)
 * and the trust-settings change raises the Security Agent; note that this path
 * deliberately does *not* go through osascript, for the reason set out in the
 * header of `crates/core/src/proxy/ca.rs`. Linux escalates
 * `ca_linux::ensure_trusted` through `primitives::run_as_admin`, which picks
 * `pkexec` in a GUI session and `sudo` on a tty - the drawn strings are
 * polkit's, so they describe the GUI case, which is the one a popover user is
 * in.
 *
 * `confirm` - `unknown`, where we do not know what will appear.
 * `trustPromptHint` deliberately promises only that "your system will ask you
 * to confirm", so the drawing must not show a password field the copy declines
 * to promise. */
type TrustDialog =
  | { kind: "capture"; src: string; width: number; height: number }
  | { kind: "password" | "confirm"; title: string; confirm: string; dismiss: string };

const TRUST_DIALOG: Record<Platform, TrustDialog> = {
  windows: { kind: "capture", src: windowsTrustDialog, width: 516, height: 475 },
  // UNVERIFIED: raised by `/usr/bin/security`, not by this app, and not
  // reachable from a Linux dev box. Check against a real Mac before trusting
  // these three strings; the shape (a password is wanted) is the part that is
  // certain.
  macos: {
    kind: "password",
    title: "Certificate Trust Settings",
    confirm: "Update Settings",
    dismiss: "Cancel",
  },
  linux: {
    kind: "password",
    title: "Authentication required",
    confirm: "Authenticate",
    dismiss: "Cancel",
  },
  unknown: { kind: "confirm", title: "Confirm", confirm: "OK", dismiss: "Cancel" },
};

/** The frame every depiction shares.
 *
 * `aria-hidden` sits on the `<figure>` rather than on the drawing inside it, so
 * it covers the caption too: a screen reader that is handed no picture should
 * not be told what the picture roughly shows. Everything it needs is in the
 * panel's real copy. */
function Depiction({ children }: { children: ReactNode }) {
  return (
    <figure aria-hidden>
      {children}
      <figcaption className="mt-1.5 text-gc-label text-gc-ink-3">
        Roughly what your system will show
      </figcaption>
    </figure>
  );
}

/** The system prompt, drawn or captured per `TRUST_DIALOG`.
 *
 * The point of the drawn ones is not pixel fidelity - it is that a window of
 * roughly this shape is about to appear, what it wants from the user, and which
 * button ends it. The captured one adds the real furniture of the Windows
 * warning, at a size where the shape and the buttons read and the body text
 * does not. */
function DialogSketch({ platform }: { platform: Platform }) {
  const dialog = TRUST_DIALOG[platform];
  if (dialog.kind === "capture") {
    return (
      <Depiction>
        <img
          alt=""
          src={dialog.src}
          // Intrinsic size of the capture, so the box is reserved before the
          // PNG decodes; `w-full` still drives the rendered width.
          //
          // 280px, and deliberately a px literal rather than a rem one - the
          // only element here off the rem ramp. Two reasons. It is a bitmap
          // captured at 1x, so growing it past native only enlarges its
          // blur, and it is `aria-hidden` decoration: the reader who turned
          // the text up is served by the copy below, which does scale. And
          // the panel is the tallest in the app at 567px of the 620px window,
          // so every px this adds comes off the margin that keeps the buttons
          // on screen when `growWindow` is capped by a short display.
          //
          // 280 is also near the ceiling: 324px (the full content width) puts
          // the panel at ~607px, which leaves 6px of clearance.
          width={dialog.width}
          height={dialog.height}
          className="mx-auto block h-auto w-full max-w-[280px] rounded-[10px] shadow-border"
        />
      </Depiction>
    );
  }
  return (
    <Depiction>
      <div className="mx-auto w-full max-w-[248px] rounded-[10px] bg-gc-surface p-3 text-left shadow-border">
        <div className="flex items-center gap-2">
          <span className="flex h-5 w-5 shrink-0 items-center justify-center rounded-md bg-gc-accent-wash text-gc-accent">
            <Icon name={dialog.kind === "password" ? "key" : "info"} size={12} />
          </span>
          <span className="text-gc-body-sm font-medium text-gc-ink">{dialog.title}</span>
        </div>
        {/* Skeleton lines, not lorem text: inventing sentences the OS does not
            say would teach the user to look for words that never appear. */}
        <div className="mt-2 flex flex-col gap-1">
          <span className="block h-[5px] w-full rounded-full bg-gc-line" />
          <span className="block h-[5px] w-4/5 rounded-full bg-gc-line" />
        </div>
        {/* The password entry, on the two platforms that ask for one. A bare
            sunken bar read as one more skeleton line, so the field is labelled:
            "Password" is real text because macOS and Linux both really do write
            that word, while the value stays drawn as dots, since neither shows
            characters either. `unknown` is excluded on purpose - we do not know
            that its prompt wants a password, and `trustPromptHint` promises
            only that something will ask the user to confirm. */}
        {dialog.kind === "password" && (
          <div className="mt-2.5">
            <span className="block text-gc-label text-gc-ink-3">Password</span>
            <div className="mt-1 flex h-[22px] items-center gap-[3px] rounded bg-gc-sunken px-1.5 shadow-border">
              {[0, 1, 2, 3, 4, 5, 6].map((i) => (
                <span key={i} className="h-[4px] w-[4px] rounded-full bg-gc-ink-3" />
              ))}
            </div>
          </div>
        )}
        <div className="mt-2.5 flex items-center justify-end gap-1.5">
          <span className="rounded bg-gc-sunken px-2 py-0.5 text-gc-label text-gc-ink-3">
            {dialog.dismiss}
          </span>
          <span className="rounded bg-gc-accent px-2 py-0.5 text-gc-label font-medium text-white">
            {dialog.confirm}
          </span>
        </div>
      </div>
    </Depiction>
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
      {/* The depiction is this panel's tile. The other takeovers open on a 56px
          glyph; here the thing being warned about has a shape, and showing it is
          worth more than a shield. The drawn platforms carry their glyph in
          accent wash rather than a status colour - `key` where a password is
          wanted, `info` where we only know something will ask - because warning
          wash would promise an alarm that only Windows actually rings. A
          shieldCheck would be wrong on all of them: it would promise the
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
