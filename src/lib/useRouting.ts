import { useCallback, useState } from "react";
import type { ProxyState, Tool } from "./api";
import {
  connectTool,
  disconnectTool,
  listTools,
  proxyEnable,
  hermesUpstreamCoverage,
  proxySetDomain,
  proxySetEnvExport,
  proxyStatus,
  proxyTrustCa,
  proxyUntrustCa,
} from "./api";
import { track, trackError } from "./analytics";
import { describe, logInfo, logWarn } from "./log";
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
 * Beyond the per-app and per-family switches this also owns the two actions
 * that are about the engine rather than about one app: the shell-environment
 * channel, and removing the certificate. They live here because they share the
 * certificate gate and the re-sync, and because the window had no way to reach
 * either of them.
 *
 * It used to own a third, `setMasterRouted`. There is no master switch now -
 * routing runs for as long as the app is open, started at launch and torn down
 * at quit - so the only remaining way to stop it from the UI is to close the
 * window's app.
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
  /** Turning OpenCode on turns the shell-environment channel on with it.
   *
   * Not a warning bolted onto a switch. This used to call the variables "the
   * only way OpenCode routes", which is wrong and was wrong when it was
   * written: `integrations/opencode.rs` rewrites
   * `provider.<id>.options.baseURL` to the loopback relay, needing neither a
   * variable nor the CA. What that rewrite cannot do is cover what it never
   * saw - it is a snapshot taken at connect over a fixed allowlist, so a
   * provider added later, one outside it, or one skipped as local has no entry,
   * and the environment is what carries that traffic. Hence both, and hence the
   * prompt: the variables are machine-wide, reaching git, curl and npm as much
   * as OpenCode, which is a large enough change to ask about - the same reason
   * `env_proxy.rs` made the channel declinable in the first place.
   *
   * Only raised when the channel is actually off. A dialog announcing a
   * side effect that already happened is noise, and it would fire on every
   * OpenCode toggle for the majority of users, for whom the channel is on by
   * default. */
  | { kind: "opencode-env" }
  /** Turning Hermes on, when the provider Hermes talks to is a domain Gate
   * knows and has switched off.
   *
   * Hermes is one of two rows (with OpenClaw) whose upstream is chosen by the
   * user and lives in a *different* section: `groups.ts` bundles Claude's tool
   * with `anthropic` and `claude-web`, and ChatGPT's with its own provider
   * rows, so for those one switch routes the tool AND intercepts what it talks
   * to. `hermes` is `["hermes"]`. Turning it on routes Hermes and inspects
   * nothing, and the app says Protected while every request tunnels past
   * unseen - which is exactly what happened on the machine that prompted this,
   * for an afternoon.
   *
   * Carries the domains rather than a boolean, because the dialog names the
   * provider and the confirm enables those specific slugs. Only raised for
   * `switched_off`: an upstream no domain claims (Bedrock, a self-hosted
   * endpoint) has no remedy, and a dialog offering one would be lying. */
  | {
      kind: "hermes-provider";
      domains: { name: string; host: string; slug: string }[];
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

/** The one tool whose switch also flips the shell-environment channel. Named
 *  once rather than spelled inline, because the dialog copy and the action have
 *  to be talking about the same row. */
const OPENCODE_SLUG = "opencode";

/** The tool whose provider lives in another section, so its switch alone
 *  routes it without inspecting anything. OpenClaw has the same shape and the
 *  same CLI-only coverage note; it is not wired here yet. */
const HERMES_SLUG = "hermes";

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

  /**
   * The same gate, for a question whose "no" is a real answer rather than an
   * abandonment.
   *
   * `ask` rejects with `Declined` because its callers - the drift review, the
   * certificate - are gates *on the way to* something: saying no means the
   * action does not happen. The Hermes provider question is not that. Routing
   * Hermes without inspecting OpenRouter is a coherent state a person may
   * want, so declining has to let the connect proceed. Returning a boolean
   * rather than throwing is what keeps that difference visible at the call
   * site instead of hiding it in a `catch`.
   */
  const askOptional = useCallback(
    (next: RoutingPrompt) =>
      new Promise<boolean>((resolve) => {
        setPrompt(next);
        setDecide(() => (allow: boolean) => {
          setPrompt(null);
          setDecide(null);
          resolve(allow);
        });
      }),
    [],
  );

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

  /**
   * Release the busy flag, then re-read the world.
   *
   * **That order is the point.** Every write below ends in a `finally` that did
   * `await resync(); setBusy(false);` - so a throw from the resync skipped the
   * release and left `busy` stuck on for the life of the window. `BaseSwitch`
   * drops clicks while busy and each writer returns early on it, so one failed
   * resync silently killed every switch in the app: no error, no log line, and
   * nothing to do but reload. It escaped as an unhandled rejection out of the
   * `void routing.…` call sites, which is why it left no trace at all.
   *
   * Releasing first means a failed re-read costs a stale row until the next poll
   * instead of the ability to click anything, and the resync's own failure is
   * reported rather than thrown into the void.
   */
  const settle = useCallback(async () => {
    setBusy(false);
    try {
      await resync();
    } catch (e) {
      trackError(e, "resync");
    }
  }, [resync]);

  /** Trust the CA if it is not trusted yet, asking first.
   *
   * The state it reads is `proxy.ca_trusted` and nothing else. A session-long
   * memo of "we installed it" was tried here, to stop a cascade's later members
   * re-asking before the prop caught up, and it turned out to guard nothing the
   * cascade's own single gate does not already handle - while being able to
   * outlive the certificate itself, since a CA can go away by a reset, a
   * `certutil -D` or another window. If a repeated prompt ever does appear, the
   * fix is the gate in `useSectionRouting`, not a belief cached here. */
  const ensureCaTrusted = useCallback(async () => {
    // Null on a platform with no proxy subsystem: nothing to trust, and nothing
    // to interrupt the user with.
    if (!proxy || proxy.ca_trusted) return;
    // Logged on both sides of the gate. The await between them is an in-app
    // modal, and a promise that never settles there leaves `busy` stuck on with
    // no error anywhere - a pair of lines is what distinguishes "the user never
    // answered" from "trusting the CA failed", which look identical from
    // outside.
    logInfo("routing: asking to trust the CA");
    await ask({ kind: "trust" });
    logInfo("routing: trust accepted, installing the CA");
    await proxyTrustCa();
    logInfo("routing: CA installed");
  }, [proxy, ask]);

  /**
   * The same gate, asked once for a whole cascade and answered with whether to
   * go on.
   *
   * `setFamilyRouted` makes the same call for the same reason - "the dialog
   * belongs before the first command rather than sprung from member three" - but
   * the window shell's app switch cascades outside this hook, so it needs the
   * gate on its own. Declining used to abandon only the member that raised it,
   * leaving the next member's dialog on screen: the person said no and was asked
   * again, about the same certificate.
   *
   * False means do not proceed. A declined gate is an answer and reports
   * nothing; a failed install is reported here, where `onError` lives.
   */
  const confirmCaTrusted = useCallback(async (): Promise<boolean> => {
    try {
      await ensureCaTrusted();
      return true;
    } catch (e) {
      if (!(e instanceof Declined)) {
        trackError(e, "trust_ca");
        onError?.(e, "trust_ca");
      }
      return false;
    }
  }, [ensureCaTrusted, onError]);

  /**
   * Start the engine if it is not running.
   *
   * A domain routes through the engine, and `proxy_set_domain` only records the
   * flag - unlike `connect_tool`, which starts the engine on its own. Without
   * this, a chat-domain switch in a window whose engine is off writes intent,
   * routes nothing, and leaves the user no way back. Rarely reached now that
   * the launch enables the engine, but a launch whose enable failed is exactly
   * the case it was written for.
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
      if (busy) {
        // Not a no-op worth passing over in silence: `busy` sticking on is what
        // makes every switch in the window stop responding, and without this
        // line the symptom is a click that does nothing, anywhere.
        logWarn(`routing: ignored ${slug} -> ${routed ? "on" : "off"}, a write is already in flight`);
        return false;
      }
      setBusy(true);
      let changed = false;
      try {
        const tool = tools.find((t) => t.slug === slug);
        // Asked before the drift gate, not after: this is a question about what
        // else the click turns on, and the answer decides whether there is
        // anything to review at all. Two dialogs in a row is the honest cost of
        // a click that does two things.
        const couplesEnvExport =
          routed && slug === OPENCODE_SLUG && proxy !== null && !proxy.env_export_opted_in;
        if (couplesEnvExport) await ask({ kind: "opencode-env" });

        // Hermes's provider, asked in the same place and for the same reason:
        // a question about what else this click needs to reach, answered
        // before anything is written.
        //
        // Declining is an answer here, not an abort. The person may want
        // Hermes routed and their provider left alone, and that is a coherent
        // state - Hermes still goes through Gate, Gate still does not inspect
        // OpenRouter. So this is not wrapped in the `Declined` throw the drift
        // and trust gates use; it just skips the enable.
        let providerDomains: string[] = [];
        if (routed && slug === HERMES_SLUG) {
          const coverage = await hermesUpstreamCoverage().catch(() => null);
          const off = coverage?.switched_off ?? [];
          if (off.length > 0) {
            // The row's own display name, not the slug and not the host. It
            // is what the person will look for in the sidebar if they want to
            // change their mind, and "OpenRouter" is how they think of the
            // thing anyway - `openrouter` is Gate's internal key and
            // `openrouter.ai` is an implementation detail of it. Falls back to
            // the host only if the catalog has no row to name, which cannot
            // happen for a `switched_off` entry (that state means a domain
            // claimed the host) but keeps the type honest.
            const domains = off.map(([host, domainSlug]) => ({
              name:
                proxy?.domains.find((d) => d.slug === domainSlug)
                  ?.display_name ?? host,
              host,
              slug: domainSlug,
            }));
            const yes = await askOptional({ kind: "hermes-provider", domains });
            if (yes) providerDomains = domains.map((d) => d.slug);
          }
        }

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
          // After the connect, which is what starts the engine. The channel
          // exports the engine's address, so switching it on ahead of a bound
          // port would write nothing and report success.
          //
          // `proxySetEnvExport` rather than `connectTool("env-proxy")`: the two
          // reach the same `proxy::set_env_export`, but the integration's
          // connect refuses without a live engine, and a refusal here would
          // fail an OpenCode connect that had already succeeded.
          if (couplesEnvExport) await proxySetEnvExport(true);
          // After the connect, like the channel above and for the same reason:
          // the engine has to be up. Each slug separately because
          // `proxy_set_domain` takes one, and a failure on the second must not
          // undo the first - a partially inspected provider set is still
          // better than none, and `settle` re-reads the truth either way.
          for (const domainSlug of providerDomains) {
            await proxySetDomain(domainSlug, true);
          }
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
        logWarn(`routing: ${slug} -> ${routed ? "on" : "off"} failed: ${describe(e)}`);
        // Declining a gate is an answer, not a failure: nothing was written and
        // there is nothing to report - and in particular the row must not be
        // marked failed, because the user chose this.
        if (!(e instanceof Declined)) {
          trackError(e, "connect", { tool: slug, routed });
          onError?.(e, routed ? "connect" : "disconnect");
          setWriteFailures((prev) => new Set(prev).add(slug));
        }
      } finally {
        await settle();
      }
      return changed;
    },
    [busy, tools, proxy, ask, ensureCaTrusted, settle, onError],
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
        await settle();
      }
      return changed;
    },
    [busy, ensureCaTrusted, resync, onError],
  );

  /**
   * Route or unroute one proxy domain. No drift gate: a domain has no
   * hand-written config to preserve, only an enabled flag.
   *
   * Reports whether the flag actually moved, like `setAppRouted` - false for a
   * declined gate and for a failure, both of which have already been handled
   * here. A section cascade needs it: it follows up on what moved, and it has no
   * other way to find out, because nothing in this hook throws to its caller.
   */
  const setDomainRouted = useCallback(
    async (slug: string, routed: boolean): Promise<boolean> => {
      if (busy) return false;
      setBusy(true);
      let changed = false;
      try {
        if (routed) {
          await ensureCaTrusted();
          await ensureEngineRunning();
        }
        await proxySetDomain(slug, routed);
        changed = true;
        track("domain_toggled", { domain: slug, routed });
      } catch (e) {
        if (e instanceof Declined) return false;
        trackError(e, "provider_toggle", { domain: slug, routed });
        onError?.(e, "domain");
      } finally {
        await settle();
      }
      return changed;
    },
    [busy, ensureCaTrusted, ensureEngineRunning, resync, onError],
  );

  /**
   * Turn the shell-environment channel on or off.
   *
   * Its own action rather than a branch of the engine's own lifecycle, for the
   * reason `App.tsx` gives: this never starts or stops the engine, it decides whether
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
        await settle();
      }
    },
    [busy, resync, onError],
  );

  /**
   * Remove the Gate certificate from the system trust store.
   *
   * Confirmed first: every routed domain stops being inspected the moment this
   * lands, and the engine keeps running, so the state it leaves behind is one
   * the user cannot read off a switch. The quit teardown deliberately does
   * *not* do this - the next launch's enable stays promptless - which is why it
   * is an explicit action of its own.
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
      await settle();
    }
  }, [busy, ask, settle, onError]);

  return {
    busy,
    prompt,
    resolvePrompt,
    confirmCaTrusted,
    setAppRouted,
    setFamilyRouted,
    setDomainRouted,
    setEnvExport,
    untrustCa,
    writeFailures,
  };
}
