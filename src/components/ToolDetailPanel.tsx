import { useEffect, useRef, useState } from "react";
import type { Tool } from "../lib/api";
import {
  clearUpstreamCredential,
  connectTool,
  detectClaudeCodeSession,
  disconnectTool,
  hasUpstreamCredential,
  saveUpstreamApiKey,
  saveUpstreamViaClaudeOauth,
} from "../lib/api";
import { usePlatform, secretStoreName } from "../lib/platform";
import { ErrorBlock } from "./ErrorBlock";

interface Props {
  tool: Tool;
  gatewayHost: string;
  onClose: () => void;
  onChanged: () => void;
  onMigrate: () => void;
}

type CredSource = "api_key" | "claude_oauth";

/**
 * One panel for every tool-scoped action. Branches on `tool.status` so
 * it can host the initial Connect flow AND the post-connect management
 * UI (Disconnect, credential controls, Migrate from standard mode).
 *
 * Light cg-system surfaces: white panel, ink-50 callouts, ink-100
 * dividers, shadow-as-border, ink-900 primary button.
 */
export function ToolDetailPanel({ tool, gatewayHost, onClose, onChanged, onMigrate }: Props) {
  const connected = tool.status.kind === "connected" || tool.status.kind === "drifted";

  if (connected) {
    return (
      <ConnectedView
        tool={tool}
        gatewayHost={gatewayHost}
        onClose={onClose}
        onChanged={onChanged}
        onMigrate={onMigrate}
      />
    );
  }
  return <ConnectView tool={tool} gatewayHost={gatewayHost} onClose={onClose} onConnected={onChanged} />;
}

// ---------- connect (not-yet-connected) ----------

interface ConnectViewProps {
  tool: Tool;
  gatewayHost: string;
  onClose: () => void;
  onConnected: () => void;
}

function ConnectView({ tool, gatewayHost, onClose, onConnected }: ConnectViewProps) {
  const [upstreamUrl, setUpstreamUrl] = useState(tool.default_upstream_url);
  const [showAdvanced, setShowAdvanced] = useState(false);

  const platform = usePlatform();
  const needsCred = tool.requires_upstream_credential;
  // Claude OAuth delegation only makes sense for Cowork (it reads the
  // active Claude Code session's token from keychain at request time).
  // Other tools that need an upstream key always go through API-key paste.
  const supportsClaudeOauth = tool.supports_claude_oauth_delegation;
  const [hasCred, setHasCred] = useState<boolean | null>(needsCred ? null : true);
  const [replacing, setReplacing] = useState(false);
  const [source, setSource] = useState<CredSource>(supportsClaudeOauth ? "claude_oauth" : "api_key");
  const [apiKey, setApiKey] = useState("");
  // null = detection in flight (or N/A for non-delegation tools).
  const [loggedIn, setLoggedIn] = useState<boolean | null>(supportsClaudeOauth ? null : false);
  const sourceTouched = useRef(false);

  const [phase, setPhase] = useState<"idle" | "oauth" | "connecting">("idle");
  const [error, setError] = useState<{ err: unknown; ctx: "connect" | "forget" } | null>(null);
  const [validationError, setValidationError] = useState<string | null>(null);

  // Unmount guard — prevents setState on this form after the user closes
  // the popover mid-connect (osascript prompt, OAuth browser, slow keychain).
  const mounted = useRef(true);
  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
    };
  }, []);

  useEffect(() => {
    if (!needsCred) return;
    let cancelled = false;
    hasUpstreamCredential(tool.slug)
      .then((v) => {
        if (!cancelled) setHasCred(v);
      })
      .catch(() => {
        if (!cancelled) setHasCred(false);
      });
    return () => {
      cancelled = true;
    };
  }, [tool.slug, needsCred]);

  // Detect a live Claude Code session so we can recommend / pre-select the
  // delegation source and show accurate copy. If Claude Code isn't signed in
  // and the user hasn't picked a source yet, fall back to the paste path
  // rather than leaving a selection that would fail at connect time.
  useEffect(() => {
    if (!supportsClaudeOauth) return;
    let cancelled = false;
    detectClaudeCodeSession()
      .then((v) => {
        if (cancelled) return;
        setLoggedIn(v);
        if (!v && !sourceTouched.current) setSource("api_key");
      })
      .catch(() => {
        if (!cancelled) setLoggedIn(false);
      });
    return () => {
      cancelled = true;
    };
  }, [supportsClaudeOauth]);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape" && phase === "idle") onClose();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose, phase]);

  const useExisting = hasCred === true && !replacing;
  const submitting = phase !== "idle";

  const submit = async (e: React.FormEvent) => {
    e.preventDefault();
    setError(null);

    try {
      if (needsCred && !useExisting) {
        if (source === "api_key") {
          const trimmed = apiKey.trim();
          if (!trimmed) {
            setValidationError("Paste an API key, or switch to Claude subscription.");
            return;
          }
          setPhase("connecting");
          await saveUpstreamApiKey(tool.slug, trimmed);
        } else {
          setPhase("oauth");
          await saveUpstreamViaClaudeOauth(tool.slug);
        }
      }
      setPhase("connecting");
      await connectTool(tool.slug, upstreamUrl.trim());
      if (!mounted.current) return;
      onConnected();
    } catch (err) {
      if (mounted.current) setError({ err, ctx: "connect" });
    } finally {
      if (mounted.current) setPhase("idle");
    }
  };

  const onForget = async () => {
    setError(null);
    try {
      await clearUpstreamCredential(tool.slug);
      if (!mounted.current) return;
      setHasCred(false);
      setReplacing(false);
      setApiKey("");
    } catch (err) {
      if (mounted.current) setError({ err, ctx: "forget" });
    }
  };

  return (
    <form onSubmit={submit} className="absolute inset-0 z-10 flex flex-col bg-white">
      <PanelHeader
        title={`Connect ${tool.name}`}
        subtitle="Route via Gate AI"
        onClose={onClose}
        disabled={submitting}
      />

      <div className="flex-1 overflow-y-auto px-3.5 py-3 space-y-3">
        <Callout>
          <p>
            {tool.name} will send {tool.upstream_provider_name} requests through <CodeChip>{gatewayHost}</CodeChip>.
          </p>
        </Callout>

        {needsCred ? (
          <CredentialBlock
            toolSlug={tool.slug}
            toolName={tool.name}
            provider={tool.upstream_provider_name}
            supportsClaudeOauth={supportsClaudeOauth}
            loggedIn={loggedIn}
            state={hasCred === null ? "loading" : useExisting ? "existing" : "new"}
            source={source}
            apiKey={apiKey}
            phase={phase}
            hasValidationError={validationError !== null}
            onSourceChange={(s) => {
              sourceTouched.current = true;
              setSource(s);
            }}
            onApiKeyChange={setApiKey}
            onReplace={() => {
              setReplacing(true);
              setApiKey("");
            }}
            onForget={onForget}
          />
        ) : (
          <Callout>
            <p>
              {tool.name} keeps its existing {tool.upstream_provider_name} credentials. Gate forwards them upstream — no
              separate API key needed here.
            </p>
          </Callout>
        )}

        <div>
          <button
            type="button"
            onClick={() => setShowAdvanced((v) => !v)}
            className="flex items-center gap-1 text-[11px] text-ink-500 transition-colors hover:text-ink-900"
          >
            <Chevron rotated={showAdvanced} />
            Advanced
          </button>
          {showAdvanced && (
            <div className="mt-2">
              <Field
                label="Upstream URL"
                hint="Where Gate routes the request. Defaults to the provider's canonical endpoint."
              >
                <Input type="url" required value={upstreamUrl} onChange={(e) => setUpstreamUrl(e.target.value)} mono />
              </Field>
            </div>
          )}
        </div>

        {validationError && (
          <div
            id="api-key-validation"
            role="status"
            aria-live="polite"
            className="rounded-md bg-warning-50 px-3 py-2 text-[11.5px] text-warning-800 shadow-[inset_0_0_0_1px_oklch(0.924_0.12_95.746)]"
          >
            {validationError}
          </div>
        )}
        {error && <ErrorBlock error={error.err} context={error.ctx} id="connect-form-error" />}

        <p className="!mt-4 text-[11px] leading-relaxed text-ink-500">
          {needsCred ? (
            platform === "windows" ? (
              <>
                Gate Connect writes {tool.name}'s gateway settings to the Windows registry at{" "}
                <CodeChip>HKCU\Software\Policies\Claude</CodeChip> — no password prompt. Your keys stay in{" "}
                {secretStoreName(platform)} and a small helper hands them to {tool.name} at request time.{" "}
                <TermDetails>On Windows this is a registry policy Claude Desktop reads at launch.</TermDetails>
              </>
            ) : (
              <>
                {tool.slug === "cowork"
                  ? "macOS will ask for your password once — Gate Connect writes a system-level settings file at"
                  : "Gate Connect updates"}{" "}
                <CodeChip>
                  {tool.slug === "cowork"
                    ? "/Library/Managed Preferences/$USER/com.anthropic.claudefordesktop.plist"
                    : tool.slug === "codex"
                      ? "~/.codex/config.toml"
                      : tool.slug === "opencode"
                        ? "~/.config/opencode/opencode.json"
                        : "~/.claude/settings.json"}
                </CodeChip>{" "}
                so {tool.name} routes through your gateway.{" "}
                <TermDetails>
                  {tool.slug === "cowork"
                    ? "In macOS terms this is a managed-preferences plist."
                    : "This is the tool's own config file in your home folder."}
                </TermDetails>
              </>
            )
          ) : (
            <>
              Gate Connect updates{" "}
              <CodeChip>
                {tool.slug === "codex"
                  ? "~/.codex/config.toml"
                  : tool.slug === "opencode"
                    ? "~/.config/opencode/opencode.json"
                    : "~/.claude/settings.json"}
              </CodeChip>{" "}
              in your home folder — no password prompt.
            </>
          )}
        </p>
      </div>

      <footer className="shrink-0 flex justify-end gap-2 border-t border-ink-100 px-3.5 py-2.5">
        <GhostButton type="button" onClick={onClose} disabled={submitting}>
          Cancel
        </GhostButton>
        <PrimaryButton type="submit" disabled={submitting || hasCred === null}>
          {phase === "oauth" ? "Waiting for browser…" : phase === "connecting" ? "Connecting…" : "Connect"}
        </PrimaryButton>
      </footer>
    </form>
  );
}

// ---------- connected (post-connect management) ----------

interface ConnectedViewProps {
  tool: Tool;
  gatewayHost: string;
  onClose: () => void;
  onChanged: () => void;
  onMigrate: () => void;
}

function ConnectedView({ tool, gatewayHost, onClose, onChanged, onMigrate }: ConnectedViewProps) {
  const [busy, setBusy] = useState<"disconnecting" | "forgetting" | null>(null);
  const [error, setError] = useState<{ err: unknown; ctx: "disconnect" | "forget" } | null>(null);

  // Unmount guard — same rationale as ConnectView.
  const mounted = useRef(true);
  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
    };
  }, []);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape" && !busy) onClose();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose, busy]);

  const onDisconnect = async () => {
    setBusy("disconnecting");
    setError(null);
    try {
      await disconnectTool(tool.slug);
      if (!mounted.current) return;
      onChanged();
    } catch (err) {
      if (mounted.current) setError({ err, ctx: "disconnect" });
    } finally {
      if (mounted.current) setBusy(null);
    }
  };

  const onForget = async () => {
    setBusy("forgetting");
    setError(null);
    try {
      await clearUpstreamCredential(tool.slug);
      if (!mounted.current) return;
      onChanged();
    } catch (err) {
      if (mounted.current) setError({ err, ctx: "forget" });
    } finally {
      if (mounted.current) setBusy(null);
    }
  };

  const drifted = tool.status.kind === "drifted";
  const driftReason = tool.status.kind === "drifted" ? tool.status.reason : null;

  // Migration is Cowork-specific today. Hide the section for any other tool.
  const supportsMigrate = tool.supports_migrate;

  return (
    <div className="absolute inset-0 z-10 flex flex-col bg-white">
      <PanelHeader
        title={`Manage ${tool.name}`}
        subtitle={drifted ? "Out of sync" : "Routing via Gate AI"}
        onClose={onClose}
        disabled={!!busy}
      />

      <div className="flex-1 overflow-y-auto px-3.5 py-3 space-y-3">
        {drifted ? (
          <div className="rounded-md bg-warning-50 p-3 text-[12px] leading-relaxed text-warning-800 shadow-[inset_0_0_0_1px_oklch(0.924_0.12_95.746)]">
            <div className="font-medium">Out of sync</div>
            <p className="mt-1 text-[11px] text-warning-700">
              {driftReason ??
                "Your Mac's settings for this tool changed somewhere else. Click Disconnect, then connect again to fix."}
            </p>
          </div>
        ) : (
          <Callout>
            <div className="flex items-center gap-2 text-[12px] text-ink-900">
              <CheckIcon />
              <span className="font-medium">Connected via Gate AI</span>
            </div>
            <p className="mt-1 text-[11px] text-ink-500">
              {tool.name} sends {tool.upstream_provider_name} requests through <CodeChip>{gatewayHost}</CodeChip>.
            </p>
          </Callout>
        )}

        {tool.requires_upstream_credential && (
          <CredentialSummary
            toolSlug={tool.slug}
            provider={tool.upstream_provider_name}
            onForget={onForget}
            busy={busy === "forgetting"}
          />
        )}

        {supportsMigrate && (
          <section>
            <Eyebrow>Actions</Eyebrow>
            <button
              type="button"
              onClick={onMigrate}
              className="group mt-2 block w-full rounded-md bg-white p-3 text-left shadow-border transition-all hover:bg-ink-50 hover:shadow-border-hover"
            >
              <div className="flex items-start gap-2">
                <div className="flex-1 min-w-0">
                  <div className="text-[12px] font-medium text-ink-900">Bring over your existing data</div>
                  <p className="mt-1 text-[11px] leading-relaxed text-ink-500">
                    Copy your scheduled tasks, conversations, plugins, and memory from a previous Cowork install into
                    the new gateway-mode setup. One-shot, runs locally.
                  </p>
                </div>
                <div className="mt-0.5 shrink-0 text-ink-400 group-hover:text-ink-700">
                  <Chevron rotated={false} />
                </div>
              </div>
            </button>
          </section>
        )}

        {error && <ErrorBlock error={error.err} context={error.ctx} id="manage-form-error" />}
      </div>

      <footer className="shrink-0 flex justify-end gap-2 border-t border-ink-100 px-3.5 py-2.5">
        <GhostButton type="button" onClick={onClose} disabled={!!busy}>
          Close
        </GhostButton>
        <DangerButton type="button" onClick={onDisconnect} disabled={!!busy}>
          {busy === "disconnecting" ? "Disconnecting…" : "Disconnect"}
        </DangerButton>
      </footer>
    </div>
  );
}

// ---------- shared subcomponents ----------

interface PanelHeaderProps {
  title: string;
  subtitle: string;
  onClose: () => void;
  disabled?: boolean;
  /** Move keyboard focus to the back button on mount. Set false when an
      input below will autoFocus (so we don't fight over focus). */
  autoFocus?: boolean;
}

function PanelHeader({ title, subtitle, onClose, disabled, autoFocus = true }: PanelHeaderProps) {
  const backRef = useRef<HTMLButtonElement | null>(null);
  useEffect(() => {
    if (autoFocus) backRef.current?.focus();
  }, [autoFocus]);
  return (
    <header className="shrink-0 flex items-center gap-2 border-b border-ink-100 px-3.5 py-3">
      <button
        ref={backRef}
        type="button"
        onClick={onClose}
        disabled={disabled}
        className="rounded-[4px] p-1 text-ink-500 transition-colors hover:bg-ink-100 hover:text-ink-900 disabled:opacity-50"
        aria-label="Back"
        title="Back to tools"
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
      <div className="min-w-0">
        <div className="truncate text-[13px] font-semibold leading-tight tracking-[-0.01em] text-ink-900">{title}</div>
        <div className="text-[11px] leading-tight text-ink-500">{subtitle}</div>
      </div>
    </header>
  );
}

function CredentialSummary({
  toolSlug,
  provider,
  onForget,
  busy,
}: {
  toolSlug: string;
  provider: string;
  onForget: () => void;
  busy: boolean;
}) {
  const platform = usePlatform();
  const storage =
    toolSlug === "opencode"
      ? "Stored in OpenCode's credential store (~/.local/share/opencode/auth.json)."
      : `Stored in ${secretStoreName(platform)}.`;
  return (
    <Callout>
      <div className="flex items-center gap-2 text-[12px] text-ink-900">
        <CheckIcon />
        <span className="font-medium">{provider} credential saved</span>
      </div>
      <p className="mt-1 text-[11px] text-ink-500">{storage} Used for every request via this tool.</p>
      <div className="mt-2 flex gap-3 text-[11px]">
        <button
          type="button"
          onClick={onForget}
          disabled={busy}
          className="text-ink-700 underline decoration-ink-200 decoration-1 underline-offset-2 transition-colors hover:decoration-ink-500 disabled:opacity-50"
        >
          {busy ? "Forgetting…" : "Forget credential"}
        </button>
      </div>
    </Callout>
  );
}

interface CredentialBlockProps {
  toolSlug: string;
  toolName: string;
  provider: string;
  supportsClaudeOauth: boolean;
  loggedIn: boolean | null;
  state: "loading" | "existing" | "new";
  source: CredSource;
  apiKey: string;
  phase: "idle" | "oauth" | "connecting";
  hasValidationError: boolean;
  onSourceChange: (s: CredSource) => void;
  onApiKeyChange: (v: string) => void;
  onReplace: () => void;
  onForget: () => void;
}

function CredentialBlock({
  toolSlug,
  toolName,
  provider,
  supportsClaudeOauth,
  loggedIn,
  state,
  source,
  apiKey,
  phase,
  hasValidationError,
  onSourceChange,
  onApiKeyChange,
  onReplace,
  onForget,
}: CredentialBlockProps) {
  const platform = usePlatform();
  if (state === "loading") {
    return (
      <Callout>
        <p className="text-[11px] text-ink-500">Checking saved credential…</p>
      </Callout>
    );
  }

  if (state === "existing") {
    return (
      <Callout>
        <div className="flex items-center gap-2 text-[12px] text-ink-900">
          <CheckIcon />
          <span className="font-medium">{provider} credential saved</span>
        </div>
        <p className="mt-1 text-[11px] text-ink-500">
          {toolSlug === "opencode"
            ? "Stored in OpenCode's credential store. Reused for every connect."
            : `Stored in ${secretStoreName(platform)}. Reused for every connect.`}
        </p>
        <div className="mt-2 flex gap-4 text-[11px]">
          <button
            type="button"
            onClick={onReplace}
            className="text-ink-900 underline decoration-ink-200 decoration-1 underline-offset-2 transition-colors hover:decoration-ink-500"
          >
            Replace
          </button>
          <button
            type="button"
            onClick={onForget}
            className="text-ink-700 underline decoration-ink-200 decoration-1 underline-offset-2 transition-colors hover:decoration-ink-500"
          >
            Forget
          </button>
        </div>
      </Callout>
    );
  }

  return (
    <div>
      <Field
        label={`${provider} credential`}
        hint={`Used as the upstream Bearer when ${toolName} makes a request — Gate forwards it to the upstream provider.`}
      >
        {supportsClaudeOauth && (
          <div className="mb-2 inline-flex gap-0.5 rounded-md bg-ink-100 p-0.5">
            <SourceTab active={source === "claude_oauth"} onClick={() => onSourceChange("claude_oauth")}>
              Claude subscription{loggedIn ? " · Recommended" : ""}
            </SourceTab>
            <SourceTab active={source === "api_key"} onClick={() => onSourceChange("api_key")}>
              API key
            </SourceTab>
          </div>
        )}

        {supportsClaudeOauth && source === "claude_oauth" ? (
          <Callout>
            {phase === "oauth" ? (
              <p className="text-[11px] leading-relaxed text-ink-700">
                Linking Claude Code session.
                {platform === "macos" ? " macOS may ask once to allow Gate Connect access — choose Always Allow." : ""}
              </p>
            ) : (
              <>
                <p className="text-[11px] leading-relaxed text-ink-700">
                  Delegates to your live Claude Code session — the helper reads Claude Code's current access token from{" "}
                  {platform === "windows" ? "its credentials file" : "the keychain"} on every request, so refresh is
                  handled by Claude Code itself.
                </p>
                {loggedIn === false ? (
                  <p className="mt-1.5 text-[11px] text-ink-500">
                    Claude Code isn't signed in. Run <CodeChip>claude</CodeChip>, then <CodeChip>/login</CodeChip> — or
                    paste a token instead.
                  </p>
                ) : (
                  <p className="mt-1.5 text-[11px] text-ink-500">
                    Requires Claude Code to be signed in (run <CodeChip>claude</CodeChip>, then{" "}
                    <CodeChip>/login</CodeChip>).
                  </p>
                )}
              </>
            )}
          </Callout>
        ) : (
          <>
            <Input
              type="password"
              required
              autoFocus
              value={apiKey}
              onChange={(e) => onApiKeyChange(e.target.value)}
              placeholder={toolSlug === "opencode" ? "sk-…" : "sk-ant-api03-…"}
              mono
              aria-invalid={hasValidationError || undefined}
              aria-describedby={hasValidationError ? "api-key-validation" : undefined}
            />
            <p className="mt-1 text-[11px] text-ink-500">
              {toolSlug === "opencode"
                ? "Whatever API key matches the upstream URL you picked above. Saved into OpenCode's own credential store."
                : supportsClaudeOauth
                  ? "An Anthropic API key, or a token from `claude setup-token` (sk-ant-oat…) — a stable credential that survives Claude Code logout."
                  : "Create one at console.anthropic.com."}
            </p>
          </>
        )}
      </Field>
    </div>
  );
}

function SourceTab({ active, onClick, children }: { active: boolean; onClick: () => void; children: React.ReactNode }) {
  // Segmented control — matches `.cg-segment`: ink-100 track, white
  // sliding indicator with shadow-border for the active state.
  return (
    <button
      type="button"
      onClick={onClick}
      className={`flex-1 rounded-[4px] px-3 py-1 text-[12px] font-medium transition-all ${
        active ? "bg-white text-ink-900 shadow-border" : "text-ink-500 hover:text-ink-800"
      }`}
    >
      {children}
    </button>
  );
}

function Chevron({ rotated }: { rotated: boolean }) {
  return (
    <svg
      viewBox="0 0 24 24"
      className={`h-3 w-3 transition-transform ${rotated ? "rotate-90" : ""}`}
      fill="none"
      stroke="currentColor"
      strokeWidth="2"
      strokeLinecap="round"
      strokeLinejoin="round"
    >
      <path d="M9 18l6-6-6-6" />
    </svg>
  );
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

// ---------- cg-flavored primitives ----------

function Callout({ children }: { children: React.ReactNode }) {
  return (
    <div className="rounded-md bg-ink-50 p-3 text-[12px] leading-relaxed text-ink-700 shadow-[inset_0_0_0_1px_oklch(0.96_0_0)]">
      {children}
    </div>
  );
}

function CodeChip({ children }: { children: React.ReactNode }) {
  return (
    <code className="inline-flex items-center rounded-[4px] bg-ink-100 px-1.5 py-0.5 font-mono text-[11px] tracking-[-0.01em] text-ink-900">
      {children}
    </code>
  );
}

function Eyebrow({ children }: { children: React.ReactNode }) {
  return (
    <div className="px-0.5 font-mono text-[10px] font-medium uppercase tracking-[0.12em] text-ink-500">{children}</div>
  );
}

function Field({ label, hint, children }: { label: string; hint?: string; children: React.ReactNode }) {
  return (
    <label className="block">
      <span className="block text-[12px] font-medium text-ink-900">{label}</span>
      {hint && <span className="mt-0.5 block text-[11px] leading-snug text-ink-500">{hint}</span>}
      <span className="mt-2 block">{children}</span>
    </label>
  );
}

function Input({ mono, className, ...rest }: React.InputHTMLAttributes<HTMLInputElement> & { mono?: boolean }) {
  return (
    <input
      {...rest}
      className={`block w-full rounded-md bg-ink-50 px-3 py-2 text-[13px] leading-tight text-ink-900 placeholder:text-ink-400 shadow-[inset_0_0_0_1px_oklch(0.91_0_0)] outline-none transition-all hover:shadow-[inset_0_0_0_1px_oklch(0.82_0_0)] focus:bg-white focus:shadow-[inset_0_0_0_1px_oklch(0.68_0_0)] ${
        mono ? "font-mono" : "font-sans"
      } ${className ?? ""}`}
    />
  );
}

function PrimaryButton({ className, children, ...rest }: React.ButtonHTMLAttributes<HTMLButtonElement>) {
  return (
    <button
      {...rest}
      className={`inline-flex h-8 items-center justify-center rounded-md bg-ink-900 px-3.5 text-[12px] font-medium tracking-[-0.005em] text-white transition-colors hover:bg-ink-800 disabled:cursor-not-allowed disabled:opacity-50 ${
        className ?? ""
      }`}
    >
      {children}
    </button>
  );
}

function GhostButton({ className, children, ...rest }: React.ButtonHTMLAttributes<HTMLButtonElement>) {
  return (
    <button
      {...rest}
      className={`inline-flex h-8 items-center justify-center rounded-md bg-white px-3.5 text-[12px] font-medium tracking-[-0.005em] text-ink-900 shadow-border transition-all hover:bg-ink-50 hover:shadow-border-hover disabled:cursor-not-allowed disabled:opacity-50 ${
        className ?? ""
      }`}
    >
      {children}
    </button>
  );
}

function DangerButton({ className, children, ...rest }: React.ButtonHTMLAttributes<HTMLButtonElement>) {
  return (
    <button
      {...rest}
      className={`inline-flex h-8 items-center justify-center rounded-md bg-danger-600 px-3.5 text-[12px] font-medium tracking-[-0.005em] text-white transition-colors hover:bg-danger-700 disabled:cursor-not-allowed disabled:opacity-50 ${
        className ?? ""
      }`}
    >
      {children}
    </button>
  );
}

function TermDetails({ children }: { children: React.ReactNode }) {
  return (
    <details className="inline-block align-baseline">
      <summary className="cursor-pointer select-none text-[11px] text-ink-500 underline decoration-ink-200 decoration-dotted underline-offset-2 hover:text-ink-700 [&::-webkit-details-marker]:hidden">
        Technical name
      </summary>
      <span className="ml-1 text-[11px] text-ink-500">{children}</span>
    </details>
  );
}
