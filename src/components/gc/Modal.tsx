import { useId, useRef } from "react";
import type { ReactNode, RefObject } from "react";
import { useFocusTrap } from "../../lib/useFocusTrap";
import { Icon } from "./Icon";
import type { IconName } from "./Icon";

/**
 * The dialog template every modal in the new UI is drawn from. One component
 * with slots rather than a dozen near-copies:
 *
 *   tone tile + title + subtitle
 *   any of: a subject card, a note block, form fields, numbered steps,
 *           an acknowledgement checkbox, or plain body copy
 *   right-aligned secondary / primary buttons
 *
 * Centred over a scrim at 600px. These are real dialogs, not the popover's
 * full-panel takeovers - that grammar goes away with the popover.
 */

export type ModalTone = "warning" | "success" | "danger" | "neutral";

export interface ModalButton {
  label: string;
  onClick: () => void;
  /** Filled red rather than filled blue. */
  destructive?: boolean;
  /** Refused, not hidden: the reset dialog gates its primary behind a
   * checkbox, and a button that vanishes tells the user less than one that
   * stays put and explains itself by staying dim. */
  disabled?: boolean;
}

const TONE_STYLES: Record<ModalTone, string> = {
  warning: "bg-amber-100 text-amber-700",
  success: "bg-green-100 text-green-700",
  // Disconnect and Reset. Red rather than amber: these are not "are you sure",
  // they undo the setup. The 600 icon matches the destructive button fill.
  danger: "bg-red-100 text-red-600",
  neutral: "bg-gray-100 text-neutral-700",
};

export function Modal({
  tone = "neutral",
  icon,
  title,
  subtitle,
  subtitleTone = "muted",
  children,
  secondary,
  middle,
  primary,
  onDismiss,
  initialFocus,
}: {
  tone?: ModalTone;
  icon: IconName;
  title: string;
  subtitle?: string;
  /** The Gate-model dialog states its cost consequence in `base-primary`
   * rather than muted grey, which is the only place the design does this. */
  subtitleTone?: "muted" | "primary";
  children?: ReactNode;
  secondary?: ModalButton;
  /** A third action, between the safe one and the primary, for the rare dialog
   * with three genuinely different outcomes rather than yes/no. The quit dialog
   * is the only one: disconnect-and-quit, quit-anyway and cancel are three
   * different things to do, and collapsing any two of them would hide a
   * consequence. Styled as an outline button so the row still reads
   * safe → middle → primary left to right. */
  middle?: ModalButton;
  primary?: ModalButton;
  /** Escape and scrim clicks. Omit to make the dialog unskippable. */
  onDismiss?: () => void;
  /** Where focus opens. The form dialogs point this at the field being edited;
   * without it the trap takes the first focusable, which on those is the
   * read-only current-value input. Ignored when the primary is destructive,
   * since not destroying something on a stray Enter wins. */
  initialFocus?: RefObject<HTMLElement>;
}) {
  const titleId = useId();
  const panelRef = useRef<HTMLDivElement>(null);
  const safeRef = useRef<HTMLButtonElement>(null);

  // When the primary action is destructive, focus opens on the secondary
  // button: otherwise a keyboard user who opened this with Enter destroys
  // something by pressing Enter again.
  useFocusTrap(
    panelRef,
    onDismiss,
    primary?.destructive ? safeRef : initialFocus,
  );

  return (
    <div className="absolute inset-0 z-20 flex items-center justify-center bg-neutral-900/40 p-6">
      <div
        ref={panelRef}
        role="dialog"
        aria-modal="true"
        aria-labelledby={titleId}
        className="w-[600px] max-w-full rounded-xl bg-base-card p-6 shadow-base-lg"
      >
        <div className="flex items-start gap-4">
          <span
            aria-hidden
            className={`flex size-12 shrink-0 items-center justify-center rounded-lg ${TONE_STYLES[tone]}`}
          >
            <Icon name={icon} size={24} />
          </span>
          <div className="min-w-0 flex-1">
            <h2
              id={titleId}
              className="text-xl font-medium leading-6 tracking-heading text-neutral-900"
            >
              {title}
            </h2>
            {subtitle && (
              <p
                className={`mt-1 text-sm leading-5 ${
                  subtitleTone === "primary" ? "text-base-primary" : "text-neutral-600"
                }`}
              >
                {subtitle}
              </p>
            )}
          </div>
        </div>

        {children && <div className="mt-4 flex flex-col gap-3">{children}</div>}

        {(secondary || middle || primary) && (
          <div className="mt-6 flex justify-end gap-3">
            {secondary && (
              <button
                ref={safeRef}
                type="button"
                onClick={secondary.onClick}
                className="flex h-9 items-center rounded-base border border-base-border bg-base-card px-4 text-sm font-medium text-neutral-900 shadow-base-2xs transition-colors hover:bg-gray-50 focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-base-primary"
              >
                {secondary.label}
              </button>
            )}
            {middle && (
              <button
                type="button"
                onClick={middle.disabled ? undefined : middle.onClick}
                aria-disabled={middle.disabled || undefined}
                className={`flex h-9 items-center rounded-base border border-base-input bg-base-card px-4 text-sm font-medium shadow-base-2xs transition-colors focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 ${
                  middle.disabled ? "cursor-not-allowed opacity-45" : "hover:bg-gray-50"
                } ${
                  middle.destructive
                    ? "text-red-600 focus-visible:outline-red-600"
                    : "text-neutral-900 focus-visible:outline-base-primary"
                }`}
              >
                {middle.label}
              </button>
            )}
            {primary && (
              <button
                type="button"
                onClick={primary.disabled ? undefined : primary.onClick}
                aria-disabled={primary.disabled || undefined}
                className={`flex h-9 items-center rounded-base px-4 text-sm font-medium text-white transition-colors focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 ${
                  primary.disabled ? "cursor-not-allowed opacity-45" : ""
                } ${
                  primary.destructive
                    ? "bg-red-600 hover:bg-red-700 focus-visible:outline-red-600"
                    : "bg-blue-ribbon-700 hover:bg-blue-ribbon-800 focus-visible:outline-base-primary"
                }`}
              >
                {primary.label}
              </button>
            )}
          </div>
        )}
      </div>
    </div>
  );
}

export type PillTone = "amber" | "green" | "neutral";

const PILL_STYLES: Record<PillTone, string> = {
  amber: "bg-amber-100 text-amber-900",
  green: "bg-green-100 text-green-900",
  neutral: "bg-gray-100 text-neutral-700",
};

/**
 * The bordered row naming what the dialog is about - the drifted app, the
 * running process, the model being switched to - with a status pill.
 */
export function ModalSubject({
  icon,
  title,
  description,
  variant = "subject",
  pill,
}: {
  /** 16px mark, brand or glyph. */
  icon: ReactNode;
  title: string;
  description?: string;
  /**
   * `subject` names a thing and describes it: bold name over grey detail, used
   * for the drifted app and the running process. `identity` inverts that for
   * the model row, where the vendor is the quiet label and the mono model id is
   * the thing being named.
   */
  variant?: "subject" | "identity";
  pill?: { label: string; tone: PillTone };
}) {
  const identity = variant === "identity";
  return (
    <div className="flex items-center gap-3 rounded-lg border border-base-border p-3">
      <span
        aria-hidden
        className="flex size-10 shrink-0 items-center justify-center rounded-base border border-base-border text-neutral-700"
      >
        {icon}
      </span>
      <div className="min-w-0 flex-1">
        <p
          className={
            identity
              ? "truncate text-base-2xs leading-4 text-base-muted-foreground"
              : "truncate text-sm font-medium leading-5 text-neutral-900"
          }
        >
          {title}
        </p>
        {description && (
          <p
            className={
              identity
                ? "truncate font-mono text-sm leading-5 text-neutral-900"
                : "truncate text-sm leading-5 text-neutral-600"
            }
          >
            {description}
          </p>
        )}
      </div>
      {pill && (
        <span
          className={`shrink-0 rounded-base px-2 py-0.5 font-mono text-base-xs font-medium uppercase leading-4 tracking-label ${PILL_STYLES[pill.tone]}`}
        >
          {pill.label}
        </span>
      )}
    </div>
  );
}

/** The tinted block explaining what the action will actually do. */
export function ModalNote({ children }: { children: ReactNode }) {
  return (
    <div className="rounded-lg bg-gray-50 p-4 text-sm leading-5 text-neutral-600">
      {children}
    </div>
  );
}

/**
 * A selectable row, used by the organization switcher. Distinct from
 * `ModalSubject`: this one is a radio, and it leads with an initials avatar
 * rather than a product mark.
 */
export function ModalOption({
  initials,
  name,
  meta,
  selected,
  onSelect,
}: {
  initials: string;
  name: string;
  /** Secondary line, e.g. "12 members - Free plan". */
  meta: string;
  selected: boolean;
  onSelect: () => void;
}) {
  return (
    <button
      type="button"
      role="radio"
      aria-checked={selected}
      onClick={onSelect}
      className={`flex w-full items-center gap-3 rounded-lg border p-3 text-left transition-colors focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-base-primary ${
        selected ? "border-base-primary" : "border-base-border hover:bg-gray-50"
      }`}
    >
      <span
        aria-hidden
        className="flex size-10 shrink-0 items-center justify-center rounded-base bg-gray-100 text-sm font-medium text-neutral-700"
      >
        {initials}
      </span>
      <span className="min-w-0 flex-1">
        <span className="block truncate text-sm font-medium leading-5 text-neutral-900">
          {name}
        </span>
        <span className="block truncate text-sm leading-5 text-neutral-600">{meta}</span>
      </span>
      {selected ? (
        <Icon name="circleCheck" size={20} className="shrink-0 text-base-primary" />
      ) : (
        <span
          aria-hidden
          className="size-5 shrink-0 rounded-full border border-base-input"
        />
      )}
    </button>
  );
}

/**
 * A labelled field inside a dialog. The rename and replace-key dialogs each
 * stack two: the current value read-only above the new value being typed, so
 * the user can see what they are replacing rather than trusting a label.
 */
export function ModalField({
  label,
  value,
  onChange,
  readOnly,
  mono,
  placeholder,
  inputRef,
}: {
  label: string;
  value: string;
  /** Omitted for the read-only current-value field. */
  onChange?: (next: string) => void;
  readOnly?: boolean;
  /** Keys and ids render mono; device names do not. */
  mono?: boolean;
  placeholder?: string;
  inputRef?: RefObject<HTMLInputElement>;
}) {
  const id = useId();
  return (
    <div className="flex flex-col gap-1.5">
      <label htmlFor={id} className="text-base-xs font-medium leading-4 text-neutral-900">
        {label}
      </label>
      <div className="relative">
        <input
          id={id}
          ref={inputRef}
          value={value}
          readOnly={readOnly}
          placeholder={placeholder}
          onChange={(e) => onChange?.(e.target.value)}
          className={`h-9 w-full rounded-base border bg-base-card pl-3 text-sm shadow-base-2xs placeholder:text-base-muted-foreground focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-base-primary ${
            readOnly
              ? "border-base-border text-base-muted-foreground"
              : "border-base-input text-neutral-900"
          } ${mono ? "font-mono" : ""} ${onChange && value ? "pr-9" : "pr-3"}`}
        />
        {onChange && value && (
          <button
            type="button"
            onClick={() => onChange("")}
            aria-label={`Clear ${label}`}
            className="absolute inset-y-0 right-2 flex items-center text-base-muted-foreground transition-colors hover:text-neutral-900 focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-base-primary"
          >
            <Icon name="circleX" size={16} />
          </button>
        )}
      </div>
    </div>
  );
}

/**
 * "What happens next:" - the reset dialog spells out its three consequences
 * rather than asserting they exist. Numbered because they happen in order.
 */
export function ModalSteps({
  label,
  steps,
}: {
  label: string;
  steps: { title: string; description: string }[];
}) {
  return (
    <div className="flex flex-col gap-3">
      <p className="text-sm font-medium leading-5 text-neutral-900">{label}</p>
      <ol className="flex flex-col gap-2">
        {steps.map((step, i) => (
          <li
            key={step.title}
            className="flex items-start gap-3 rounded-lg border border-base-border bg-gray-50 p-3"
          >
            <span
              aria-hidden
              className="flex size-8 shrink-0 items-center justify-center rounded-base border border-base-border bg-base-card text-sm font-medium text-neutral-700"
            >
              {i + 1}
            </span>
            <span className="min-w-0 flex-1">
              <span className="block text-sm font-medium leading-5 text-neutral-900">
                {step.title}
              </span>
              <span className="block text-sm leading-5 text-neutral-600">
                {step.description}
              </span>
            </span>
          </li>
        ))}
      </ol>
    </div>
  );
}

/**
 * The acknowledgement that gates a reset. A real checkbox rather than a styled
 * div, so it reports its state to assistive tech and toggles on its label.
 */
export function ModalCheckbox({
  checked,
  onChange,
  label,
}: {
  checked: boolean;
  onChange: (next: boolean) => void;
  label: string;
}) {
  return (
    <label className="flex cursor-pointer items-center gap-2 text-sm leading-5 text-neutral-900">
      <input
        type="checkbox"
        checked={checked}
        onChange={(e) => onChange(e.target.checked)}
        className="size-4 shrink-0 accent-blue-ribbon-700 focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-base-primary"
      />
      {label}
    </label>
  );
}
