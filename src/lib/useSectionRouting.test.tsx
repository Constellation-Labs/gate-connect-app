import { describe, it, expect, vi } from "vitest";
import { renderHook } from "@testing-library/react";
import { useSectionRouting } from "./useSectionRouting";
import type { useRouting } from "./useRouting";
import type { useRunningApps } from "./useRunningApps";

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
