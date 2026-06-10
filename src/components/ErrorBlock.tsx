import { classifyError, type ErrorContext } from "../lib/errors";

interface Props {
  error: unknown;
  context: ErrorContext;
  /** DOM id so form inputs can `aria-describedby` this block. */
  id?: string;
}

/**
 * Shared error surface. Classifies the raw Rust-side error into a
 * user-facing title + hint, then tucks the raw string under a `<details>`
 * for power users / support pasting.
 *
 * The outer container is `role="status"` + `aria-live="polite"` so screen
 * readers announce the failure once without interrupting an in-progress
 * announcement. Pass `id` to wire `aria-describedby` from a form field.
 */
export function ErrorBlock({ error, context, id }: Props) {
  const { title, hint, raw } = classifyError(error, context);
  return (
    <div
      id={id}
      role="status"
      aria-live="polite"
      className="rounded-md bg-danger-50 p-3 text-[12px] leading-relaxed text-danger-800 shadow-[inset_0_0_0_1px_oklch(0.885_0.062_18.334)]"
    >
      <div className="font-medium">{title}</div>
      <p className="mt-1 text-[11px] text-danger-700">{hint}</p>
      <details className="mt-2 group">
        <summary className="cursor-pointer select-none text-[11px] text-danger-700 transition-colors hover:text-danger-800 [&::-webkit-details-marker]:hidden">
          <span className="inline-flex items-center gap-1">
            <Chevron />
            <span className="group-open:hidden">Show details</span>
            <span className="hidden group-open:inline">Hide details</span>
          </span>
        </summary>
        <pre className="mt-1.5 max-h-32 overflow-y-auto whitespace-pre-wrap break-all font-mono text-[10.5px] leading-relaxed text-danger-700/90">
          {raw}
        </pre>
      </details>
    </div>
  );
}

function Chevron() {
  return (
    <svg
      viewBox="0 0 24 24"
      className="h-3 w-3 transition-transform group-open:rotate-90"
      fill="none"
      stroke="currentColor"
      strokeWidth="2"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden
    >
      <path d="M9 18l6-6-6-6" />
    </svg>
  );
}
