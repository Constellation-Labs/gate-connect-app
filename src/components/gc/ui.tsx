import type {
  ButtonHTMLAttributes,
  InputHTMLAttributes,
  ReactNode,
} from "react";
import { Icon, type IconName } from "./Icon";

/** Shared primitives for the Gate Connect popover, built on the `gc-*`
 *  Tailwind tokens. Visual port of the prototype's primitives/kit. */

export function Button({
  variant = "secondary",
  full,
  className = "",
  children,
  ...rest
}: {
  variant?: "accent" | "secondary";
  full?: boolean;
} & ButtonHTMLAttributes<HTMLButtonElement>) {
  const styles =
    variant === "accent"
      ? "bg-gc-accent text-white hover:brightness-110 active:brightness-95"
      : "bg-gc-surface text-gc-ink shadow-border hover:shadow-border-hover";
  return (
    <button
      className={`inline-flex h-10 items-center justify-center gap-2 rounded px-4 text-[13.5px] font-medium transition disabled:pointer-events-none disabled:opacity-45 ${styles} ${full ? "w-full" : ""} ${className}`}
      {...rest}
    >
      {children}
    </button>
  );
}

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
      className={`flex h-7 w-7 items-center justify-center rounded text-gc-ink-3 transition hover:bg-gc-subtle hover:text-gc-ink-2 disabled:opacity-40 ${className}`}
      {...rest}
    >
      <Icon name={icon} size={size} />
    </button>
  );
}

export function Switch({
  on,
  onClick,
  disabled,
}: {
  on: boolean;
  onClick?: () => void;
  disabled?: boolean;
}) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={on}
      disabled={disabled}
      onClick={onClick}
      className={`relative inline-flex h-[22px] w-[38px] shrink-0 items-center rounded-full transition-colors disabled:opacity-50 ${on ? "bg-gc-accent" : "bg-gc-line-strong"}`}
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
      className="flex w-full items-center gap-3 rounded-[10px] bg-gc-surface p-3.5 text-left shadow-border transition hover:shadow-border-hover"
    >
      {children}
    </button>
  );
}

export function SectionLabel({ children }: { children: ReactNode }) {
  return (
    <div className="px-3.5 pb-1.5 pt-3 font-mono text-[10.5px] font-medium uppercase tracking-[0.08em] text-gc-ink-4">
      {children}
    </div>
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
        className={`min-w-0 flex-1 bg-transparent text-[13px] text-gc-ink outline-none placeholder:text-gc-ink-4 ${className}`}
        {...rest}
      />
      {trailing && <span className="flex shrink-0 items-center gap-0.5">{trailing}</span>}
    </div>
  );
}

export function ConnPill({
  state = "connected",
}: {
  state?: "connected" | "idle" | "signedout";
}) {
  if (state === "connected") {
    return (
      <span className="inline-flex items-center gap-1.5 rounded-gc-pill bg-[rgba(46,204,113,0.14)] px-2 py-1 text-[11px] font-medium text-[#1f8a4c]">
        <span className="h-1.5 w-1.5 rounded-full bg-gc-success" />
        Connected
      </span>
    );
  }
  const label = state === "idle" ? "Idle" : "Signed out";
  return (
    <span className="inline-flex items-center gap-1.5 rounded-gc-pill bg-gc-sunken px-2 py-1 text-[11px] font-medium text-gc-ink-4">
      <span className="h-1.5 w-1.5 rounded-full bg-gc-ink-5" />
      {label}
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
    <div className="flex items-center gap-1.5 border-b border-gc-line px-2.5 py-2.5">
      <IconButton icon="chevronLeft" size={18} onClick={onBack} aria-label="Back" />
      <span className="text-[14px] font-semibold tracking-[-0.01em] text-gc-ink">
        {title}
      </span>
    </div>
  );
}
