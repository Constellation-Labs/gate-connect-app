import { useState } from "react";
import { openUrl } from "@tauri-apps/plugin-opener";
import { oauthBeginLogin, saveAccount } from "../lib/api";
import { DEFAULT_GATEWAY_BASE_URL, GATEWAY_SERVERS } from "../lib/config";
import { trackError } from "../lib/analytics";
import { ConstellationHexMark } from "../components/gc/ConstellationHexMark";
import { Button, Input } from "../components/gc/ui";
import { Icon } from "../components/gc/Icon";

function hostOf(url: string): string {
  try {
    return new URL(url).host;
  } catch {
    return url;
  }
}

/** Welcome / sign-in. The primary path signs in through the Constellation
 *  (Cognito) Hosted UI in the browser; a secondary, collapsible path keeps the
 *  legacy "paste a Gate API key" flow. Either way the account's gateway URL is
 *  persisted first (defaulting to DEFAULT_GATEWAY_BASE_URL) so the backend can
 *  record the chosen auth mode. Dev mode targets another environment before
 *  connecting. `initialGateway` pre-points at a previously-selected gateway;
 *  `reauth` swaps the copy for an expired-session prompt (OAuth account whose
 *  silent refresh failed). */
export function FirstRun({
  onConnected,
  initialGateway,
  reauth = false,
}: {
  onConnected: () => void;
  initialGateway?: string;
  reauth?: boolean;
}) {
  const [key, setKey] = useState("");
  const [submitting, setSubmitting] = useState(false);
  const [signingIn, setSigningIn] = useState(false);
  const [showKey, setShowKey] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [devMode, setDevMode] = useState(
    !!initialGateway && initialGateway !== DEFAULT_GATEWAY_BASE_URL,
  );
  const [gateway, setGateway] = useState(initialGateway ?? DEFAULT_GATEWAY_BASE_URL);

  const busy = submitting || signingIn;
  const canSubmitKey = key.trim().length > 0 && !busy;

  async function signIn() {
    if (busy) return;
    setError(null);
    setSigningIn(true);
    try {
      // Persist the gateway first (no key) so the account exists on disk; the
      // sign-in then records OAuth as the auth mode against it.
      await saveAccount(gateway, null);
      await oauthBeginLogin();
      onConnected();
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
      trackError(err, "sign_in");
      setSigningIn(false);
    }
  }

  async function connectWithKey() {
    if (!canSubmitKey) return;
    setError(null);
    setSubmitting(true);
    try {
      await saveAccount(gateway, key.trim());
      onConnected();
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
      trackError(err, "sign_in");
      setSubmitting(false);
    }
  }

  return (
    <div className="flex flex-col px-5 pb-5 pt-7">
      <div className="flex flex-col items-center text-center">
        <ConstellationHexMark size={40} fill="#002a5f" />
        <div className="mt-3 text-[19px] font-semibold tracking-[-0.025em] text-gc-navy">
          {reauth ? (
            "Welcome back"
          ) : (
            <>
              Welcome to Gate <span className="text-gc-accent">Connect</span>
            </>
          )}
        </div>
        <p className="mt-1.5 max-w-[290px] text-[12.5px] leading-[1.45] text-gc-ink-3">
          {reauth
            ? "Your session expired. Sign in again to keep routing your desktop agents through Gate."
            : "Sign in to route your desktop agents through Gate - right from the menu bar."}
        </p>
      </div>

      <Button variant="accent" full className="mt-5" disabled={busy} onClick={signIn}>
        <Icon name="shieldCheck" size={15} />
        {signingIn ? "Waiting for browser…" : "Sign in with Constellation"}
      </Button>
      {signingIn && (
        <p className="mt-2 text-center text-[11px] text-gc-ink-4">
          Finish signing in on the page that just opened in your browser.
        </p>
      )}

      {!showKey ? (
        <div className="mt-4 flex justify-center">
          <button
            type="button"
            onClick={() => setShowKey(true)}
            className="text-[12px] font-medium text-gc-ink-3 transition hover:text-gc-ink-2"
          >
            Use an API key instead
          </button>
        </div>
      ) : (
        <div className="mt-5">
          <div className="mb-1.5 font-mono text-[10.5px] font-medium uppercase tracking-[0.08em] text-gc-ink-4">
            Gate API Key <span className="text-gc-ink-5">(legacy)</span>
          </div>
          <Input
            leadingIcon={<Icon name="key" size={14} />}
            placeholder="sk-gw-…"
            value={key}
            autoFocus
            spellCheck={false}
            onChange={(e) => setKey(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter") connectWithKey();
            }}
          />
          <p className="mt-1 text-[11px] text-gc-ink-4">
            Find it under{" "}
            <button
              type="button"
              onClick={() => {
                void openUrl("https://app.constellationgate.ai/api-keys");
              }}
              className="font-medium text-gc-ink-2 underline decoration-gc-line-strong underline-offset-2 transition hover:decoration-gc-ink-3"
            >
              API Keys
            </button>{" "}
            in your Gate dashboard.
          </p>
          <Button full className="mt-3" disabled={!canSubmitKey} onClick={connectWithKey}>
            {submitting ? "Connecting…" : "Connect with key"}
          </Button>
        </div>
      )}

      {error && (
        <p className="mt-3 text-[11.5px] leading-snug text-gc-error">{error}</p>
      )}

      <div className="mt-4">
        <div className="flex justify-center">
          <button
            type="button"
            onClick={() => setDevMode((v) => !v)}
            className="inline-flex items-center gap-1.5 text-[12.5px] font-medium text-gc-ink-3"
          >
            <Icon name="settings" size={14} />
            Dev mode
          </button>
        </div>

        {!devMode ? (
          <div className="mt-3 text-center font-mono text-[10.5px] text-gc-ink-5">
            {gateway}
          </div>
        ) : (
          <>
            <div className="mb-1.5 mt-3 font-mono text-[10.5px] font-medium uppercase tracking-[0.08em] text-gc-ink-4">
              Gateway server
            </div>
            <div className="flex flex-col gap-2">
              {GATEWAY_SERVERS.map((server) => {
                const active = server.url === gateway;
                return (
                  <button
                    key={server.url}
                    type="button"
                    onClick={() => setGateway(server.url)}
                    disabled={active}
                    className="flex items-center gap-3 rounded bg-gc-surface px-3 py-2 text-left shadow-border transition hover:shadow-border-hover disabled:cursor-default disabled:hover:shadow-border"
                  >
                    <div className="min-w-0 flex-1">
                      <div className="text-[13px] font-medium text-gc-ink">{server.label}</div>
                      <div className="truncate font-mono text-[10.5px] text-gc-ink-4">
                        {hostOf(server.url)}
                      </div>
                    </div>
                    {active && <Icon name="check" size={15} className="shrink-0 text-gc-accent" />}
                  </button>
                );
              })}
            </div>
          </>
        )}
      </div>
    </div>
  );
}
