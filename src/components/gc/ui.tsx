import { forwardRef } from "react";
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
  /** `sm` is for dense chrome (the onboarding window's 52px footer), not the
   * popover, where buttons stay `md`. */
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
    size === "sm" ? "h-8 px-3.5 text-[12.5px]" : "h-10 px-4 text-[13.5px]";
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
} & ButtonHTMLAttributes<HTMLButtonElement>) {
  return (
    <button
      type="button"
      className={`flex h-7 w-7 items-center justify-center rounded text-gc-ink-3 transition hover:bg-gc-subtle hover:text-gc-ink-2 focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-1 focus-visible:outline-gc-accent disabled:opacity-40 ${className}`}
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
      className={`relative inline-flex h-[22px] w-[38px] shrink-0 items-center rounded-full transition-colors focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-gc-accent disabled:opacity-50 ${busy ? "opacity-70" : ""} ${on ? "bg-gc-accent" : "bg-gc-switch-off"}`}
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
    <h2 className="px-3.5 pb-1.5 pt-3 font-mono text-[10.5px] font-medium uppercase tracking-[0.08em] text-gc-ink-3">
      {children}
    </h2>
  );
}

export function Input({
  leadingIcon,
  trailing,
  className = "",
  ...rest
}: {
  leadingIcon?: ReactNode;
  trailing?: ReactNode;
} & InputHTMLAttributes<HTMLInputElement>) {
  return (
    <div className="flex h-9 items-center gap-2 rounded bg-gc-surface px-3 shadow-border transition focus-within:shadow-border-hover">
      {leadingIcon && <span className="shrink-0 text-gc-ink-4">{leadingIcon}</span>}
      <input
        className={`min-w-0 flex-1 bg-transparent text-[13px] text-gc-ink outline-none placeholder:text-gc-ink-3 ${className}`}
        {...rest}
      />
      {trailing && <span className="flex shrink-0 items-center gap-0.5">{trailing}</span>}
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
      <span className="inline-flex items-center gap-1.5 rounded-gc-pill bg-gc-success-wash px-2 py-1 text-[11px] font-medium text-gc-success-deep">
        <span className="h-1.5 w-1.5 rounded-full bg-gc-success-deep" />
        {label ?? "Connected"}
      </span>
    );
  }
  // Partial: the system is genuinely half-on (routing up, CA untrusted), so
  // the pill tells that truth instead of rounding up to green.
  if (state === "partial") {
    return (
      <span className="inline-flex items-center gap-1.5 rounded-gc-pill bg-gc-warning-wash px-2 py-1 text-[11px] font-medium text-gc-ink-2">
        <span className="h-1.5 w-1.5 rounded-full bg-gc-warning-deep" />
        {label ?? "Partly routed"}
      </span>
    );
  }
  const fallback = state === "idle" ? "Idle" : "Signed out";
  return (
    <span className="inline-flex items-center gap-1.5 rounded-gc-pill bg-gc-sunken px-2 py-1 text-[11px] font-medium text-gc-ink-3">
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
        className="text-[14px] font-semibold tracking-[-0.01em] text-gc-ink outline-none"
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
  return (
    <div role="alert" className={`flex gap-2.5 rounded bg-gc-sunken px-3 py-2.5 text-left ${className}`}>
      <Icon name="info" size={15} className="mt-px shrink-0 text-gc-error" />
      <div className="min-w-0 flex-1">
        <div className="text-[11.5px] font-semibold leading-snug text-gc-ink">{error.title}</div>
        <div className="mt-0.5 text-[11.5px] leading-snug text-gc-ink-2">{error.hint}</div>
        {error.raw && error.raw !== error.title && (
          <details className="mt-1">
            <summary className="cursor-pointer py-0.5 text-[11px] text-gc-ink-3">Details</summary>
            <div className="mt-1 break-all font-mono text-[10.5px] leading-snug text-gc-ink-3">
              {error.raw}
            </div>
          </details>
        )}
      </div>
    </div>
  );
}
