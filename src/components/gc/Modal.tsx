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
 * Centred over a scrim at 16px radius. Width is per dialog rather than one
 * house number: the file draws 480, 512, 544 and 600 and each frame means it -
 * see `ModalWidth`. These are real dialogs, not the popover's full-panel
 * takeovers - that grammar goes away with the popover.
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

/**
 * The tone tile. Every drawn dialog gives it a 1px border, `shadow/2xs` and a
 * vertical 50-to-200 gradient rather than the flat 100 fill this used to carry:
 * `130:57444` (warning), `134:61661` (success), `177:79233` (danger). The ink
 * stays as it was - the frames export the tile but not the glyph's own fill.
 */
const TONE_STYLES: Record<ModalTone, string> = {
  warning: "border-amber-300 bg-gradient-to-b from-amber-50 to-amber-200 text-amber-700",
  success: "border-green-300 bg-gradient-to-b from-green-50 to-green-200 text-green-700",
  // Disconnect and Reset. Red rather than amber: these are not "are you sure",
  // they undo the setup. The 600 icon matches the destructive button fill.
  danger: "border-red-300 bg-gradient-to-b from-red-50 to-red-200 text-red-600",
  neutral: "border-base-border bg-base-card text-neutral-700",
};

/**
 * The four widths the file draws. Not a size scale invented here: 480 is the
 * Settings form and confirm dialogs (`143:67735`, `143:70617`), 512 the
 * organization and model confirmations (`130:55314`, `134:61659`, `130:48278`),
 * 544 the reset dialog alone (`177:74223`), and 600 everything that carries a
 * subject card, a report or a model list.
 */
export type ModalWidth = 480 | 512 | 544 | 600;

const WIDTH_STYLES: Record<ModalWidth, string> = {
  480: "w-[480px]",
  512: "w-[512px]",
  544: "w-[544px]",
  600: "w-[600px]",
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
  onClose,
  width = 600,
  edge = "default",
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
  /** Which of the file's four widths this dialog is drawn at. Defaults to the
   * widest, which is what an undrawn dialog gets. */
  width?: ModalWidth;
  /**
   * The card's own border. `danger` is `custom/destructive\40` (#dc262666), a
   * named variable rather than a one-off, which is why the Disconnect dialog
   * drawing it and the Reset dialog not is taken as intent rather than a slip.
   */
  edge?: "default" | "danger";
  /** Draws an X at the header's right edge. Only for the dialogs the design
   * gives one - the model picker is the first - and separate from `onDismiss`
   * because a visible control and an escape hatch are different affordances. */
  onClose?: () => void;
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
        className={`${WIDTH_STYLES[width]} max-w-full rounded-2xl border bg-base-card p-6 shadow-base-lg ${
          edge === "danger" ? "border-base-destructive/40" : "border-base-border"
        }`}
      >
        <div className="flex items-center gap-3">
          {/* 44px on a toned dialog, 40px on a neutral one, which is the size
           * difference the frames draw rather than a rounding of one number. */}
          <span
            aria-hidden
            className={`flex shrink-0 items-center justify-center rounded-md border shadow-base-2xs ${
              tone === "neutral" ? "size-10" : "size-11"
            } ${TONE_STYLES[tone]}`}
          >
            <Icon name={icon} size={tone === "neutral" ? 20 : 24} />
          </span>
          <div className="min-w-0 flex-1">
            <h2
              id={titleId}
              className="text-lg font-medium leading-6 tracking-heading text-base-foreground"
            >
              {title}
            </h2>
            {subtitle && (
              <p
                className={`text-sm leading-5 ${
                  subtitleTone === "primary"
                    ? "text-base-primary"
                    : "text-base-muted-foreground"
                }`}
              >
                {subtitle}
              </p>
            )}
          </div>
          {onClose && (
            <button
              type="button"
              aria-label="Close"
              onClick={onClose}
              className="shrink-0 rounded-sm p-1 text-neutral-500 transition-colors hover:text-base-foreground focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-base-primary"
            >
              <Icon name="x" size={16} />
            </button>
          )}
        </div>

        {children && <div className="mt-6 flex flex-col gap-3">{children}</div>}

        {(secondary || middle || primary) && (
          <div className="mt-6 flex justify-end gap-3">
            {secondary && (
              <button
                ref={safeRef}
                type="button"
                onClick={secondary.onClick}
                className="flex h-9 items-center gap-2 rounded-md border border-base-input bg-base-card px-3 text-sm font-medium tracking-button-sm text-base-primary shadow-base-btn transition-colors hover:bg-gray-50 focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-base-primary"
              >
                {secondary.label}
              </button>
            )}
            {middle && (
              <button
                type="button"
                onClick={middle.disabled ? undefined : middle.onClick}
                aria-disabled={middle.disabled || undefined}
                className={`flex h-9 items-center gap-2 rounded-md border border-base-input bg-base-card px-3 text-sm font-medium tracking-button-sm shadow-base-btn transition-colors focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 ${
                  middle.disabled ? "cursor-not-allowed opacity-45" : "hover:bg-gray-50"
                } ${
                  middle.destructive
                    ? "text-red-600 focus-visible:outline-red-600"
                    : "text-base-foreground focus-visible:outline-base-primary"
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
                className={`flex h-9 items-center gap-2 rounded-md px-3 text-sm font-medium tracking-button-sm transition-colors focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 ${
                  primary.disabled ? "cursor-not-allowed opacity-45" : ""
                } ${
                  primary.destructive
                    ? "bg-base-destructive text-base-destructive-foreground shadow-base-btn-destructive hover:bg-red-700 focus-visible:outline-red-600"
                    : "border border-white/20 bg-base-primary bg-gradient-to-b from-white/[0.08] to-black/[0.08] text-base-primary-foreground shadow-base-btn-primary hover:bg-blue-ribbon-800 focus-visible:outline-base-primary"
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
    <div className="flex items-center gap-3 rounded-md border border-base-border p-3">
      <span
        aria-hidden
        className="flex size-10 shrink-0 items-center justify-center rounded-sm border border-base-border text-neutral-700"
      >
        {icon}
      </span>
      <div className="min-w-0 flex-1">
        <p
          className={
            identity
              ? "truncate text-base-2xs leading-4 text-base-muted-foreground"
              : "truncate text-sm font-medium leading-5 text-base-foreground"
          }
        >
          {title}
        </p>
        {description && (
          <p
            className={
              identity
                ? "truncate font-mono text-sm leading-5 text-base-foreground"
                : "truncate text-sm leading-5 text-neutral-600"
            }
          >
            {description}
          </p>
        )}
      </div>
      {pill && (
        <span
          className={`shrink-0 rounded-sm px-2 py-0.5 font-mono text-base-xs font-medium uppercase leading-4 tracking-label ${PILL_STYLES[pill.tone]}`}
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
    <div className="rounded-md bg-gray-50 p-4 text-sm leading-5 text-neutral-600">
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
      // 8px padding, and the two states carry different elevation: selected is
      // `shadow/sm` on a `base/primary` hairline, unselected `shadow/xs` on
      // `base/input` (`Flows / Setup` org picker, 451:7795, read 2026-08-26).
      className={`flex w-full items-center gap-3 rounded-md border p-2 text-left transition-colors focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-base-primary ${
        selected
          ? "border-base-primary shadow-base-sm"
          : "border-base-input shadow-base-xs hover:bg-gray-50"
      }`}
    >
      <span
        aria-hidden
        className="flex size-9 shrink-0 items-center justify-center rounded-sm bg-gray-100 text-sm font-medium text-neutral-700"
      >
        {initials}
      </span>
      <span className="min-w-0 flex-1">
        <span className="block truncate text-sm font-medium leading-5 text-base-foreground">
          {name}
        </span>
        <span className="block truncate text-sm leading-5 text-neutral-600">{meta}</span>
      </span>
      {selected ? (
        <Icon name="circleCheck" size={16} className="shrink-0 text-base-primary" />
      ) : (
        <span
          aria-hidden
          className="size-4 shrink-0 rounded-full border border-base-input bg-base-background shadow-base-xs"
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
    <div className="flex flex-col gap-2">
      <label htmlFor={id} className="text-base-xs font-medium leading-4 text-base-foreground">
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
          className={`h-9 w-full rounded-sm border pl-3 text-sm placeholder:text-base-muted-foreground focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-base-primary ${
            readOnly
              ? // Drawn with no fill and no shadow at 60%: it is the value being
                // replaced, not a field. `143:67746`.
                "border-base-input bg-transparent text-base-foreground opacity-60"
              : "border-base-input bg-base-card text-base-foreground shadow-base-xs"
          } ${mono ? "font-mono" : ""} ${onChange && value ? "pr-9" : "pr-3"}`}
        />
        {onChange && value && (
          <button
            type="button"
            onClick={() => onChange("")}
            aria-label={`Clear ${label}`}
            className="absolute inset-y-0 right-2 flex items-center text-base-muted-foreground transition-colors hover:text-base-foreground focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-base-primary"
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
      <p className="text-sm font-medium leading-5 text-base-foreground">{label}</p>
      <ol className="flex flex-col gap-2">
        {steps.map((step, i) => (
          <li
            key={step.title}
            className="flex items-start gap-3 rounded-md border border-base-border bg-gray-50 p-3"
          >
            <span
              aria-hidden
              className="flex size-8 shrink-0 items-center justify-center rounded-sm border border-base-border bg-base-card text-sm font-medium text-neutral-700"
            >
              {i + 1}
            </span>
            <span className="min-w-0 flex-1">
              <span className="block text-sm font-medium leading-5 text-base-foreground">
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
    <label className="flex cursor-pointer items-center gap-2 text-sm leading-5 text-base-foreground">
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
