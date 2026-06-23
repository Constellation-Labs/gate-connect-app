import { useState } from "react";
import { saveAccount } from "../lib/api";
import { DEFAULT_GATEWAY_BASE_URL } from "../lib/config";
import { trackError } from "../lib/analytics";
import { ConstellationHexMark } from "../components/gc/ConstellationHexMark";
import { Button, Input } from "../components/gc/ui";
import { Icon } from "../components/gc/Icon";

/** Welcome / first-run — paste a Gate API key to connect. Wires to
 *  `save_account(DEFAULT_GATEWAY_BASE_URL, key)`. */
export function FirstRun({ onConnected }: { onConnected: () => void }) {
  const [key, setKey] = useState("");
  const [submitting, setSubmitting] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const canSubmit = key.trim().length > 0 && !submitting;

  async function connect() {
    if (!canSubmit) return;
    setError(null);
    setSubmitting(true);
    try {
      await saveAccount(DEFAULT_GATEWAY_BASE_URL, key.trim());
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

      <div className="mt-3 text-center font-mono text-[10.5px] text-gc-ink-5">
        {DEFAULT_GATEWAY_BASE_URL}
      </div>
    </div>
  );
}
