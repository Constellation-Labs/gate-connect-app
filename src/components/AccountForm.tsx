import { useEffect, useRef, useState } from "react";
import { saveAccount } from "../lib/api";
import { DEFAULT_GATEWAY_BASE_URL } from "../lib/config";
import { usePlatform, secretStoreName } from "../lib/platform";
import { ErrorBlock } from "./ErrorBlock";

interface Props {
  /** Existing base URL when editing; empty string for first-time sign-in. */
  existingBaseUrl?: string;
  existingHasApiKey?: boolean;
  /** Whether the user can dismiss this form (true for "Edit", false for first-run). */
  dismissible: boolean;
  onDone: () => void;
  onCancel?: () => void;
}

export function AccountForm({
  existingBaseUrl,
  existingHasApiKey,
  dismissible,
  onDone,
  onCancel,
}: Props) {
  const platform = usePlatform();
  const editing = !!existingBaseUrl;
  const keyOptional = editing && !!existingHasApiKey;

  // First-run gets the build-time default (overridable via VITE_GATE_*).
  // Edit mode keeps whatever URL is already saved.
  const [baseUrl, setBaseUrl] = useState(
    existingBaseUrl ?? DEFAULT_GATEWAY_BASE_URL
  );
  const [apiKey, setApiKey] = useState("");
  const [replacing, setReplacing] = useState(!keyOptional);
  const [submitting, setSubmitting] = useState(false);
  const [error, setError] = useState<unknown | null>(null);

  // Component-unmount guard - prevents setState on an unmounted form when
  // the user closes the popover mid-flight on a slow keychain or auth panel
  // resolve. (`saveAccount` can sit on a sudo/osascript prompt for a while.)
  const mounted = useRef(true);
  useEffect(() => { mounted.current = true; return () => { mounted.current = false; }; }, []);


  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape" && dismissible && !submitting) onCancel?.();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [dismissible, onCancel, submitting]);

  const submit = async (e: React.FormEvent) => {
    e.preventDefault();
    setError(null);
    setSubmitting(true);
    try {
      const trimmed = apiKey.trim();
      const keyToSend =
        replacing && trimmed.length > 0
          ? trimmed
          : keyOptional
          ? null
          : trimmed;
      await saveAccount(baseUrl.trim(), keyToSend);
      if (!mounted.current) return;
      onDone();
    } catch (err) {
      if (mounted.current) setError(err);
    } finally {
      if (mounted.current) setSubmitting(false);
    }
  };

  return (
    <form onSubmit={submit} className="flex h-full flex-col">
      <header className="shrink-0 flex items-center gap-2 border-b border-ink-100 px-3.5 py-3">
        {dismissible && (
          <button
            type="button"
            onClick={onCancel}
            disabled={submitting}
            className="rounded-[4px] p-1 text-ink-500 transition-colors hover:bg-ink-100 hover:text-ink-900 disabled:opacity-50"
            aria-label="Back"
            title="Back"
          >
            <svg
              viewBox="0 0 24 24"
              className="h-3.5 w-3.5"
              fill="none"
              stroke="currentColor"
              strokeWidth="2"
              strokeLinecap="round"
              strokeLinejoin="round"
            >
              <path d="M15 18l-6-6 6-6" />
            </svg>
          </button>
        )}
        <div className="min-w-0">
          <div className="truncate text-[13px] font-semibold leading-tight tracking-[-0.01em] text-ink-900">
            {editing ? "Edit Gate AI account" : "Sign in to Gate AI"}
          </div>
          <div className="text-[11px] leading-tight text-ink-500">
            {editing
              ? "Update the credentials Gate Connect uses for every tool."
              : "Enter once. Reused for every tool you connect."}
          </div>
        </div>
      </header>

      <div className="flex-1 overflow-y-auto px-3.5 py-3 space-y-3">
        {editing ? (
          <Field
            label="Gateway base URL"
            hint="HTTPS endpoint of your Gate AI gateway."
          >
            <Input
              type="url"
              required
              value={baseUrl}
              onChange={(e) => setBaseUrl(e.target.value)}
              placeholder="https://gate.constellation.network"
              mono
            />
          </Field>
        ) : (
          // First-run: URL is set at build time. Surface it as a small
          // read-only display so the user knows where they're connecting,
          // with a hint that the menu's "Edit account" can change it later.
          <div>
            <span className="block text-[12px] font-medium text-ink-900">
              Gateway
            </span>
            <div className="mt-2 flex items-center gap-2 rounded-md bg-ink-50 px-3 py-2 font-mono text-[12px] text-ink-900 shadow-[inset_0_0_0_1px_oklch(0.96_0_0)]">
              <span className="truncate">{hostFromUrl(baseUrl)}</span>
            </div>
            <span className="mt-1.5 block text-[11px] leading-snug text-ink-500">
              Change later from the menu → Edit account.
            </span>
          </div>
        )}

        <div>
          <div className="flex items-baseline justify-between">
            <span className="block text-[12px] font-medium text-ink-900">
              Gateway API key
            </span>
            {keyOptional && !replacing && (
              <button
                type="button"
                onClick={() => setReplacing(true)}
                className="text-[11px] text-ink-900 underline decoration-ink-200 decoration-1 underline-offset-2 transition-colors hover:decoration-ink-500"
              >
                Replace
              </button>
            )}
          </div>
          <span className="mt-0.5 block text-[11px] leading-snug text-ink-500">
            Authenticates Gate Connect to your workspace. Sent with every
            request your tools make through the gateway.
          </span>

          {keyOptional && !replacing ? (
            <div className="mt-2 flex items-center gap-2 rounded-md bg-ink-50 px-3 py-2 text-[12px] text-ink-700 shadow-[inset_0_0_0_1px_oklch(0.96_0_0)]">
              <CheckIcon />
              <span>Saved in {secretStoreName(platform)}. We never show it again.</span>
            </div>
          ) : (
            <div className="mt-2">
              <Input
                type="password"
                required={!keyOptional}
                autoFocus
                value={apiKey}
                onChange={(e) => setApiKey(e.target.value)}
                placeholder={keyOptional ? "Enter new key to replace" : "sk-gw-…"}
                mono
                aria-invalid={error !== null || undefined}
                aria-describedby={error !== null ? "account-form-error" : undefined}
              />
              {keyOptional && (
                <button
                  type="button"
                  onClick={() => {
                    setApiKey("");
                    setReplacing(false);
                  }}
                  className="mt-1.5 text-[11px] text-ink-500 transition-colors hover:text-ink-900"
                >
                  Keep existing key
                </button>
              )}
            </div>
          )}
        </div>

        {error !== null && <ErrorBlock error={error} context="sign_in" id="account-form-error" />}

        {!editing && (
          <p className="text-[11px] leading-relaxed text-ink-500">
            Sign in to Anthropic from inside Cowork itself when prompted -
            Cowork handles its own auth, Gate Connect just points it at
            your gateway.
          </p>
        )}
      </div>

      <footer className="shrink-0 flex justify-end gap-2 border-t border-ink-100 px-3.5 py-2.5">
        {dismissible && (
          <button
            type="button"
            onClick={onCancel}
            disabled={submitting}
            className="inline-flex h-8 items-center justify-center rounded-md px-3 text-[12px] text-ink-700 transition-colors hover:text-ink-900 disabled:opacity-50"
          >
            Cancel
          </button>
        )}
        <button
          type="submit"
          disabled={submitting}
          className="inline-flex h-8 items-center justify-center rounded-md bg-ink-900 px-3.5 text-[12px] font-medium tracking-[-0.005em] text-white transition-colors hover:bg-ink-800 disabled:cursor-not-allowed disabled:opacity-50"
        >
          {submitting ? "Saving…" : editing ? "Save" : "Sign in"}
        </button>
      </footer>
    </form>
  );
}

function Field({
  label,
  hint,
  children,
}: {
  label: string;
  hint?: string;
  children: React.ReactNode;
}) {
  return (
    <label className="block">
      <span className="block text-[12px] font-medium text-ink-900">{label}</span>
      {hint && (
        <span className="mt-0.5 block text-[11px] leading-snug text-ink-500">
          {hint}
        </span>
      )}
      <span className="mt-2 block">{children}</span>
    </label>
  );
}

function Input({
  mono,
  className,
  ...rest
}: React.InputHTMLAttributes<HTMLInputElement> & { mono?: boolean }) {
  return (
    <input
      {...rest}
      className={`block w-full rounded-md bg-ink-50 px-3 py-2 text-[13px] leading-tight text-ink-900 placeholder:text-ink-400 shadow-[inset_0_0_0_1px_oklch(0.91_0_0)] outline-none transition-all hover:shadow-[inset_0_0_0_1px_oklch(0.82_0_0)] focus:bg-white focus:shadow-[inset_0_0_0_1px_oklch(0.68_0_0)] ${
        mono ? "font-mono" : "font-sans"
      } ${className ?? ""}`}
    />
  );
}

function hostFromUrl(url: string): string {
  try {
    return new URL(url).host;
  } catch {
    return url;
  }
}

function CheckIcon() {
  return (
    <svg
      viewBox="0 0 24 24"
      className="h-3.5 w-3.5 shrink-0 text-success-600"
      fill="none"
      stroke="currentColor"
      strokeWidth="2"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden
    >
      <path d="M20 6L9 17l-5-5" />
    </svg>
  );
}
