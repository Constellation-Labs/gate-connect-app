import { useId, useRef } from "react";
import type { ReactNode } from "react";
import { useFocusTrap } from "../../lib/useFocusTrap";
import { Icon } from "./Icon";
import type { IconName } from "./Icon";

/**
 * The dialog template every modal in the new UI is drawn from (Figma: switch
 * organization, organization switched, review config, apply changes, close
 * affected apps, change ready, and the Gate-model confirmation). All six share
 * one shape, so this is one component with slots rather than six near-copies:
 *
 *   tone tile + title + subtitle
 *   optional subject card (icon, name, description, status pill)
 *   optional note block
 *   right-aligned secondary / primary buttons
 *
 * Centred over a scrim at 600px. These are real dialogs, not the popover's
 * full-panel takeovers - that grammar goes away with the popover.
 */

export type ModalTone = "warning" | "success" | "neutral";

export interface ModalButton {
  label: string;
  onClick: () => void;
  /** Filled red rather than filled blue. */
  destructive?: boolean;
}

const TONE_STYLES: Record<ModalTone, string> = {
  warning: "bg-amber-100 text-amber-700",
  success: "bg-green-100 text-green-700",
  neutral: "bg-gray-100 text-neutral-700",
};

export function Modal({
  tone = "neutral",
  icon,
  title,
  subtitle,
  children,
  secondary,
  primary,
  onDismiss,
}: {
  tone?: ModalTone;
  icon: IconName;
  title: string;
  subtitle?: string;
  children?: ReactNode;
  secondary?: ModalButton;
  primary?: ModalButton;
  /** Escape and scrim clicks. Omit to make the dialog unskippable. */
  onDismiss?: () => void;
}) {
  const titleId = useId();
  const panelRef = useRef<HTMLDivElement>(null);
  const safeRef = useRef<HTMLButtonElement>(null);

  // When the primary action is destructive, focus opens on the secondary
  // button: otherwise a keyboard user who opened this with Enter destroys
  // something by pressing Enter again.
  useFocusTrap(panelRef, onDismiss, primary?.destructive ? safeRef : undefined);

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
              <p className="mt-1 text-sm leading-5 text-neutral-600">{subtitle}</p>
            )}
          </div>
        </div>

        {children && <div className="mt-4 flex flex-col gap-3">{children}</div>}

        {(secondary || primary) && (
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
            {primary && (
              <button
                type="button"
                onClick={primary.onClick}
                className={`flex h-9 items-center rounded-base px-4 text-sm font-medium text-white transition-colors focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 ${
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
  pill,
}: {
  /** 16px mark, brand or glyph. */
  icon: ReactNode;
  title: string;
  description?: string;
  pill?: { label: string; tone: PillTone };
}) {
  return (
    <div className="flex items-center gap-3 rounded-lg border border-base-border p-3">
      <span
        aria-hidden
        className="flex size-10 shrink-0 items-center justify-center rounded-base border border-base-border text-neutral-700"
      >
        {icon}
      </span>
      <div className="min-w-0 flex-1">
        <p className="truncate text-sm font-medium leading-5 text-neutral-900">{title}</p>
        {description && (
          <p className="truncate text-sm leading-5 text-neutral-600">{description}</p>
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
