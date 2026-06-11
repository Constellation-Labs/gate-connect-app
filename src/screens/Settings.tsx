import { useState } from "react";
import type { Account } from "../lib/api";
import { SubHeader, SectionLabel, ConnPill, Button, Input } from "../components/gc/ui";
import { Icon } from "../components/gc/Icon";

function hostOf(url: string): string {
  try {
    return new URL(url).host;
  } catch {
    return url;
  }
}

/** Settings — workspace + Gate API key management. The key itself is held in
 *  the OS keychain and never returned to the UI, so it shows masked; Replace
 *  key calls save_account, Disconnect calls clear_account. */
export function Settings({
  account,
  onBack,
  onReplaceKey,
  onDisconnect,
}: {
  account: Account;
  onBack: () => void;
  onReplaceKey: (key: string) => Promise<void>;
  onDisconnect: () => Promise<void>;
}) {
  const [replacing, setReplacing] = useState(false);
  const [newKey, setNewKey] = useState("");
  const [submitting, setSubmitting] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function saveKey() {
    if (newKey.trim().length === 0 || submitting) return;
    setError(null);
    setSubmitting(true);
    try {
      await onReplaceKey(newKey.trim());
      setReplacing(false);
      setNewKey("");
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setSubmitting(false);
    }
  }

  async function disconnect() {
    if (submitting) return;
    setError(null);
    setSubmitting(true);
    try {
      await onDisconnect();
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setSubmitting(false);
    }
  }

  return (
    <div className="flex flex-col">
      <SubHeader title="Settings" onBack={onBack} />

      <SectionLabel>Workspace</SectionLabel>
      <div className="flex items-center gap-3 px-3.5 py-2.5">
        <div className="flex h-[30px] w-[30px] shrink-0 items-center justify-center rounded-lg bg-gc-accent-wash text-gc-accent">
          <Icon name="cube" size={16} />
        </div>
        <div className="min-w-0 flex-1">
          <div className="truncate text-[13px] font-medium text-gc-ink">
            {hostOf(account.gateway_base_url)}
          </div>
          <div className="truncate font-mono text-[10.5px] text-gc-ink-4">
            {account.gateway_base_url}
          </div>
        </div>
        <ConnPill state={account.has_api_key ? "connected" : "signedout"} />
      </div>

      <SectionLabel>Gate API Key</SectionLabel>
      <div className="px-3.5">
        {!replacing ? (
          <div className="flex h-9 items-center gap-2 rounded bg-gc-subtle px-3 text-gc-ink-3 shadow-border">
            <Icon name="key" size={14} className="text-gc-ink-4" />
            <span className="flex-1 font-mono text-[12px] tracking-wide">
              {account.has_api_key ? "sk-gw-••••••••••••••••" : "No key stored"}
            </span>
          </div>
        ) : (
          <div className="flex flex-col gap-2">
            <Input
              leadingIcon={<Icon name="key" size={14} />}
              placeholder="Enter new sk-gw-… key"
              value={newKey}
              autoFocus
              spellCheck={false}
              onChange={(e) => setNewKey(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Enter") saveKey();
              }}
            />
            <div className="flex gap-2">
              <Button
                variant="accent"
                className="flex-1"
                disabled={newKey.trim().length === 0 || submitting}
                onClick={saveKey}
              >
                {submitting ? "Saving…" : "Save key"}
              </Button>
              <Button
                className="flex-1"
                onClick={() => {
                  setReplacing(false);
                  setNewKey("");
                  setError(null);
                }}
              >
                Cancel
              </Button>
            </div>
          </div>
        )}
        {error && <p className="mt-2 text-[11.5px] text-gc-error">{error}</p>}
        {account.has_api_key && !replacing && (
          <p className="mt-1.5 text-[11px] text-gc-ink-4">Stored in your keychain.</p>
        )}
      </div>

      {!replacing && (
        <div className="mt-3 flex items-center gap-4 px-3.5 pb-1">
          <button
            type="button"
            onClick={() => setReplacing(true)}
            className="inline-flex items-center gap-1.5 text-[12.5px] font-medium text-gc-accent"
          >
            <Icon name="refresh" size={14} />
            Replace key
          </button>
          <button
            type="button"
            onClick={disconnect}
            disabled={submitting}
            className="inline-flex items-center gap-1.5 text-[12.5px] font-medium text-gc-error"
          >
            <Icon name="trash" size={14} />
            Disconnect
          </button>
        </div>
      )}
    </div>
  );
}
