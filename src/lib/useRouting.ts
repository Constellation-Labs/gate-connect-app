import { useCallback, useState } from "react";
import type { ProxyState, Tool } from "./api";
import {
  connectTool,
  disconnectTool,
  listTools,
  proxyDisable,
  proxyEnable,
  proxySetDomain,
  proxySetEnvExport,
  proxyStatus,
  proxyTrustCa,
  proxyUntrustCa,
} from "./api";
import { track, trackError } from "./analytics";
import { cascadeTargets } from "./groups";
import type { Group } from "./groups";

/**
 * Routing actions for the new window UI: the gates that have to happen before a
 * tool's config is rewritten, and the re-sync that has to happen after.
 *
 * Written fresh rather than extracted from `App.tsx` for two reasons. The design
 * sequences these differently - a review dialog rather than the popover's
 * takeover - and `App.tsx`'s version is entangled with popover concerns
 * (`pinPopover`, screen depth, takeover z-order) that mean nothing in a window.
 * The popover's copy dies with the popover.
 *
 * The gates, in order, and why each exists:
 *
 * 1. **Drift.** A tool whose config changed outside Gate is not adopted
 *    silently: connecting rewrites settings somebody hand-wrote, so the design
 *    puts a review dialog in front of it. Turning a tool *off* skips this - it
 *    restores what was there.
 * 2. **Certificate.** Connecting starts the proxy engine, which trusts the CA,
 *    which prompts the operating system. Ask first, because a surprise system
 *    dialog reads as malware.
 *
 * Every path re-reads tools and proxy state in a `finally`. Optimistic UI is
 * wrong here: connecting can flip a provider headline and auto-start the
 * engine, so backend truth is the only safe thing to render.
 *
 * Beyond the per-app and per-family switches this also owns the three actions
 * that are about the engine rather than about one app: the master toggle, the
 * shell-environment channel, and removing the certificate. They live here
 * because they share the certificate gate and the re-sync, and because the
 * window had no way to reach any of them.
 */

/** A decision the UI has to collect before the action can continue. */
export type RoutingPrompt =
  | {
      kind: "drift";
      slug: string;
      name: string;
      /** What Gate found, for the dialog's subject row. */
      existingConfig: string;
    }
  | { kind: "trust" }
  /** Removing the certificate, which is not a gate on the way to something
   * else: it is the action, and it stops every routed domain. Confirmed for
   * the same reason the destructive dialogs are. */
  | { kind: "untrust" };

export interface RoutingSnapshot {
  tools: Tool[];
  proxy: ProxyState | null;
}

/** Thrown internally when the user declines a gate. Never surfaces: declining
 *  is an answer, not a failure, so it resolves quietly. */
class Declined extends Error {}

/**
 * Some members of a family switch failed. Carries their names, because the
 * useful sentence is "couldn't connect Codex and OpenCode", not "couldn't
 * connect this tool" - which names nobody, reports no partial success, and
 * leaves the user to work out which rows moved.
 */
export class FamilyCascadeError extends Error {
  constructor(
    readonly names: string[],
    readonly attempted: number,
    readonly routed: boolean,
    readonly cause: unknown,
  ) {
    super(`${names.length} of ${attempted} members failed`);
  }
}

export function useRouting({
  tools,
  proxy,
  onSnapshot,
  onError,
}: {
  tools: Tool[];
  proxy: ProxyState | null;
  /** Fresh backend truth after any action, successful or not. */
  onSnapshot: (next: RoutingSnapshot) => void;
  /** A failure the user should see, already classified by the caller. */
  onError?: (error: unknown, context: string) => void;
}) {
  const [busy, setBusy] = useState(false);
  /**
   * Slugs whose last config write failed.
   *
   * Without this, a failed write left the row asserting whatever it said before:
   * the error went to a transient banner, and the status line - which is the
   * thing the user reads next to the switch they just clicked - carried on
   * claiming the old state. A write that failed is not an observation the sweep
   * can make, because nothing was written and there is no trace on disk to
   * probe, so it has to be remembered here.
   *
   * Session-only and per-slug, cleared the moment a write for that slug
   * succeeds. Not persisted: a failure that survived a restart would outlive
   * whatever caused it.
   */
  const [writeFailures, setWriteFailures] = useState<ReadonlySet<string>>(new Set());
  const [prompt, setPrompt] = useState<RoutingPrompt | null>(null);
  // The pending gate's resolver. A promise the dialog completes, so the action
  // reads as a straight sequence rather than a callback chain.
  const [decide, setDecide] = useState<((allow: boolean) => void) | null>(null);

  const ask = useCallback((next: RoutingPrompt) => {
    return new Promise<void>((resolve, reject) => {
      setPrompt(next);
      setDecide(() => (allow: boolean) => {
        setPrompt(null);
        setDecide(null);
        if (allow) resolve();
        else reject(new Declined());
      });
    });
  }, []);

  /** Answer the open prompt. `false` abandons the action. */
  const resolvePrompt = useCallback(
    (allow: boolean) => {
      decide?.(allow);
    },
    [decide],
  );

  const resync = useCallback(async () => {
    const [freshTools, freshProxy] = await Promise.all([
      listTools().catch(() => tools),
      proxyStatus().catch(() => proxy),
    ]);
    onSnapshot({ tools: freshTools, proxy: freshProxy });
    return freshProxy;
  }, [tools, proxy, onSnapshot]);

  /** Trust the CA if it is not trusted yet, asking first. */
  const ensureCaTrusted = useCallback(async () => {
    // Null on a platform with no proxy subsystem: nothing to trust, and nothing
    // to interrupt the user with.
    if (!proxy || proxy.ca_trusted) return;
    await ask({ kind: "trust" });
    await proxyTrustCa();
  }, [proxy, ask]);

  /**
   * Start the engine if it is not running.
   *
   * A domain routes through the engine, and `proxy_set_domain` only records the
   * flag - unlike `connect_tool`, which starts the engine on its own. Without
   * this, a chat-domain switch in a window whose engine is off writes intent,
   * routes nothing, and leaves the user no way back: the popover's master switch
   * was the only thing that could start it, and this shell had no equivalent.
   *
   * Called after `ensureCaTrusted`, so `proxy_enable`'s own trust step is a
   * no-op and the OS prompt has already been asked for.
   */
  const ensureEngineRunning = useCallback(async () => {
    // Null on a platform with no proxy subsystem: there is no engine to start.
    if (!proxy || proxy.running) return;
    await proxyEnable();
  }, [proxy]);

  /**
   * Route or unroute one config-file tool. `force` skips the drift gate, which
   * is how the review dialog's "Replace config and protect" comes back in.
   *
   * Resolves to whether a config was actually written, so a caller can follow up
   * on a real change and stay quiet after a declined gate or a failure.
   */
  const setAppRouted = useCallback(
    async (slug: string, routed: boolean, force = false): Promise<boolean> => {
      if (busy) return false;
      setBusy(true);
      let changed = false;
      try {
        const tool = tools.find((t) => t.slug === slug);
        if (routed) {
          if (!force && tool?.status.kind === "drifted") {
            await ask({
              kind: "drift",
              slug,
              name: tool.name,
              existingConfig: tool.status.reason,
            });
          }
          await ensureCaTrusted();
          await connectTool(slug, tool?.default_upstream_url ?? "");
        } else {
          await disconnectTool(slug);
        }
        track("tool_toggled", { tool: slug, routed });
        changed = true;
        setWriteFailures((prev) => {
          if (!prev.has(slug)) return prev;
          const next = new Set(prev);
          next.delete(slug);
          return next;
        });
      } catch (e) {
        // Declining a gate is an answer, not a failure: nothing was written and
        // there is nothing to report - and in particular the row must not be
        // marked failed, because the user chose this.
        if (!(e instanceof Declined)) {
          trackError(e, "connect", { tool: slug, routed });
          onError?.(e, routed ? "connect" : "disconnect");
          setWriteFailures((prev) => new Set(prev).add(slug));
        }
      } finally {
        await resync();
        setBusy(false);
      }
      return changed;
    },
    [busy, tools, ask, ensureCaTrusted, resync, onError],
  );

  /**
   * Route or unroute a whole family.
   *
   * Which members are touched is `cascadeTargets`, shared with the popover: chat
   * members never ride a family switch, a drifted config is never adopted by
   * one, and members already in the target state are left alone.
   *
   * The certificate is trusted **ahead of the loop**, not inside it. A config
   * member's connect auto-enables the engine, which trusts the CA, so the system
   * dialog belongs before the first command rather than sprung from member
   * three. Declining aborts the whole family, which is what the implicit trust
   * did to that member's connect anyway.
   *
   * Every member is attempted even after one fails, and the failures are named.
   */
  const setFamilyRouted = useCallback(
    async (group: Group, routed: boolean): Promise<boolean> => {
      if (busy) return false;
      const targets = cascadeTargets(group, routed);
      if (targets.length === 0) return false;
      setBusy(true);
      let changed = false;
      const failed: string[] = [];
      let last: unknown = null;
      try {
        if (routed) await ensureCaTrusted();
        for (const member of targets) {
          try {
            if (member.kind === "config" && member.tool) {
              await (routed
                ? connectTool(member.key, member.tool.default_upstream_url)
                : disconnectTool(member.key));
            } else if (member.domain) {
              await proxySetDomain(member.key, routed);
            }
            changed = true;
          } catch (e) {
            failed.push(member.name);
            last = e;
            trackError(e, "connect", { provider: group.id, tool: member.key, routed });
          }
        }
        track("group_toggled", { provider: group.id, enabled: routed });
        if (last !== null) {
          onError?.(new FamilyCascadeError(failed, targets.length, routed, last), "connect");
        }
      } catch (e) {
        // Only the pre-flight lands here; a member failure is caught per member.
        if (!(e instanceof Declined)) {
          trackError(e, "connect", { provider: group.id, routed });
          onError?.(e, "connect");
        }
      } finally {
        await resync();
        setBusy(false);
      }
      return changed;
    },
    [busy, ensureCaTrusted, resync, onError],
  );

  /**
   * Route or unroute one proxy domain. No drift gate: a domain has no
   * hand-written config to preserve, only an enabled flag.
   */
  const setDomainRouted = useCallback(
    async (slug: string, routed: boolean) => {
      if (busy) return;
      setBusy(true);
      try {
        if (routed) {
          await ensureCaTrusted();
          await ensureEngineRunning();
        }
        await proxySetDomain(slug, routed);
        track("domain_toggled", { domain: slug, routed });
      } catch (e) {
        if (e instanceof Declined) return;
        trackError(e, "provider_toggle", { domain: slug, routed });
        onError?.(e, "domain");
      } finally {
        await resync();
        setBusy(false);
      }
    },
    [busy, ensureCaTrusted, ensureEngineRunning, resync, onError],
  );

  /**
   * Turn all routing on or off: the engine itself, not one app.
   *
   * The popover's master switch (`App.tsx`'s `toggleProxy`) minus the takeover.
   * The certificate is trusted on the way on, because enabling is the step that
   * prompts, and never on the way off, which is promptless. The backend owns the
   * routed set across the toggle - off snapshots what was on, on restores that
   * snapshot - so this reflects the result through `resync` rather than
   * reconstructing it here.
   *
   * Reports whether the engine actually moved, so the caller can follow up with
   * the running-apps offer: every routed tool is on its old route until it
   * restarts, exactly as after a config write.
   */
  const setMasterRouted = useCallback(
    async (routed: boolean): Promise<boolean> => {
      if (busy) return false;
      setBusy(true);
      let changed = false;
      try {
        if (routed) await ensureCaTrusted();
        await (routed ? proxyEnable() : proxyDisable());
        track(routed ? "proxy_enabled" : "proxy_disabled", { source: "toggle" });
        changed = true;
      } catch (e) {
        if (!(e instanceof Declined)) {
          trackError(e, "proxy_toggle");
          onError?.(e, "proxy_toggle");
        }
      } finally {
        await resync();
        setBusy(false);
      }
      return changed;
    },
    [busy, ensureCaTrusted, resync, onError],
  );

  /**
   * Turn the shell-environment channel on or off.
   *
   * Its own action rather than a branch of `setMasterRouted`, for the reason
   * `App.tsx` gives: this never starts or stops the engine, it decides whether
   * the proxy is also written into the user's environment - a machine-wide
   * change that reaches `git` and `curl`, not just the AI tools.
   */
  const setEnvExport = useCallback(
    async (enabled: boolean) => {
      if (busy) return;
      setBusy(true);
      try {
        await proxySetEnvExport(enabled);
        track(enabled ? "env_export_enabled" : "env_export_disabled");
      } catch (e) {
        trackError(e, "env_export");
        onError?.(e, "env_export");
      } finally {
        await resync();
        setBusy(false);
      }
    },
    [busy, resync, onError],
  );

  /**
   * Remove the Gate certificate from the system trust store.
   *
   * Confirmed first: every routed domain stops being inspected the moment this
   * lands, and the engine keeps running, so the state it leaves behind is one
   * the user cannot read off a switch. Turning routing off deliberately does
   * *not* do this - re-enabling stays promptless - which is why it is a separate
   * action rather than part of the master toggle.
   */
  const untrustCa = useCallback(async () => {
    if (busy) return;
    setBusy(true);
    try {
      await ask({ kind: "untrust" });
      await proxyUntrustCa();
      track("ca_untrusted");
    } catch (e) {
      if (!(e instanceof Declined)) {
        trackError(e, "untrust_ca");
        onError?.(e, "untrust_ca");
      }
    } finally {
      await resync();
      setBusy(false);
    }
  }, [busy, ask, resync, onError]);

  return {
    busy,
    prompt,
    resolvePrompt,
    setAppRouted,
    setFamilyRouted,
    setDomainRouted,
    setMasterRouted,
    setEnvExport,
    untrustCa,
    writeFailures,
  };
}
