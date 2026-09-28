import { beforeEach, describe, expect, it, vi } from "vitest";
import { cleanup, renderHook, waitFor } from "@testing-library/react";

import { useInstallations, type Installation } from "./activity";
import { activityInstallations as activityInstallationsRaw } from "./api";

vi.mock("./api", () => ({
  activityInstallations: vi.fn(),
  activityOverview: vi.fn(),
}));

const activityInstallations = vi.mocked(activityInstallationsRaw);

/**
 * The installation read (AG-572 AC 1).
 *
 * The picker this also covered is gone - no frame drew it - but the read is not
 * decoration: `current` and `resolved` are what scope the App pane and the tray
 * to *this machine*, and `current === null` means two different things without
 * `resolved` to separate them.
 */
const INSTALL = "3f2b9c4e-7a1d-4f88-9d1e-0c5a6b7e8f90";
const OTHER = "9a8b7c6d-5e4f-4321-8765-0fedcba98765";

function installation(overrides: Partial<Installation>): Installation {
  return {
    installId: INSTALL,
    label: INSTALL,
    current: false,
    lastSeenAt: "2026-08-17T12:00:00.000Z",
    requests: 42,
    ...overrides,
  };
}

describe("useInstallations", () => {
  beforeEach(() => {
    cleanup();
    vi.clearAllMocks();
  });

  it("does not read until it is enabled", () => {
    renderHook(() => useInstallations(false, "oauth|https://gw|org-1"));

    expect(activityInstallations).not.toHaveBeenCalled();
  });

  it("re-reads and drops the old list when the credential changes", async () => {
    activityInstallations.mockResolvedValue(
      JSON.stringify({ installations: [installation({}), installation({ installId: OTHER })] }),
    );
    const { result, rerender } = renderHook(
      ({ credential }: { credential: string }) => useInstallations(true, credential),
      { initialProps: { credential: "oauth|https://gw|org-1" } },
    );
    await waitFor(() => expect(result.current.installations).toHaveLength(2));

    // The second org's read is still in flight here. The first org's machines
    // are already gone, because offering them would be offering a scope the new
    // reading cannot honour.
    activityInstallations.mockReturnValue(new Promise(() => {}));
    rerender({ credential: "oauth|https://gw|org-2" });

    expect(result.current.installations).toEqual([]);
    expect(activityInstallations).toHaveBeenCalledTimes(2);
  });

  it("tells a failed read apart from an answer that does not name this machine", async () => {
    // Both end resolved with `current` null. Only `failure` separates "the
    // gateway does not know this machine" from "we could not ask", and the app
    // pane draws an empty state for the first and "couldn't be read" for the
    // second.
    activityInstallations.mockResolvedValue(JSON.stringify({ installations: [], current: null }));
    const answered = renderHook(() => useInstallations(true, "oauth|https://gw|org-1"));
    await waitFor(() => expect(answered.result.current.resolved).toBe(true));
    expect(answered.result.current.current).toBeNull();
    expect(answered.result.current.failure).toBeNull();

    activityInstallations.mockRejectedValue('{"code":"offline","message":"no route to host"}');
    const failed = renderHook(() => useInstallations(true, "oauth|https://gw|org-1"));
    await waitFor(() => expect(failed.result.current.resolved).toBe(true));
    expect(failed.result.current.current).toBeNull();
    expect(failed.result.current.failure).toEqual({ code: "offline", message: "no route to host" });
  });

  it("reload re-reads and keeps the held answer until the new one lands", async () => {
    activityInstallations.mockRejectedValue('{"code":"offline","message":"down"}');
    const { result } = renderHook(() => useInstallations(true, "oauth|https://gw|org-1"));
    await waitFor(() => expect(result.current.failure?.code).toBe("offline"));

    // Held, not cleared: a retry that is still in flight must not flip the pane
    // back to "not asked yet" and a skeleton.
    let answer!: (text: string) => void;
    activityInstallations.mockReturnValue(new Promise((resolve) => (answer = resolve)));
    result.current.reload();
    await waitFor(() => expect(activityInstallations).toHaveBeenCalledTimes(2));
    expect(result.current.resolved).toBe(true);
    expect(result.current.failure?.code).toBe("offline");

    answer(JSON.stringify({ installations: [installation({ current: true })], current: INSTALL }));
    await waitFor(() => expect(result.current.current).toBe(INSTALL));
    expect(result.current.failure).toBeNull();
  });
});
