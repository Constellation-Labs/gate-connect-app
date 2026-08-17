import { useCallback, useState } from "react";
import type { ProxyState, Tool } from "./api";
import {
  connectTool,
  disconnectTool,
  listTools,
  proxySetDomain,
  proxyStatus,
  proxyTrustCa,
} from "./api";
import { track, trackError } from "./analytics";

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
  | { kind: "trust" };

export interface RoutingSnapshot {
  tools: Tool[];
  proxy: ProxyState | null;
}

/** Thrown internally when the user declines a gate. Never surfaces: declining
 *  is an answer, not a failure, so it resolves quietly. */
class Declined extends Error {}

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
   * Route or unroute one config-file tool. `force` skips the drift gate, which
   * is how the review dialog's "Replace config and protect" comes back in.
   */
  const setAppRouted = useCallback(
    async (slug: string, routed: boolean, force = false) => {
      if (busy) return;
      setBusy(true);
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
      } catch (e) {
        if (e instanceof Declined) return;
        trackError(e, "connect", { tool: slug, routed });
        onError?.(e, routed ? "connect" : "disconnect");
      } finally {
        await resync();
        setBusy(false);
      }
    },
    [busy, tools, ask, ensureCaTrusted, resync, onError],
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
        if (routed) await ensureCaTrusted();
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
    [busy, ensureCaTrusted, resync, onError],
  );

  return { busy, prompt, resolvePrompt, setAppRouted, setDomainRouted };
}
