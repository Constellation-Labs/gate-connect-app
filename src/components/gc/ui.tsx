import { forwardRef, useEffect, useState } from "react";
import type {
  ButtonHTMLAttributes,
  InputHTMLAttributes,
  ReactNode,
} from "react";
import type { ClassifiedError } from "../../lib/errors";
import { Icon, type IconName } from "./Icon";

/** Shared primitives for the Gate Connect popover, built on the `gc-*`
 *  Tailwind tokens. Visual port of the prototype's primitives/kit. */

/** `forwardRef` so a panel can point its focus trap at a specific button -
 * the confirms need focus to land on the safe choice, not on whichever
 * destructive action happens to come first in DOM order. */
export const Button = forwardRef<HTMLButtonElement, {
  variant?: "accent" | "secondary" | "danger";
  /** `md` (40px, the DESIGN.md button spec) is the default and is what
   * standalone and full-width actions use. `sm` (32px) is for a button
   * *embedded* in something else, where a 40px control would outweigh the
   * thing it sits in: an inline banner, an expanded row, an inline confirm
   * pair, or the onboarding window's 52px footer. This doc used to say `sm`
   * never appears in the popover, which eight call sites had already
   * disproved; the rule is about the container, not the window. */
  size?: "sm" | "md";
  full?: boolean;
} & ButtonHTMLAttributes<HTMLButtonElement>>(function Button(
  { variant = "secondary", size = "md", full, type = "button", className = "", children, ...rest },
  ref,
) {
  // Indigo is defined as affordance and live state - the encouraged path - so
  // it must never be the thing that destroys work. `danger` gives the two
  // destructive confirms (close everything, reset everything) one grammar of
  // their own instead of borrowing the primary skin.
  const styles =
    variant === "accent"
      ? "bg-gc-accent text-white hover:bg-gc-accent-ink active:bg-gc-accent-ink"
      : variant === "danger"
        ? "bg-gc-error-deep text-white hover:brightness-95 active:brightness-90"
        : "bg-gc-surface text-gc-ink shadow-border hover:shadow-border-hover";
  const sizing =
    size === "sm" ? "h-8 px-3.5 text-gc-body-sm" : "h-10 px-4 text-gc-body";
  return (
    <button
      ref={ref}
      // Never inherit the implicit "submit": these sit next to inputs (the
      // key form), where a stray submit would reload the webview.
      type={type}
      className={`inline-flex items-center justify-center gap-2 rounded font-medium transition focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-gc-accent disabled:pointer-events-none disabled:opacity-45 ${sizing} ${styles} ${full ? "w-full" : ""} ${className}`}
      {...rest}
    >
      {children}
    </button>
  );
});

export function IconButton({
  icon,
  size = 15,
  className = "",
  ...rest
}: {
  icon: IconName;
  size?: number;
  /** Required: the button's only content is a glyph, so without this it has no
   * accessible name at all. Every call site already passes one; the type is
   * what stops the next one from forgetting. */
  "aria-label": string;
} & ButtonHTMLAttributes<HTMLButtonElement>) {
  return (
    <button
      type="button"
      className={`flex h-7 w-7 items-center justify-center rounded text-gc-ink-3 transition hover:bg-gc-subtle hover:text-gc-ink-2 focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-1 focus-visible:outline-gc-accent disabled:opacity-45 ${className}`}
      {...rest}
    >
      <Icon name={icon} size={size} />
    </button>
  );
}

export function Switch({
  on,
  label,
  describedBy,
  onClick,
  disabled,
  busy,
  className = "",
}: {
  on: boolean;
  /** Id of a node describing what is actually happening, for the cases where
   * the switch's own state (intent) and the observable result differ. */
  describedBy?: string;
  /** Accessible name: what this switch controls ("Route through Gate",
   * a provider's display name, "Launch at login"). */
  label: string;
  onClick?: () => void;
  /** Structurally unusable (nothing installed, still loading): removed from
   * the tab order. */
  disabled?: boolean;
  /** An operation is in flight: clicks are ignored but the switch keeps
   * keyboard focus, so toggling never ejects a keyboard user mid-flip. */
  busy?: boolean;
  /** Layout only, like Button and IconButton already take. Used where a wrapping
   * flex row needs the switch pushed to the end of its line. */
  className?: string;
}) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={on}
      aria-label={label}
      aria-describedby={describedBy}
      aria-busy={busy || undefined}
      aria-disabled={busy || undefined}
      disabled={disabled}
      onClick={busy ? undefined : onClick}
      // `before:`: the visible track is the locked 38x22 from DESIGN.md, which
      // is 2px under the 24px target minimum, and in both ledgers it paints on
      // top of a sibling `absolute inset-0` button covering the whole row. A
      // 24px circle centered on it therefore always intersects another target,
      // so neither the Inline nor the Spacing exception applies, and the
      // consequence is real: missing the family switch by 2px opens the detail
      // panel instead of routing the family, with no error either way. The
      // pseudo-element takes 4px back from the row's padding to make the hit
      // area 30px without moving a pixel of the switch or its focus ring.
      className={`relative inline-flex h-[22px] w-[38px] shrink-0 items-center rounded-full transition-colors before:absolute before:inset-x-0 before:-inset-y-1 before:content-[''] focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-gc-accent disabled:opacity-45 ${busy ? "opacity-70" : ""} ${on ? "bg-gc-accent" : "bg-gc-switch-off"} ${className}`}
    >
      <span
        className={`absolute flex h-[18px] w-[18px] items-center justify-center rounded-full bg-white shadow-sm transition-transform ${on ? "translate-x-[18px]" : "translate-x-[2px]"}`}
      >
        {on && <Icon name="check" size={11} stroke={3} className="text-gc-accent" />}
      </span>
    </button>
  );
}

export function CardButton({
  onClick,
  children,
}: {
  onClick?: () => void;
  children: ReactNode;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      className="flex w-full items-center gap-3 rounded-[10px] bg-gc-surface p-3.5 text-left shadow-border transition hover:shadow-border-hover focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-gc-accent"
    >
      {children}
    </button>
  );
}

export function SectionLabel({ children }: { children: ReactNode }) {
  return (
    <h2 className="px-3.5 pb-1.5 pt-3 font-mono text-gc-label font-medium uppercase tracking-[0.08em] text-gc-ink-3">
      {children}
    </h2>
  );
}

export function Input({
  leadingIcon,
  secret,
  className = "",
  ...rest
}: {
  leadingIcon?: ReactNode;
  /** Masks the value and adds a reveal toggle. For the `sk-gw-` field: a live
   * gateway key is the one string in this app worth hiding while it is being
   * pasted, and Settings already puts a confirm in front of revealing the
   * stored one - typing it in the clear was the louder inconsistency. */
  secret?: boolean;
} & InputHTMLAttributes<HTMLInputElement>) {
  const [revealed, setRevealed] = useState(false);
  return (
    <div className="flex h-9 items-center gap-2 rounded bg-gc-surface px-3 shadow-border transition focus-within:shadow-border-hover">
      {leadingIcon && <span className="shrink-0 text-gc-ink-4">{leadingIcon}</span>}
      <input
        type={secret && !revealed ? "password" : "text"}
        className={`min-w-0 flex-1 bg-transparent text-gc-body-md text-gc-ink outline-none placeholder:text-gc-ink-3 ${className}`}
        {...rest}
      />
      {/* `IconButton`, not a bare button: this was a 14px glyph with no box at
          all, half the size of every other icon control in the app, for the
          action that decides whether a live key is on screen. The negative
          margin keeps the glyph optically where it was while the hit area grows
          to 28px around it. */}
      {secret && (
        <IconButton
          icon={revealed ? "eyeOff" : "eye"}
          size={14}
          onClick={() => setRevealed((v) => !v)}
          aria-label={revealed ? "Hide key" : "Show key"}
          className="-mr-1.5 shrink-0"
        />
      )}
    </div>
  );
}

export function ConnPill({
  state = "connected",
  label,
}: {
  state?: "connected" | "partial" | "idle" | "signedout";
  /** Overrides the default pill text so each surface can say what the state
   * actually means there ("Routing on" in the header, "Signed in" in
   * Settings) instead of an ambiguous "Connected". */
  label?: string;
}) {
  if (state === "connected") {
    return (
      <span className="inline-flex items-center gap-1.5 rounded-gc-pill bg-gc-success-wash px-2 py-1 text-gc-micro font-medium text-gc-success-deep ring-1 ring-gc-success-deep/30">
        <span className="h-1.5 w-1.5 rounded-full bg-gc-success-deep" />
        {label ?? "Connected"}
      </span>
    );
  }
  // Partial: the system is genuinely half-on (routing up, CA untrusted), so
  // the pill tells that truth instead of rounding up to green.
  if (state === "partial") {
    return (
      <span className="inline-flex items-center gap-1.5 rounded-gc-pill bg-gc-warning-wash px-2 py-1 text-gc-micro font-medium text-gc-ink-2 ring-1 ring-gc-warning-deep/45">
        <span className="h-1.5 w-1.5 rounded-full bg-gc-warning-deep" />
        {label ?? "Partly routed"}
      </span>
    );
  }
  const fallback = state === "idle" ? "Idle" : "Signed out";
  return (
    <span className="inline-flex items-center gap-1.5 rounded-gc-pill bg-gc-sunken px-2 py-1 text-gc-micro font-medium text-gc-ink-3 ring-1 ring-gc-ink-4/45">
      <span className="h-1.5 w-1.5 rounded-full bg-gc-ink-3" />
      {label ?? fallback}
    </span>
  );
}

export function SubHeader({
  title,
  onBack,
}: {
  title: string;
  onBack: () => void;
}) {
  return (
    <div className="sticky top-0 z-[5] flex items-center gap-1.5 border-b border-gc-line bg-gc-surface px-2.5 py-2.5">
      <IconButton icon="chevronLeft" size={18} onClick={onBack} aria-label="Back" />
      {/* Real heading + programmatic focus target: App moves focus here on
          panel entry so screen changes are announced and Tab starts at the
          top of the new panel. */}
      <h1
        tabIndex={-1}
        data-screen-focus
        className="text-gc-title-sm font-semibold tracking-[-0.01em] text-gc-ink outline-none"
      >
        {title}
      </h1>
    </div>
  );
}

/** Failure surfaced in plain language: what failed, what to do, with the raw
 * backend payload tucked behind a disclosure for reporting. Pair with
 * classifyError so no screen ever prints String(e) directly. Per the
 * Wash-First rule the status color rides the icon, not the text: the title
 * stays ink so it holds AA at this size. */
export function ErrorNote({
  error,
  className = "",
}: {
  error: ClassifiedError;
  className?: string;
}) {
  const [copied, setCopied] = useState(false);
  // Timed reset with a cleanup, rather than a bare setTimeout in the handler:
  // dismissing the note inside the 1.6s window set state on an unmounted
  // component, and the note is dismissed by whatever the user does next.
  useEffect(() => {
    if (!copied) return;
    const t = setTimeout(() => setCopied(false), 1600);
    return () => clearTimeout(t);
  }, [copied]);
  return (
    <div role="alert" className={`flex gap-2.5 rounded bg-gc-sunken px-3 py-2.5 text-left ${className}`}>
      <Icon name="info" size={15} className="mt-px shrink-0 text-gc-error" />
      <div className="min-w-0 flex-1">
        <div className="text-gc-caption font-semibold leading-snug text-gc-ink">{error.title}</div>
        <div className="mt-0.5 text-gc-caption leading-snug text-gc-ink-2">{error.hint}</div>
        {error.raw && error.raw !== error.title && (
          <details className="mt-1">
            <summary className="cursor-pointer py-0.5 text-gc-micro text-gc-ink-3">Details</summary>
            <div className="mt-1 break-all font-mono text-gc-label leading-snug text-gc-ink-3">
              {error.raw}
            </div>
            {/* Five error branches tell the user "the details below help when
                reporting it" and then gave them a break-all mono blob to
                select by hand inside a 360px popover. */}
            <button
              type="button"
              onClick={() => {
                void navigator.clipboard.writeText(error.raw).then(() => setCopied(true));
              }}
              className="mt-1.5 inline-flex items-center gap-1 text-gc-micro font-medium text-gc-ink-3 transition hover:text-gc-ink"
            >
              <Icon name={copied ? "check" : "copy"} size={12} />
              {copied ? "Copied" : "Copy details"}
            </button>
          </details>
        )}
      </div>
    </div>
  );
}
