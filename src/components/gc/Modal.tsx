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
  /** On the PRIMARY: filled red rather than filled blue, and focus opens on
   *  the secondary instead.
   *
   *  On the SECONDARY: appearance is unchanged (its class never consults
   *  this) and it only moves initial focus to the primary. That case exists
   *  because `ApplyChangesDialog` draws the destructive action as the
   *  *secondary* - the frame makes "No, I will reopen later" the filled
   *  primary (`130:58448`, `Variant=Default`) and "Yes, close affected apps"
   *  the outline one (`130:58447`, `Variant=Outline`) - which is the one
   *  arrangement the primary-only rule below could not protect. */
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
  warning:
    "border-amber-300 bg-gradient-to-b from-amber-50 to-amber-200 text-amber-700",
  success:
    "border-green-300 bg-gradient-to-b from-green-50 to-green-200 text-green-700",
  // Disconnect and Reset. Red rather than amber: these are not "are you sure",
  // they undo the setup. The 600 icon matches the destructive button fill.
  danger: "border-red-300 bg-gradient-to-b from-red-50 to-red-200 text-red-600",
  neutral: "border-base-border bg-base-card text-neutral-700",
};

/**
 * The widths the file draws. Not a size scale invented here: 480 is the
 * Settings form and confirm dialogs (`143:67735`, `143:70617`), 512 the
 * organization and model confirmations (`130:55314`, `134:61659`, `130:48278`),
 * 536 the quit confirmation (`694:33002`, `694:33340`), 544 the reset dialog
 * alone (`177:74223`), and 600 everything that carries a subject card, a
 * report, a model list or the quit chooser.
 *
 * 536 is the odd one and is drawn rather than derived: both frames sit with
 * their left edge at 255.64, which is exactly where a *512* dialog centred in
 * 1024 would start, and their right edge 24px past centre. That reads like a
 * stretched edge rather than a chosen number, so it is worth a designer
 * question - but the file says 536 and the file wins.
 */
export type ModalWidth = 480 | 512 | 536 | 544 | 600;

const WIDTH_STYLES: Record<ModalWidth, string> = {
  480: "w-[480px]",
  512: "w-[512px]",
  536: "w-[536px]",
  544: "w-[544px]",
  600: "w-[600px]",
};

/**
 * The tone tile's geometry, which does **not** follow from the tone.
 *
 * The 2026-08-26 pass read four dialogs and concluded "44px on a toned dialog,
 * 40px on a neutral one". The Settings frames say otherwise: the three 480px
 * dialogs draw a 32px tile with a 16px glyph - `danger` Disconnect
 * (`143:70620`) among them - while the 600px Diagnostics report
 * (`363:9029`) is *neutral* and draws 44px with a 24px glyph. Size tracks the
 * dialog's width, and the glyph does not track the box, so both are named here
 * and each dialog says which it draws.
 *
 * `md` is the 40/20 pair the earlier pass measured. No frame has been shown to
 * draw it since, but it is the default for every undrawn dialog, so changing
 * those is not this pass's business.
 */
const TILE_SIZES = {
  /** 480px Settings dialogs: rename, replace key, disconnect. */
  sm: { box: "size-8 rounded-sm", glyph: 16 as const },
  /** The 536px quit dialogs, which draw the same 32px box on a 20px glyph
   * (`694:33004`). Same box as `sm`, one step up on the glyph, which is the
   * clearest case there is of the glyph not tracking the box. */
  sm20: { box: "size-8 rounded-sm", glyph: 20 as const },
  /** Undrawn dialogs, and what the 2026-08-26 read recorded as "neutral". */
  md: { box: "size-10 rounded-md", glyph: 20 as const },
  /** 600px dialogs, toned or not: the drift review, the diagnostics report. */
  lg: { box: "size-11 rounded-md", glyph: 24 as const },
} as const;

export type ModalTile = keyof typeof TILE_SIZES;

export function Modal({
  tone = "neutral",
  icon,
  title,
  subtitle,
  children,
  secondary,
  middle,
  primary,
  closeButton,
  onDismiss,
  onClose,
  width = 600,
  tile,
  edge = "default",
  initialFocus,
}: {
  tone?: ModalTone;
  icon: IconName;
  title: string;
  /** ReactNode, not a string: the quit chooser's subtitle sets its count in
   * Medium inside an otherwise regular sentence (`694:32278`). */
  subtitle?: ReactNode;
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
  /** Draw a close X in the top-right corner (Figma 139:66759).
   *
   *  Opt-in because most dialogs here end in a button row, and a second way out
   *  would be a second thing to explain. The model picker has no footer - a
   *  selection applies on click - so without this its only exits would be Escape
   *  and a scrim click, neither of which is visible. */
  closeButton?: boolean;
  /** Escape and scrim clicks. Omit to make the dialog unskippable. */
  onDismiss?: () => void;
  /** Which of the file's widths this dialog is drawn at. Defaults to the
   * widest, which is what an undrawn dialog gets. */
  width?: ModalWidth;
  /** Which tone-tile geometry this dialog draws - see `TILE_SIZES`, and set it
   * from the frame rather than inferring it from the tone. Defaults to what the
   * 2026-08-26 read recorded, so every undrawn dialog keeps the size it had. */
  tile?: ModalTile;
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
  const primaryRef = useRef<HTMLButtonElement>(null);
  // The pre-2026-08-30 rule, kept as the default so an undrawn dialog draws
  // exactly what it drew before. Every dialog the file actually draws names its
  // own tile.
  const tileSize = tile ?? (tone === "neutral" ? "md" : "lg");

  // Focus opens on whichever button is NOT the destructive one: otherwise a
  // keyboard user who opened this with Enter destroys something by pressing
  // Enter again.
  //
  // Both directions, because the rule used to cover only the primary. With
  // the destructive action as the SECONDARY - which is how the file draws
  // apply-changes - nothing matched, so the trap fell through to
  // `focusables[0]`, and the secondary renders first. Enter therefore landed
  // on "Yes, close affected apps".
  useFocusTrap(
    panelRef,
    onDismiss,
    primary?.destructive
      ? safeRef
      : secondary?.destructive
        ? primaryRef
        : initialFocus,
  );

  return (
    <div className="absolute inset-0 z-20 flex items-center justify-center bg-neutral-900/40 p-6">
      <div
        ref={panelRef}
        role="dialog"
        aria-modal="true"
        aria-labelledby={titleId}
        className={`${WIDTH_STYLES[width]} max-w-full rounded-2xl border bg-base-card p-6 shadow-base-lg ${
          edge === "danger"
            ? "border-base-destructive/40"
            : "border-base-border"
        }`}
      >
        {closeButton && onDismiss && (
          <button
            type="button"
            onClick={onDismiss}
            aria-label="Close"
            className="float-right -mr-1 -mt-1 flex size-6 items-center justify-center rounded-sm text-base-muted-foreground transition-colors hover:bg-gray-50 hover:text-base-foreground focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-base-primary"
          >
            <Icon name="x" size={24} />
          </button>
        )}
        <div className="flex items-center gap-3">
          <span
            aria-hidden
            className={`flex shrink-0 items-center justify-center border shadow-base-2xs ${TILE_SIZES[tileSize].box} ${TONE_STYLES[tone]}`}
          >
            <Icon name={icon} size={TILE_SIZES[tileSize].glyph} />
          </span>
          <div className="min-w-0 flex-1">
            <h2
              id={titleId}
              className="text-lg font-medium leading-6 tracking-heading text-base-foreground"
            >
              {title}
            </h2>
            {subtitle && (
              <p className="text-sm leading-5 text-base-muted-foreground">
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

        {/* 16px between body blocks, measured on the rename fields
         * (`143:67460` -> `143:67465`) and the reset steps -> checkbox
         * (`177:73952` -> `177:73976`). This was 12px. */}
        {children && <div className="mt-6 flex flex-col gap-4">{children}</div>}

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
                  middle.disabled
                    ? "cursor-not-allowed opacity-45"
                    : "hover:bg-gray-50"
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
                ref={primaryRef}
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

/**
 * The tinted block explaining what the action will actually do.
 *
 * Three tones, all drawn. `muted` is the original gray-50 block the Settings
 * and reset dialogs carry. The quit flow adds two, both on the same bordered
 * 12px box at radius 8: `info` is `blue-ribbon/50` under `blue-ribbon/900` ink
 * for the aside about closing the window (`694:32290`), and `neutral` is
 * `base/background` under full-foreground ink for the confirmation's report of
 * what happened (`694:33020`). The two new ones state facts rather than
 * qualify an action, which is why neither is muted.
 */
export function ModalNote({
  children,
  tone = "muted",
}: {
  children: ReactNode;
  tone?: "muted" | "info" | "neutral";
}) {
  if (tone === "muted") {
    return (
      <div className="rounded-md bg-gray-50 p-4 text-sm leading-5 text-neutral-600">
        {children}
      </div>
    );
  }
  return (
    <div
      className={`rounded-md border border-base-border p-3 text-sm leading-5 ${
        tone === "info"
          ? "bg-blue-ribbon-50 text-blue-ribbon-900"
          : "bg-base-background text-base-foreground"
      }`}
    >
      {children}
    </div>
  );
}

/**
 * One row of a "pick how this happens" choice: a title, the consequence under
 * it, and an optional pill recommending one of them.
 *
 * Distinct from `ModalOption`, which is the organization switcher's row and
 * leads with an initials avatar. This one is drawn by the quit chooser
 * (`694:32280` / `694:32456`): 12px padding at radius 8, and unlike the org
 * picker *both* states carry `shadow/sm` - only the hairline moves, from
 * `base/input` to `base/primary`.
 */
export function ModalChoice({
  title,
  description,
  pill,
  selected,
  onSelect,
}: {
  title: string;
  description: string;
  /** The drawn recommendation ("SAFEST"). Green-200 on green-800 at radius 4 -
   * a third pairing beside the Overview pills' 200/900 and the App table's
   * 100/700, so it is spelled out here rather than folded into `PillTone`. */
  pill?: string;
  selected: boolean;
  onSelect: () => void;
}) {
  return (
    <button
      type="button"
      role="radio"
      aria-checked={selected}
      onClick={onSelect}
      className={`flex w-full items-center gap-3 rounded-md border bg-base-card p-3 text-left shadow-base-sm transition-colors focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-base-primary ${
        selected ? "border-base-primary" : "border-base-input hover:bg-gray-50"
      }`}
    >
      <span className="min-w-0 flex-1">
        <span className="block text-sm font-medium leading-5 text-base-foreground">
          {title}
        </span>
        <span className="block text-base-xs font-medium leading-4 text-base-muted-foreground">
          {description}
        </span>
      </span>
      {pill && (
        <span className="shrink-0 rounded-control bg-green-200 px-2 py-1 font-mono text-base-xs font-medium uppercase leading-4 tracking-label text-green-800">
          {pill}
        </span>
      )}
    </button>
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
        <span className="block truncate text-sm leading-5 text-neutral-600">
          {meta}
        </span>
      </span>
      {selected ? (
        <Icon
          name="circleCheck"
          size={16}
          className="shrink-0 text-base-primary"
        />
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
  maxLength,
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
  /** Stops a paste the backend would only truncate. */
  maxLength?: number;
}) {
  const id = useId();
  return (
    <div className="flex flex-col gap-2">
      <label
        htmlFor={id}
        className="text-base-xs font-medium leading-4 text-base-foreground"
      >
        {label}
      </label>
      <div className="relative">
        <input
          id={id}
          ref={inputRef}
          value={value}
          readOnly={readOnly}
          placeholder={placeholder}
          maxLength={maxLength}
          onChange={(e) => onChange?.(e.target.value)}
          className={`h-9 w-full rounded-sm border pl-3 text-sm placeholder:text-base-muted-foreground focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-base-primary ${
            readOnly
              ? // Drawn with no fill and no shadow at 60%: it is the value being
                // replaced, not a field. `143:67746`.
                "border-base-input bg-transparent text-base-foreground opacity-60"
              : "border-base-input bg-base-background text-base-foreground shadow-base-xs"
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
      <p className="text-sm font-medium leading-5 text-base-foreground">
        {label}
      </p>
      <ol className="flex flex-col gap-2">
        {steps.map((step, i) => (
          <li
            key={step.title}
            className="flex items-start gap-3 rounded-md border border-base-border bg-gray-50 p-3"
          >
            <span
              aria-hidden
              // 36px, as `177:73957` draws it - the text group starts at 48,
              // which is the tile plus this row's 12px gap.
              className="flex size-9 shrink-0 items-center justify-center rounded-sm border border-base-border bg-base-card text-base font-medium leading-6 text-base-foreground"
            >
              {i + 1}
            </span>
            <span className="min-w-0 flex-1">
              <span className="block text-sm font-medium leading-5 text-base-foreground">
                {step.title}
              </span>
              <span className="block text-base-xs leading-4 text-base-muted-foreground">
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
