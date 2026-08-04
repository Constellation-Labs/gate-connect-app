import { useRef, useState } from "react";
import { openExternal } from "../lib/openExternal";
import { oauthBeginLogin, saveAccount } from "../lib/api";
import { DEFAULT_GATEWAY_BASE_URL, GATEWAY_SERVERS, GATE_API_KEYS_URL } from "../lib/config";
import { trackError } from "../lib/analytics";
import { classifyError, type ClassifiedError } from "../lib/errors";
import { markOAuthOfferSeen } from "../lib/oauthOffer";
import { secretStoreName, usePlatform } from "../lib/platform";
import { ConstellationHexMark } from "../components/gc/ConstellationHexMark";
import { Button, Input, ErrorNote } from "../components/gc/ui";
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
  startOnKey,
  reauth = false,
}: {
  onConnected: () => void;
  initialGateway?: string;
  /** Open directly on the API-key form. */
  startOnKey?: boolean;
  reauth?: boolean;
}) {
  const [key, setKey] = useState("");
  const [submitting, setSubmitting] = useState(false);
  const [signingIn, setSigningIn] = useState(false);
  // Seeded so the org-picker dead end can hand a user straight to the key
  // form: for someone with no organization and no admin to ask, that is the
  // only path forward, and making them find the disclosure again is a tax.
  const [showKey, setShowKey] = useState(startOnKey ?? false);
  const [error, setError] = useState<ClassifiedError | null>(null);
  const [devMode, setDevMode] = useState(
    !!initialGateway && initialGateway !== DEFAULT_GATEWAY_BASE_URL,
  );
  const [gateway, setGateway] = useState(initialGateway ?? DEFAULT_GATEWAY_BASE_URL);
  const platform = usePlatform();

  const busy = submitting || signingIn;
  const canSubmitKey = key.trim().length > 0 && !busy;

  // Bumped by Cancel so a stale sign-in attempt can't re-lock the screen or
  // surface its error after the user has moved on. There is no backend abort
  // for the pending browser flow; if the user does finish it in the browser,
  // onConnected still fires - completing is never wrong.
  const signInAttempt = useRef(0);

  async function signIn() {
    if (busy) return;
    setError(null);
    setSigningIn(true);
    const attempt = ++signInAttempt.current;
    try {
      // Persist the gateway first (no key) so the account exists on disk; the
      // sign-in then records OAuth as the auth mode against it.
      await saveAccount(gateway, null);
      await oauthBeginLogin();
      onConnected();
    } catch (err) {
      trackError(err, "sign_in");
      if (attempt === signInAttempt.current) {
        setError(classifyError(err, "sign_in"));
        setSigningIn(false);
      }
    }
  }

  function cancelSignIn() {
    signInAttempt.current++;
    setSigningIn(false);
  }

  async function connectWithKey() {
    if (!canSubmitKey) return;
    setError(null);
    setSubmitting(true);
    try {
      await saveAccount(gateway, key.trim());
      // Choosing the key here answers the "would you rather sign in?" question.
      // Without this the one-time offer arrives on the next launch and reverses
      // a decision the user just made on purpose.
      markOAuthOfferSeen();
      onConnected();
    } catch (err) {
      setError(classifyError(err, "sign_in"));
      trackError(err, "sign_in");
      setSubmitting(false);
    }
  }

  return (
    <div className="flex flex-col px-5 pb-5 pt-7">
      <div className="flex flex-col items-center text-center">
        <ConstellationHexMark size={40} />
        {/* An h1, not a div: this is the first screen of the app and it had no
            heading at all, so a screen reader landed in an unlabelled region
            with nothing to orient by. */}
        <h1 className="mt-3 text-[17px] font-semibold tracking-[-0.02em] text-gc-navy">
          {reauth ? (
            "Welcome back"
          ) : (
            <>
              Welcome to Gate <span className="text-gc-accent">Connect</span>
            </>
          )}
        </h1>
        <p className="mt-1.5 max-w-[290px] text-[12.5px] leading-[1.45] text-gc-ink-3">
          {reauth
            ? "Your session expired. Sign in again to keep routing your desktop agents through Gate."
            : "Sign in to route your desktop agents through Gate, right from the menu bar."}
        </p>
      </div>

      <Button variant="accent" full className="mt-5" disabled={busy} onClick={signIn}>
        <Icon name="shieldCheck" size={15} />
        {signingIn ? "Waiting for browser…" : "Sign in with Constellation"}
      </Button>
      {signingIn && (
        <div className="mt-2 flex flex-col items-center gap-1.5">
          <p className="text-center text-[11px] text-gc-ink-3">
            Finish signing in on the page that just opened in your browser.
          </p>
          <button
            type="button"
            onClick={cancelSignIn}
            className="text-[12px] font-medium text-gc-ink-3 transition hover:text-gc-ink"
          >
            Cancel
          </button>
        </div>
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
          <div className="mb-1.5 font-mono text-[10.5px] font-medium uppercase tracking-[0.08em] text-gc-ink-3">
            Gate API Key
          </div>
          <p className="mb-2 text-[11px] leading-snug text-gc-ink-3">
            Best for CI or headless machines where browser sign-in isn’t
            practical. Otherwise, sign in with Constellation: nothing to
            paste, and it refreshes on its own.
          </p>
          <Input
            leadingIcon={<Icon name="key" size={14} />}
            placeholder="sk-gw-…"
            secret
            value={key}
            autoFocus
            spellCheck={false}
            onChange={(e) => setKey(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter") connectWithKey();
            }}
          />
          {/* The destination, said at the moment the secret is in the user's
              hands. The pinned footer says this everywhere else, but it is
              suppressed here (no account exists yet), so the one screen that
              handles a live key was the one screen that never named the
              vault. */}
          <p className="mt-1.5 text-[11px] leading-snug text-gc-ink-3">
            Saved to {secretStoreName(platform)}. Your config files get the
            gateway URL, never the key.
          </p>
          <p className="mt-1 text-[11px] text-gc-ink-3">
            Find it under{" "}
            <button
              type="button"
              onClick={() => {
                void openExternal(GATE_API_KEYS_URL);
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

      {error && <ErrorNote error={error} className="mt-3" />}

      {/* Environment picker for people working on Gate itself; kept out of
          the main flow's visual hierarchy (footer-quiet, no icon) so the
          sign-in decision stays a two-option screen. */}
      <div className="mt-5">
        {!devMode ? (
          <div className="flex items-baseline justify-center gap-2 text-center">
            <span className="font-mono text-[10.5px] text-gc-ink-3">{gateway}</span>
            <button
              type="button"
              onClick={() => setDevMode(true)}
              className="text-[10.5px] font-medium text-gc-ink-3 underline decoration-gc-line-strong underline-offset-2 transition hover:text-gc-ink"
            >
              change
            </button>
          </div>
        ) : (
          <>
            <div className="mb-1.5 flex items-center justify-between">
              <span className="font-mono text-[10.5px] font-medium uppercase tracking-[0.08em] text-gc-ink-3">
                Gateway server
              </span>
              <button
                type="button"
                onClick={() => setDevMode(false)}
                className="text-[10.5px] font-medium text-gc-ink-3 transition hover:text-gc-ink"
              >
                Hide
              </button>
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
                      <div className="truncate font-mono text-[10.5px] text-gc-ink-3">
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
