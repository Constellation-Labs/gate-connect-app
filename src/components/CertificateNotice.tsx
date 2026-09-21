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
type Drawn = {
  kind: "polkit" | "security-agent" | "confirm";
  title: string;
  /** The second, quieter line, where we know it exactly. */
  body?: string;
  confirm: string;
  dismiss: string;
};

type TrustDialog = { kind: "capture"; src: string; width: number; height: number } | Drawn;

const TRUST_DIALOG: Record<Platform, TrustDialog> = {
  windows: { kind: "capture", src: windowsTrustDialog, width: 516, height: 475 },
  // Verified against a capture of the real prompt, including the user-domain
  // wording. Newer Macs offer Touch ID first; this is the password path, which
  // is where declining Touch ID lands.
  macos: {
    kind: "security-agent",
    title: "You are making changes to your Certificate Trust Settings.",
    body: "Enter your password to allow this.",
    confirm: "Update Settings",
    dismiss: "Cancel",
  },
  // Verified against a capture of the real prompt on Ubuntu (GNOME/polkit),
  // down to the capital R.
  linux: {
    kind: "polkit",
    title: "Authentication Required",
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
  if (dialog.kind === "polkit") return <PolkitSketch dialog={dialog} />;
  if (dialog.kind === "security-agent") return <SecurityAgentSketch dialog={dialog} />;
  return <ConfirmSketch dialog={dialog} />;
}

/** Linux: the GNOME/polkit prompt, drawn from a capture of the real dialog on
 * Ubuntu rather than from the shape of the code.
 *
 * It is drawn and not shipped as that capture for the reason the Windows one
 * is shipped: the Windows warning is about *our* certificate, so it says the
 * same thing on every machine, while this one is about *the user* - it carries
 * their own photo and login name, which would be a stranger's on every screen
 * but the one it was taken on. So the account is drawn blank.
 *
 * What the earlier drawing got wrong, all of it visible in the real dialog:
 * the content is centred, not left-aligned; there is no status glyph, the
 * account avatar is the only mark; and the buttons are a split bar across the
 * foot where the confirm is *dimmed* until a password is typed, rather than a
 * coloured default. Drawing the confirm as the bright, obvious button taught
 * the user to look for something that is not there until they have typed. */
function PolkitSketch({ dialog }: { dialog: Drawn }) {
  return (
    <Depiction>
      <div className="mx-auto w-full max-w-[248px] overflow-hidden rounded-[10px] bg-gc-surface text-center shadow-border">
        <div className="px-3 pb-2.5 pt-3">
          <span className="block text-gc-body-sm font-medium text-gc-ink">{dialog.title}</span>
          {/* Skeleton lines, not lorem text: inventing sentences the OS does
              not say would teach the user to look for words that never appear.
              The real line names the command being run as root, which is ours
              and unreadable at this size either way. */}
          <div className="mt-2 flex flex-col items-center gap-1">
            <span className="block h-[5px] w-4/5 rounded-full bg-gc-line" />
            <span className="block h-[5px] w-3/5 rounded-full bg-gc-line" />
          </div>
          <span className="mx-auto mt-2.5 block h-6 w-6 rounded-full bg-gc-sunken" />
          {/* The empty state, which is the dialog as it appears: "Password" is
              the field's own placeholder rather than a label above it, and it
              pairs with the dimmed confirm below - that button is dimmed
              precisely because nothing has been typed yet. */}
          <div className="mt-2.5 flex h-[22px] items-center justify-between rounded bg-gc-sunken px-1.5 shadow-border">
            <span className="text-gc-label text-gc-ink-4">Password</span>
            <Icon name="eye" size={11} />
          </div>
        </div>
        {/* A hairline rather than the `shadow-border` stack: this is depicting
            another OS's chrome, not dressing one of our own surfaces. */}
        <div className="h-px w-full bg-gc-line" />
        <div className="flex text-gc-label">
          <span className="flex-1 py-1.5 text-gc-ink-3">{dialog.dismiss}</span>
          <span className="w-px self-stretch bg-gc-line" />
          <span className="flex-1 py-1.5 text-gc-ink-5">{dialog.confirm}</span>
        </div>
      </div>
    </Depiction>
  );
}

/** macOS: the Security Agent prompt that `security add-trusted-cert` raises
 * when it changes trust settings. Drawn from a capture of the real one.
 *
 * Nothing like polkit's, and nothing like the older macOS screenshots that
 * circulate either - those put the icon left of the text with the buttons
 * bottom right. The current one is a narrow vertical card: a gold padlock on
 * top, the process name under it, two stacked full-width fields, and the
 * buttons stacked full-width with the confirm *above* Cancel. Neither button
 * is a coloured default.
 *
 * Both sentences are the Security Agent's own, and they are the user trust
 * domain's wording ("your Certificate Trust Settings", not the admin domain's
 * "System"), which is the domain `ca::ensure_trusted` writes to: it installs
 * into the login keychain. */
function SecurityAgentSketch({ dialog }: { dialog: Drawn }) {
  return (
    <Depiction>
      <div className="mx-auto w-full max-w-[208px] rounded-[10px] bg-gc-page p-3 text-left shadow-border">
        {/* The gold padlock is the first thing on screen. Warning wash is the
            nearest this palette has to it, and it is depicting macOS's icon
            rather than claiming a status of our own. */}
        <span className="mx-auto mb-2 flex h-9 w-9 items-center justify-center rounded-lg bg-gc-warning-wash text-gc-warning-deep">
          <Icon name="key" size={18} />
        </span>
        {/* macOS puts the *process* name in bold, and it is `security` - the
            binary `ca::ensure_trusted` shells out to - not "Gate Connect". A
            user hunting for our name will not find it, which is worth drawing
            rather than quietly improving. */}
        <span className="block text-gc-label font-semibold text-gc-ink">security</span>
        <span className="mt-1.5 block text-gc-label leading-snug text-gc-ink-2">
          {dialog.title}
        </span>
        <span className="mt-1.5 block text-gc-label leading-snug text-gc-ink-2">{dialog.body}</span>
        <div className="mt-2.5 flex flex-col gap-1">
          {/* Filled but nameless: the real field carries this user's own
              account name, which is why this stays a drawing. */}
          <span className="flex h-[18px] items-center rounded bg-gc-sunken px-1.5">
            <span className="block h-[5px] w-2/5 rounded-full bg-gc-line-strong" />
          </span>
          <span className="flex h-[18px] items-center rounded bg-gc-sunken px-1.5 text-gc-label text-gc-ink-4">
            Password
          </span>
        </div>
        <div className="mt-2 flex flex-col gap-1 text-center text-gc-label text-gc-ink">
          <span className="rounded bg-gc-surface py-1 shadow-border">{dialog.confirm}</span>
          <span className="rounded bg-gc-surface py-1 shadow-border">{dialog.dismiss}</span>
        </div>
      </div>
    </Depiction>
  );
}

/** The fallback shape for `unknown`, where we do not know what will appear:
 * something will ask, and there are two ways out of it. No password field,
 * because `trustPromptHint` declines to promise one. */
function ConfirmSketch({ dialog }: { dialog: Drawn }) {
  return (
    <Depiction>
      <div className="mx-auto w-full max-w-[248px] rounded-[10px] bg-gc-surface p-3 text-left shadow-border">
        <div className="flex items-center gap-2">
          <span className="flex h-5 w-5 shrink-0 items-center justify-center rounded-md bg-gc-accent-wash text-gc-accent">
            <Icon name="info" size={12} />
          </span>
          <span className="text-gc-body-sm font-medium text-gc-ink">{dialog.title}</span>
        </div>
        <div className="mt-2 flex flex-col gap-1">
          <span className="block h-[5px] w-full rounded-full bg-gc-line" />
          <span className="block h-[5px] w-4/5 rounded-full bg-gc-line" />
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
        {/* Two short sentences: what is about to happen, then why it is safe.
            The old copy opened on "Apps with no gateway setting of their own",
            which asks the reader to hold a category they have no use for at the
            moment a system dialog is about to steal focus. The mechanism lives
            on Home's certificate card, where there is room to read it. */}
        <p className="text-gc-body-sm leading-snug text-gc-ink-3">
          Gate Connect routes some apps through a proxy on this machine, so your
          {" "}
          {trustStoreName(platform)} needs to trust its certificate. It is made
          here, never leaves your computer, and you can remove it in Settings.
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
