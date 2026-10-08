import { describe, it, expect, vi } from "vitest";
import { renderHook } from "@testing-library/react";
import { useSectionRouting } from "./useSectionRouting";
import type { useRouting } from "./useRouting";
import type { useRunningApps } from "./useRunningApps";
import type { GroupMember } from "./groups";

vi.mock("./log", () => ({ logInfo: vi.fn(), logWarn: vi.fn(), logError: vi.fn() }));

/**
 * The two branches of `toggle` past the section lookup.
 *
 * Both were unpinned at the call site: the `isDeclaredSection` tests cover the
 * predicate, and the e2e never clicks an orphaned switch because the pane
 * redirect gets there first. So the guard was once the wrong predicate
 * (`sectionMemberKeys(...).length`, always true) and the per-tool fallback went
 * dead with 235 e2e tests green. Raised in review on #375.
 *
 * Neither branch reaches `routing` or `runningApps` - only a built section
 * does - so both are left empty.
 */
function toggleWith(routeApp: (slug: string, next: boolean) => void) {
  const { result } = renderHook(() =>
    useSectionRouting({
      groups: [],
      routing: {} as ReturnType<typeof useRouting>,
      runningApps: {} as ReturnType<typeof useRunningApps>,
      onBeforeRoute: () => {},
      routeApp,
    }),
  );
  return result.current.toggle;
}

describe("useSectionRouting toggle", () => {
  it("does not send a declared section id with no built group to the per-tool path", () => {
    // `openai-api` once its only domain is off: a section id, not a `ToolId`.
    const routeApp = vi.fn();
    toggleWith(routeApp)("openai-api", true);
    expect(routeApp).not.toHaveBeenCalled();
  });

  it("sends a member slug no section claims to the per-tool path", () => {
    // The row `buildGroups` synthesises for an unplaced member: its id IS the
    // tool slug, so the per-tool path is the right one.
    const routeApp = vi.fn();
    toggleWith(routeApp)("acme-router", true);
    expect(routeApp).toHaveBeenCalledWith("acme-router", true);
  });
});

/**
 * `routeMembers`, the partly protected card's way in. Its body is the one
 * `routeSection` always ran, so these pin what the card relies on rather than
 * re-deriving the cascade: the clear, the empty list, the certificate gate and
 * the one-at-a-time guard.
 */
describe("useSectionRouting routeMembers", () => {
  const member = (key: string, kind: "config" | "proxy") =>
    ({ key, kind }) as GroupMember;

  function setup({ trusted = true }: { trusted?: boolean } = {}) {
    let release: () => void = () => {};
    const routing = {
      confirmCaTrusted: vi.fn(async () => trusted),
      runCascade: vi.fn(async (body: () => Promise<void>) => {
        await new Promise<void>((r) => (release = r));
        await body();
      }),
      setDomainRouted: vi.fn(async () => true),
      setAppRouted: vi.fn(async () => true),
    };
    const runningApps = { offerAfterChange: vi.fn(async () => {}) };
    const onBeforeRoute = vi.fn();
    const { result } = renderHook(() =>
      useSectionRouting({
        groups: [],
        routing: routing as unknown as ReturnType<typeof useRouting>,
        runningApps: runningApps as unknown as ReturnType<
          typeof useRunningApps
        >,
        onBeforeRoute,
        routeApp: () => {},
      }),
    );
    return {
      route: result.current.routeMembers,
      routing,
      runningApps,
      onBeforeRoute,
      release: () => release(),
    };
  }

  it("clears the last failure, and writes each member through its own path", async () => {
    const h = setup();
    const done = h.route(
      [member("claude-code", "config"), member("anthropic", "proxy")],
      true,
    );
    expect(h.onBeforeRoute).toHaveBeenCalledTimes(1);
    await vi.waitFor(() => expect(h.routing.runCascade).toHaveBeenCalled());
    h.release();
    await done;

    expect(h.routing.setAppRouted).toHaveBeenCalledWith("claude-code", true);
    expect(h.routing.setDomainRouted).toHaveBeenCalledWith("anthropic", true);
    expect(h.runningApps.offerAfterChange).toHaveBeenCalledWith([
      "claude-code",
      "anthropic",
    ]);
  });

  it("does nothing for an empty list", async () => {
    const h = setup();
    await h.route([], true);
    expect(h.routing.confirmCaTrusted).not.toHaveBeenCalled();
    expect(h.routing.runCascade).not.toHaveBeenCalled();
  });

  it("writes nothing when the certificate prompt is declined", async () => {
    const h = setup({ trusted: false });
    await h.route([member("anthropic", "proxy")], true);
    expect(h.routing.runCascade).not.toHaveBeenCalled();
    expect(h.runningApps.offerAfterChange).not.toHaveBeenCalled();
  });

  it("ignores a second click while the first is still writing", async () => {
    const h = setup();
    const first = h.route([member("anthropic", "proxy")], true);
    await vi.waitFor(() => expect(h.routing.runCascade).toHaveBeenCalled());
    await h.route([member("anthropic", "proxy")], true);
    h.release();
    await first;

    expect(h.routing.runCascade).toHaveBeenCalledTimes(1);
    expect(h.routing.setDomainRouted).toHaveBeenCalledTimes(1);
  });
});
