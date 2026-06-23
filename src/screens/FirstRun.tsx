import { useState } from "react";
import { saveAccount } from "../lib/api";
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

/** Welcome / first-run — paste a Gate API key to connect. Wires to
 *  `save_account(gateway, key)`, defaulting to DEFAULT_GATEWAY_BASE_URL; Dev
 *  mode lets a developer target another environment before connecting. */
export function FirstRun({ onConnected }: { onConnected: () => void }) {
  const [key, setKey] = useState("");
  const [submitting, setSubmitting] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [devMode, setDevMode] = useState(false);
  const [gateway, setGateway] = useState(DEFAULT_GATEWAY_BASE_URL);

  const canSubmit = key.trim().length > 0 && !submitting;

  async function connect() {
    if (!canSubmit) return;
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
          Welcome to Gate <span className="text-gc-accent">Connect</span>
        </div>
        <p className="mt-1.5 max-w-[290px] text-[12.5px] leading-[1.45] text-gc-ink-3">
          Paste your Gate API key to connect your desktop agents — right from the
          menu bar.
        </p>
      </div>

      <div className="mt-5">
        <div className="mb-1.5 font-mono text-[10.5px] font-medium uppercase tracking-[0.08em] text-gc-ink-4">
          Gate API Key
        </div>
        <Input
          leadingIcon={<Icon name="key" size={14} />}
          placeholder="sk-gw-…"
          value={key}
          autoFocus
          spellCheck={false}
          onChange={(e) => setKey(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter") connect();
          }}
        />
        <p className="mt-1 text-[11px] text-gc-ink-4">
          Find it under <span className="font-medium text-gc-ink-2">API Keys</span> in
          your Gate dashboard.
        </p>
      </div>

      {error && (
        <p className="mt-3 text-[11.5px] leading-snug text-gc-error">{error}</p>
      )}

      <Button variant="accent" full className="mt-4" disabled={!canSubmit} onClick={connect}>
        {submitting ? "Connecting…" : "Connect"}
      </Button>

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
