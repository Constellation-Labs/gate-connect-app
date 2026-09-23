import { useCallback, useState } from "react";
import type { HermesCoverageEntry, ProxyState, Tool } from "./api";
import {
  connectTool,
  disconnectTool,
  listTools,
  proxyEnable,
  hermesUpstreamCoverage,
  proxySetDomain,
  recordAutoEnabledDomains,
  readAutoEnabledDomains,
  proxySetEnvExport,
  proxyStatus,
  proxyTrustCa,
  proxyUntrustCa,
} from "./api";
import { track, trackError } from "./analytics";
import { describe, logInfo, logWarn } from "./log";
import { TOOL_MANAGED_DOMAINS, cascadeTargets } from "./groups";
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
  | { kind: "trust" }
  /** Removing the certificate, which is not a gate on the way to something
   * else: it is the action, and it stops every routed domain. Confirmed for
   * the same reason the destructive dialogs are. */
  | { kind: "untrust" };

export interface RoutingSnapshot {
  tools: Tool[];
  proxy: ProxyState | null;
}

/**
 * One provider row the popover's Hermes notice asks about.
 *
 * The new shell no longer asks: its OpenRouter row is not drawn and Hermes'
 * switch owns the domain (`TOOL_MANAGED_DOMAINS`, AG-934's sibling decision on
 * 2026-09-23). The popover keeps its own `HermesProviderNotice` until it is
 * retired, which is why this type outlived `HermesProviderDialog`.
 *
 * Its answer is deliberately NOT recorded in `auto_enabled_domains`: somebody
 * who said yes to a question turned the domain on themselves, and the record
 * is for what Gate turned on without being asked. So a later Hermes-off in the
 * new shell leaves it alone, which is the rule working rather than a gap.
 */
export type HermesProviderChoice = { name: string; slug: string; tools: string[] };

/** The one tool whose switch also flips the shell-environment channel. Named
 *  once rather than spelled inline, because the dialog copy and the action have
 *  to be talking about the same row. */
const OPENCODE_SLUG = "opencode";

/** The tool whose provider lives in another section, so its switch alone
 *  routes it without inspecting anything. OpenClaw has the same shape and the
 *  same CLI-only coverage note; it is not wired here yet.
 *
 *  Exported because the popover connects tools through `App.tsx` rather than
 *  through this hook and has to gate the same row. One name, so the two shells
 *  cannot come to disagree about which row this is. */
export const HERMES_SLUG = "hermes";

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
   * Ask about the provider Hermes talks to, when Gate knows it and has it off.
   *
   * Resolves to the slugs to enable once Hermes is connected: empty when there
   * is nothing to ask, and rejecting with `Declined` on Cancel like every other
   * gate. Shared by the app switch and the family switch, because both connect
   * Hermes and a gate only one of them runs is the bug by another door.
   *
   * The command itself cannot fail: a missing or unparseable `config.yaml` is
   * reported as Hermes' default and marked `defaulted`, not returned as an
   * error. So a rejection here is the IPC failing, and the toggle goes ahead
   * without the question rather than costing the person the connect they asked
   * for - said in the log, because the state it leaves is the one this gate
   * exists to prevent, and a silent fall-through is how it went unnoticed for
   * an afternoon the first time.
   */
  const hermesProviderDomains = useCallback(async (): Promise<string[]> => {
    const coverage = await hermesUpstreamCoverage().catch((e: unknown) => {
      logWarn(`routing: hermes coverage read failed, connecting without it: ${describe(e)}`);
      return null;
    });
    if (coverage === null) return [];
    if (coverage.unknown.length > 0) {
      // No domain covers these, so nothing can be switched on for them. Logged
      // because it is the one place the window records that Hermes has traffic
      // Gate will never see, and the CLI has printed the same note since the
      // integration was written.
      logInfo(`routing: hermes upstreams Gate has no domain for: ${coverage.unknown.join(", ")}`);
    }
    const off = coverage.switched_off.map(({ slug }: HermesCoverageEntry) => slug);
    /**
     * Only the rows Hermes owns.
     *
     * `switched_off` is not only OpenRouter: a Hermes whose `config.yaml`
     * points at `api.anthropic.com` or `api.openai.com` comes back with
     * `anthropic` / `openai`, and those rows are still drawn and still belong
     * to the person. Turning one on silently would route it for every client
     * on the machine, and - worse - the record would then have Hermes' own OFF
     * switch end Claude's interception from a control that says nothing about
     * Claude.
     *
     * The decision that removed the dialog was about OpenRouter. So the
     * silent path is limited to `TOOL_MANAGED_DOMAINS`, which is exactly the
     * set with no row to manage it from, and the rest are left alone.
     */
    const mine = off.filter((slug) => TOOL_MANAGED_DOMAINS.includes(slug));
    const theirs = off.filter((slug) => !TOOL_MANAGED_DOMAINS.includes(slug));
    if (theirs.length > 0) {
      // Logged, like `unknown` above and for the same reason: this is the one
      // place the window records that Hermes has traffic Gate is not reading.
      // These at least have a row the person can turn on themselves, which is
      // why they are not routed for them.
      logInfo(
        `routing: hermes points at domains it does not own, left off: ${theirs.join(", ")}`,
      );
    }
    return mine;
  }, []);

  /**
   * Turn the provider domains on, after the connect and each on its own.
   *
   * After, because the engine has to be up and the connect is what starts it.
   * Each on its own because `proxy_set_domain` takes one, and a failure is
   * reported against the domain rather than thrown at the tool: by now Hermes'
   * config is written and on disk, so marking its row as a failed write would
   * say the opposite of what happened. The remaining slugs are still attempted -
   * a partially inspected provider set beats none - and `settle` re-reads the
   * truth either way.
   */
  const setProviderDomains = useCallback(
    async (slugs: string[], on: boolean): Promise<string[]> => {
      const done: string[] = [];
      for (const domainSlug of slugs) {
        try {
          await proxySetDomain(domainSlug, on);
          done.push(domainSlug);
        } catch (e) {
          const dir = on ? "on" : "off";
          logWarn(`routing: turning ${dir} ${domainSlug} for hermes failed: ${describe(e)}`);
          trackError(e, "provider_toggle", { domain: domainSlug, routed: on });
          onError?.(e, "domain");
        }
      }
      return done;
    },
    [onError],
  );

  /**
   * Turn Hermes' provider domains on and record that Gate did it.
   *
   * Only the ones that actually landed are recorded. Claiming authorship of a
   * domain whose enable failed would have the next disconnect switch off a row
   * Gate never switched on - which is the one thing the record exists to stop.
   */
  const enableProviderDomains = useCallback(
    async (slugs: string[]) => {
      const enabled = await setProviderDomains(slugs, true);
      // **Union with what is already recorded, not a replace.**
      //
      // `hermesProviderDomains` only returns domains that are currently OFF,
      // so a second connect over an already-routed Hermes enables nothing and
      // a replace would wipe the record - leaving the domain on with no row
      // and no way back. The concrete path is a drifted config taken through
      // "Replace config and protect": `switched_off` is empty, the record is
      // cleared, and the next Hermes-off has nothing to give back.
      //
      // Over-claiming a domain Hermes has stopped pointing at costs one
      // idempotent `proxy_set_domain` on the way out. Under-claiming strands
      // it for good. Swallowed for the same asymmetry: a failed record costs a
      // domain left on, which the CLI can still reach.
      const held = await readAutoEnabledDomains(HERMES_SLUG).catch((e: unknown) => {
        logWarn(`routing: reading hermes' auto-enabled domains failed: ${describe(e)}`);
        return [] as string[];
      });
      const next = [...new Set([...held, ...enabled])];
      await recordAutoEnabledDomains(HERMES_SLUG, next).catch((e: unknown) => {
        logWarn(`routing: recording hermes' auto-enabled domains failed: ${describe(e)}`);
      });
    },
    [setProviderDomains],
  );

  /**
   * The other half: switch off what the record held, and write back whatever
   * would not go.
   *
   * The record is read before the disconnect and rewritten after, rather than
   * taken up front. A take was the first shape and it strands: if the
   * disconnect or a disable then fails, the record is gone, the domain is
   * still on, and with no row drawn nothing can reach it - the next disconnect
   * reads an empty list. See `preferences::read_auto_enabled_domains`.
   */
  const disableProviderDomains = useCallback(
    async (slugs: string[]) => {
      if (slugs.length === 0) return;
      const disabled = await setProviderDomains(slugs, false);
      const left = slugs.filter((slug) => !disabled.includes(slug));
      await recordAutoEnabledDomains(HERMES_SLUG, left).catch((e: unknown) => {
        logWarn(`routing: recording hermes' auto-enabled domains failed: ${describe(e)}`);
      });
    },
    [setProviderDomains],
  );

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
        // What else this click has to reach, worked out before anything is
        // written. It used to be a question - `HermesProviderDialog`, a gate
        // like the drift and certificate ones - and is now simply done: the
        // OpenRouter row is no longer drawn, so the app owns the domain on
        // Hermes's behalf rather than sending the person to find it
        // (2026-09-23).
        //
        // Routing Hermes without it is the state this whole path exists to
        // prevent: Protected on every surface while the traffic tunnels past
        // unseen.
        const providerDomains =
          slug === HERMES_SLUG
            ? routed
              ? await hermesProviderDomains()
              : // Off: undo what Gate turned on for Hermes and nothing else. A
                // domain the person enabled themselves was never recorded, so
                // it is not in this list and stays on.
                await readAutoEnabledDomains(HERMES_SLUG).catch((e: unknown) => {
                  logWarn(`routing: reading hermes' auto-enabled domains failed: ${describe(e)}`);
                  return [] as string[];
                })
            : [];

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
          // After the connect, like the channel above and for the same reason.
          // Its failures are its own and never reach the catch below: Hermes
          // is connected by this line, whatever happens to its provider.
          await enableProviderDomains(providerDomains);
        } else {
          await disconnectTool(slug);
          // And take back the domains Gate turned on for it. Same treatment as
          // the enable: each on its own, failures logged rather than thrown,
          // because Hermes is disconnected by the line above whatever happens
          // to its provider.
          await disableProviderDomains(providerDomains);
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
    [
      busy,
      tools,
      proxy,
      ask,
      hermesProviderDomains,
      enableProviderDomains,
      ensureCaTrusted,
      settle,
      onError,
    ],
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
   *
   * Hermes is a one-member family, so its section switch is the same click as
   * its app switch by another control, and it does the same thing with its
   * provider domain: turns it on with the connect, gives it back on the way
   * out. There was a gate here - `HermesProviderDialog` - and this path
   * existed so the family switch was not a door around it; it is now the same
   * path for the same reason, one level up.
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
      // Whether the Hermes member's own write landed, in whichever direction.
      // Its provider domains are only moved behind a Hermes that actually
      // moved - and `routed` is not enough on its own, because the off
      // direction has to give those domains back and a boolean named for the
      // connect would be false exactly then.
      let hermesTouched = false;
      try {
        const hermesInSection = targets.some(
          (m) => m.kind === "config" && m.key === HERMES_SLUG,
        );
        // Same two directions as the single-app path, for the same reason: on
        // turns the provider domains on and records it, off gives back exactly
        // what was recorded. See `setAppRouted`.
        const providerDomains = !hermesInSection
          ? []
          : routed
            ? await hermesProviderDomains()
            : await readAutoEnabledDomains(HERMES_SLUG).catch((e: unknown) => {
                logWarn(`routing: reading hermes' auto-enabled domains failed: ${describe(e)}`);
                return [] as string[];
              });
        if (routed) await ensureCaTrusted();
        for (const member of targets) {
          try {
            if (member.kind === "config" && member.tool) {
              await (routed
                ? connectTool(member.key, member.tool.default_upstream_url)
                : disconnectTool(member.key));
              if (member.key === HERMES_SLUG) hermesTouched = true;
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
        // Only behind a Hermes that actually moved. A failed `disconnectTool`
        // leaves this false and the domains untouched - and, since the record
        // is only read here rather than taken, untouched means still recorded,
        // so the next attempt has something to give back. That was not true
        // while this consumed the record up front.
        if (hermesTouched) {
          if (routed) await enableProviderDomains(providerDomains);
          else await disableProviderDomains(providerDomains);
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
    [
      busy,
      hermesProviderDomains,
      enableProviderDomains,
      disableProviderDomains,
      ensureCaTrusted,
      resync,
      onError,
    ],
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
