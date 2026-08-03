import { useRef, useState } from "react";
import { track, trackError } from "../lib/analytics";
import { classifyError, type ClassifiedError } from "../lib/errors";
import { useFocusTrap } from "../lib/useFocusTrap";
import { Button, ErrorNote } from "./gc/ui";
import { Icon } from "./gc/Icon";

/** Full-popover offer, shown once, to accounts still on a pasted API key.
 *
 *  OAuth is the primary path: FirstRun leads with it and the key form is a
 *  collapsed alternative. But anyone who installed before OAuth existed, or
 *  who took the key path once, has no reason to revisit the decision - and
 *  Settings deliberately keeps its switch-over as a quiet row, because a
 *  permanent accent CTA there out-shouted the user's own key on the screen
 *  they open to check that key.
 *
 *  So the offer lives here instead: once, at the moment the app opens, with a
 *  real way to decline that is remembered. Declining is not a lesser choice -
 *  a pasted key is still supported, and the copy says so. */
export function OAuthOffer({
  onUpgrade,
  onDismiss,
}: {
  /** Begins the browser sign-in. Resolves once the flow is handed off. */
  onUpgrade: () => Promise<void>;
  /** Always marks the offer seen, whichever way the user leaves. */
  onDismiss: () => void;
}) {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<ClassifiedError | null>(null);
  const panelRef = useRef<HTMLDivElement>(null);
  // An offer the user did not ask for should not open with its accept
  // focused: Space or Enter would launch a browser sign-in flow.
  const safeRef = useRef<HTMLButtonElement>(null);
  useFocusTrap(panelRef, onDismiss, safeRef);

  async function upgrade() {
    setBusy(true);
    setError(null);
    try {
      await onUpgrade();
      track("oauth_offer_accepted");
      onDismiss();
    } catch (e) {
      trackError(e, "sign_in");
      setError(classifyError(e, "sign_in"));
      setBusy(false);
    }
  }

  return (
    <div
      ref={panelRef}
      role="dialog"
      aria-modal="true"
      aria-labelledby="oauth-offer-title"
      className="gc-panel-in absolute inset-0 z-10 flex flex-col items-center justify-center gap-5 bg-gc-surface px-7 text-center"
    >
      <div className="flex h-14 w-14 items-center justify-center rounded-gc-lg bg-gc-accent-wash text-gc-accent">
        <Icon name="shieldCheck" size={26} />
      </div>

      <div className="flex flex-col gap-1.5">
        <h1
          id="oauth-offer-title"
          className="text-[17px] font-semibold tracking-[-0.01em] text-gc-ink"
        >
          Sign in instead of pasting a key
        </h1>
        <p className="text-[12.5px] leading-snug text-gc-ink-3">
          Constellation sign-in keeps your session in the keychain and refreshes
          it on its own, so there&rsquo;s nothing to rotate when a key expires.
          Your gateway and your routing stay exactly as they are.
        </p>
        {error && <ErrorNote error={error} />}
      </div>

      <div className="mt-1 flex w-full flex-col gap-2">
        <Button variant="accent" full disabled={busy} onClick={() => void upgrade()}>
          <Icon name="shieldCheck" size={15} />
          {busy ? "Waiting for browser…" : "Sign in with Constellation"}
        </Button>
        {/* Not a throwaway "Not now": a pasted key is a supported choice, and
            this says so rather than implying the user is postponing. */}
        <Button ref={safeRef} variant="secondary" full disabled={busy} onClick={onDismiss}>
          Keep using my API key
        </Button>
      </div>

      {/* ink-3, not ink-4: DESIGN.md names ink-3 as the smallest ink that may
          carry real text, and ink-4 measures 4.0:1 on white. */}
      <p className="text-[11px] leading-snug text-gc-ink-3">
        You can switch either way later, under Account in Settings.
      </p>
    </div>
  );
}
