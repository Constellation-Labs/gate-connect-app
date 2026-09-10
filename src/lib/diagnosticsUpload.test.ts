import { describe, it, expect, vi, beforeEach } from "vitest";
import {
  canSendDiagnostics,
  newDiagnosticReference,
  sendDiagnosticReport,
} from "./diagnosticsUpload";
import { analyticsId } from "./analytics";
import { installId } from "./api";

// What this pins is the criterion the module exists for: Send uploads one
// report whether automatic collection is On or Off. Everything else here is in
// service of that - the report leaves verbatim, the reference is readable, and
// a refusal surfaces rather than being swallowed the way `track` swallows one.

vi.mock("./config", async (importOriginal) => ({
  ...(await importOriginal<typeof import("./config")>()),
  POSTHOG_KEY_VALUE: "phc_test",
  POSTHOG_HOST: "https://example.invalid",
}));

vi.mock("./analytics", () => ({
  analyticsId: vi.fn(() => ({ kind: "disabled" as const })),
}));

vi.mock("./api", () => ({
  installId: vi.fn(async () => "install-abc"),
}));

function okResponse() {
  return { ok: true, status: 200 } as Response;
}

/** The single event the capture call was given, parsed back out. */
function sentBody(): {
  api_key: string;
  event: string;
  distinct_id: string;
  properties: Record<string, unknown>;
} {
  const fetchMock = vi.mocked(globalThis.fetch);
  expect(fetchMock).toHaveBeenCalledTimes(1);
  const [, init] = fetchMock.mock.calls[0];
  return JSON.parse(String(init?.body));
}

beforeEach(() => {
  vi.clearAllMocks();
  vi.mocked(analyticsId).mockReturnValue({ kind: "disabled" });
  vi.mocked(installId).mockResolvedValue("install-abc");
  globalThis.fetch = vi.fn(async () => okResponse()) as typeof fetch;
});

describe("newDiagnosticReference", () => {
  it("is shaped to be read aloud and typed back in", () => {
    for (let i = 0; i < 200; i++) {
      expect(newDiagnosticReference()).toMatch(
        /^GC-[0-9A-HJKMNP-TV-Z]{4}-[0-9A-HJKMNP-TV-Z]{4}$/,
      );
    }
  });

  it("never mints the four characters that come back wrong", () => {
    // I/L/O/U are excluded on purpose: someone who did not generate the
    // reference reads it into a support form, and these are the ones that
    // arrive as 1, 1, 0 and V.
    const many = Array.from({ length: 400 }, () => newDiagnosticReference()).join("");
    expect(many).not.toMatch(/[ILOU]/);
  });

  it("does not repeat itself", () => {
    const seen = new Set(Array.from({ length: 500 }, () => newDiagnosticReference()));
    expect(seen.size).toBe(500);
  });
});

describe("sendDiagnosticReport", () => {
  it("sends while automatic collection is opted OUT", async () => {
    // The criterion, and the whole reason this does not go through posthog-js:
    // `analyticsId` reporting `disabled` is an install that opted out, and it
    // is precisely the install a support thread is about to ask for a report.
    vi.mocked(analyticsId).mockReturnValue({ kind: "disabled" });

    const reference = await sendDiagnosticReport("[system]\nos  Ubuntu");

    expect(reference).toMatch(/^GC-/);
    expect(sentBody().properties.report).toBe("[system]\nos  Ubuntu");
  });

  it("files the report under the reference it returns", async () => {
    const reference = await sendDiagnosticReport("report");
    expect(sentBody().properties.diagnostic_reference).toBe(reference);
  });

  it("sends the report verbatim, not a quieter version of it", async () => {
    // The View report dialog, the Copy report button and this send must all
    // hand over the same text, or the screen that says what leaves the machine
    // is describing something else.
    const report = "[account]\nemail  someone@example.com\ndata dir  /home/u/.gate";
    await sendDiagnosticReport(report);
    expect(sentBody().properties.report).toBe(report);
  });

  it("joins the event stream when analytics is live", async () => {
    vi.mocked(analyticsId).mockReturnValue({ kind: "id", value: "device-id" });
    await sendDiagnosticReport("report");
    expect(sentBody().distinct_id).toBe("device-id");
    expect(installId).not.toHaveBeenCalled();
  });

  it("falls back to the install id when there is no analytics id", async () => {
    await sendDiagnosticReport("report");
    expect(sentBody().distinct_id).toBe("install-abc");
  });

  it("still sends when neither id can be read", async () => {
    vi.mocked(installId).mockRejectedValue(new Error("ipc down"));
    const reference = await sendDiagnosticReport("report");
    expect(sentBody().distinct_id).toBe(reference);
  });

  it("does not create a person profile for an install that never identified", async () => {
    await sendDiagnosticReport("report");
    expect(sentBody().properties.$process_person_profile).toBe(false);
  });

  it("throws on a refusal rather than swallowing it", async () => {
    globalThis.fetch = vi.fn(async () => ({ ok: false, status: 413 }) as Response) as typeof fetch;
    await expect(sendDiagnosticReport("report")).rejects.toThrow("413");
  });

  it("propagates a network failure", async () => {
    globalThis.fetch = vi.fn(async () => {
      throw new Error("Failed to fetch");
    }) as typeof fetch;
    await expect(sendDiagnosticReport("report")).rejects.toThrow("Failed to fetch");
  });
});

describe("canSendDiagnostics", () => {
  it("is true where a key is configured", () => {
    expect(canSendDiagnostics()).toBe(true);
  });
});
