import { describe, expect, it, vi } from "vitest";
import { createEvent, googleErrorReason, listEvents, syncEvents } from "../src/calendar.js";

describe("listEvents", () => {
  it("calls Google's Calendar API with the right URL and Bearer token", async () => {
    const fetchMock = vi.fn().mockResolvedValue({
      ok: true,
      json: async () => ({ items: [{ id: "evt-1", summary: "Standup" }] }),
    });

    const result = await listEvents("2026-07-29T00:00:00Z", "2026-07-30T00:00:00Z", "test-token", fetchMock);

    expect(fetchMock).toHaveBeenCalledTimes(1);
    const [url, options] = fetchMock.mock.calls[0];
    expect(url.toString()).toBe(
      "https://www.googleapis.com/calendar/v3/calendars/primary/events?timeMin=2026-07-29T00%3A00%3A00Z&timeMax=2026-07-30T00%3A00%3A00Z&singleEvents=true&orderBy=startTime",
    );
    expect(options.headers.Authorization).toBe("Bearer test-token");
    expect(result).toEqual({ items: [{ id: "evt-1", summary: "Standup" }] });
  });

  it("throws when Google's API returns a non-OK response", async () => {
    const fetchMock = vi.fn().mockResolvedValue({ ok: false, status: 401 });

    await expect(listEvents("2026-07-29T00:00:00Z", "2026-07-30T00:00:00Z", "bad-token", fetchMock)).rejects.toThrow(
      "Google Calendar API returned 401",
    );
  });
});

describe("createEvent", () => {
  it("POSTs to the events endpoint and returns the created event", async () => {
    const fetchMock = vi.fn().mockResolvedValue({
      ok: true,
      json: async () => ({ id: "abc123", summary: "Focus Time" }),
    });

    const result = await createEvent(
      "Focus Time",
      "2026-08-03T09:00:00-04:00",
      "2026-08-03T10:00:00-04:00",
      "fake-token",
      fetchMock,
    );

    expect(fetchMock).toHaveBeenCalledTimes(1);
    const [url, options] = fetchMock.mock.calls[0];
    expect(url.toString()).toBe("https://www.googleapis.com/calendar/v3/calendars/primary/events");
    expect(options.method).toBe("POST");
    expect(options.headers.Authorization).toBe("Bearer fake-token");
    const body = JSON.parse(options.body);
    expect(body.summary).toBe("Focus Time");
    expect(body.start.dateTime).toBe("2026-08-03T09:00:00-04:00");
    expect(body.end.dateTime).toBe("2026-08-03T10:00:00-04:00");
    expect(result).toEqual({ id: "abc123", summary: "Focus Time" });
  });

  it("throws when Google's API returns a non-OK response", async () => {
    const fetchMock = vi.fn().mockResolvedValue({ ok: false, status: 403 });

    await expect(
      createEvent("Focus Time", "2026-08-03T09:00:00-04:00", "2026-08-03T10:00:00-04:00", "fake-token", fetchMock),
    ).rejects.toThrow("Google Calendar API returned 403");
  });
});

describe("syncEvents", () => {
  it("does a full baseline fetch with showDeleted, singleEvents, and a recent timeMin when no token is given", async () => {
    const fetchMock = vi.fn().mockResolvedValue({
      ok: true,
      status: 200,
      json: async () => ({ items: [], nextSyncToken: "token-abc" }),
    });
    const result = await syncEvents(undefined, "access-token", fetchMock);
    expect(result).toEqual({ items: [], nextSyncToken: "token-abc" });
    const calledUrl = fetchMock.mock.calls[0][0] as URL;
    expect(calledUrl.searchParams.get("showDeleted")).toBe("true");
    expect(calledUrl.searchParams.get("singleEvents")).toBe("true");
    expect(calledUrl.searchParams.get("syncToken")).toBeNull();
    expect(calledUrl.searchParams.get("timeMin")).not.toBeNull();
    const calledInit = fetchMock.mock.calls[0][1] as { headers: Record<string, string> };
    expect(calledInit.headers.Authorization).toBe("Bearer access-token");
  });

  it("passes the sync token, keeps singleEvents, and omits showDeleted/timeMin when a token is given", async () => {
    const fetchMock = vi.fn().mockResolvedValue({
      ok: true,
      status: 200,
      json: async () => ({ items: [{ id: "evt-1", status: "confirmed" }], nextSyncToken: "token-def" }),
    });
    const result = await syncEvents("token-abc", "access-token", fetchMock);
    expect(result).toEqual({ items: [{ id: "evt-1", status: "confirmed" }], nextSyncToken: "token-def" });
    const calledUrl = fetchMock.mock.calls[0][0] as URL;
    expect(calledUrl.searchParams.get("syncToken")).toBe("token-abc");
    // Google requires the same singleEvents value on every call of a sync sequence.
    expect(calledUrl.searchParams.get("singleEvents")).toBe("true");
    expect(calledUrl.searchParams.get("showDeleted")).toBeNull();
    expect(calledUrl.searchParams.get("timeMin")).toBeNull();
  });

  it("pages through the baseline fetch until the last page and returns its nextSyncToken", async () => {
    const fetchMock = vi
      .fn()
      .mockResolvedValueOnce({
        ok: true,
        status: 200,
        json: async () => ({ items: [{ id: "evt-1" }], nextPageToken: "page-2" }),
      })
      .mockResolvedValueOnce({
        ok: true,
        status: 200,
        json: async () => ({ items: [{ id: "evt-2" }], nextSyncToken: "token-final" }),
      });

    const result = await syncEvents(undefined, "access-token", fetchMock);

    expect(fetchMock).toHaveBeenCalledTimes(2);
    const firstUrl = fetchMock.mock.calls[0][0] as URL;
    expect(firstUrl.searchParams.get("pageToken")).toBeNull();
    const secondUrl = fetchMock.mock.calls[1][0] as URL;
    expect(secondUrl.searchParams.get("pageToken")).toBe("page-2");
    expect(secondUrl.searchParams.get("singleEvents")).toBe("true");
    expect(secondUrl.searchParams.get("showDeleted")).toBe("true");
    expect(result).toEqual({ items: [{ id: "evt-1" }, { id: "evt-2" }], nextSyncToken: "token-final" });
  });

  it("returns needsFullResync on a 410 response instead of throwing", async () => {
    const fetchMock = vi.fn().mockResolvedValue({ ok: false, status: 410, json: async () => ({}) });
    const result = await syncEvents("stale-token", "access-token", fetchMock);
    expect(result).toEqual({ needsFullResync: true });
  });

  it("throws on a non-OK, non-410 response", async () => {
    const fetchMock = vi.fn().mockResolvedValue({ ok: false, status: 500, json: async () => ({}) });
    await expect(syncEvents(undefined, "access-token", fetchMock)).rejects.toThrow("Google Calendar API returned 500");
  });
});

// Google's real 403 bodies, trimmed to the fields the connector reads.
const scopeInsufficientBody = JSON.stringify({
  error: {
    code: 403,
    message: "Request had insufficient authentication scopes.",
    errors: [{ message: "Insufficient Permission", domain: "global", reason: "insufficientPermissions" }],
    status: "PERMISSION_DENIED",
    details: [{ "@type": "type.googleapis.com/google.rpc.ErrorInfo", reason: "ACCESS_TOKEN_SCOPE_INSUFFICIENT" }],
  },
});
const accessNotConfiguredBody = JSON.stringify({
  error: {
    code: 403,
    message: "Google Calendar API has not been used in project 123 before or it is disabled.",
    errors: [{ message: "Access Not Configured.", domain: "usageLimits", reason: "accessNotConfigured" }],
    status: "PERMISSION_DENIED",
  },
});

function errorResponse(status: number, body: string) {
  return { ok: false, status, json: async () => JSON.parse(body), text: async () => body };
}

describe("non-OK responses carry Google's reason", () => {
  it("appends insufficientPermissions for a 403 scope error, on every call", async () => {
    const fetchMock = vi.fn().mockResolvedValue(errorResponse(403, scopeInsufficientBody));
    const expected = "Google Calendar API returned 403 (insufficientPermissions)";
    await expect(syncEvents(undefined, "secret-token", fetchMock)).rejects.toThrow(expected);
    await expect(syncEvents("tok", "secret-token", fetchMock)).rejects.toThrow(expected);
    await expect(listEvents("a", "b", "secret-token", fetchMock)).rejects.toThrow(expected);
    await expect(createEvent("t", "a", "b", "secret-token", fetchMock)).rejects.toThrow(expected);
  });

  it("appends accessNotConfigured and never the message text or token", async () => {
    const fetchMock = vi.fn().mockResolvedValue(errorResponse(403, accessNotConfiguredBody));
    const error = await syncEvents(undefined, "secret-token", fetchMock).catch((e: Error) => e);
    expect((error as Error).message).toBe("Google Calendar API returned 403 (accessNotConfigured)");
  });

  it("keeps the bare message for a non-JSON body", async () => {
    const fetchMock = vi.fn().mockResolvedValue(errorResponse(403, "<html>Forbidden</html>"));
    const error = await syncEvents(undefined, "t", fetchMock).catch((e: Error) => e);
    expect((error as Error).message).toBe("Google Calendar API returned 403");
  });

  it("keeps the bare message for an empty body or a response with no body methods", async () => {
    const empty = vi.fn().mockResolvedValue(errorResponse(403, ""));
    expect(((await syncEvents(undefined, "t", empty).catch((e: Error) => e)) as Error).message).toBe(
      "Google Calendar API returned 403",
    );
    const bare = vi.fn().mockResolvedValue({ ok: false, status: 403 });
    expect(((await syncEvents(undefined, "t", bare).catch((e: Error) => e)) as Error).message).toBe(
      "Google Calendar API returned 403",
    );
  });

  it("reads a streamed body only up to the cap", async () => {
    const chunk = new TextEncoder().encode("x".repeat(8 * 1024));
    let pulls = 0;
    const body = new ReadableStream<Uint8Array>({
      pull(controller) {
        pulls += 1;
        controller.enqueue(chunk);
      },
    });
    const fetchMock = vi.fn().mockResolvedValue({ ok: false, status: 403, json: async () => ({}), body });
    const error = await syncEvents(undefined, "t", fetchMock).catch((e: Error) => e);
    expect((error as Error).message).toBe("Google Calendar API returned 403");
    expect(pulls).toBeLessThan(5);
  });

  it("reads a streamed JSON body", async () => {
    const body = new ReadableStream<Uint8Array>({
      start(controller) {
        controller.enqueue(new TextEncoder().encode(scopeInsufficientBody));
        controller.close();
      },
    });
    const fetchMock = vi.fn().mockResolvedValue({ ok: false, status: 403, json: async () => ({}), body });
    await expect(syncEvents(undefined, "t", fetchMock)).rejects.toThrow("Google Calendar API returned 403 (insufficientPermissions)");
  });
});

describe("googleErrorReason", () => {
  it("prefers errors[0].reason, then details[].reason, then status", () => {
    expect(googleErrorReason(scopeInsufficientBody)).toBe("insufficientPermissions");
    expect(googleErrorReason(JSON.stringify({ error: { status: "PERMISSION_DENIED", details: [{ reason: "ACCESS_TOKEN_SCOPE_INSUFFICIENT" }] } }))).toBe(
      "ACCESS_TOKEN_SCOPE_INSUFFICIENT",
    );
    expect(googleErrorReason(JSON.stringify({ error: { status: "PERMISSION_DENIED" } }))).toBe("PERMISSION_DENIED");
  });

  it("drops anything that isn't a short plain token", () => {
    expect(googleErrorReason(JSON.stringify({ error: { errors: [{ reason: "has spaces and a@b.com" }] } }))).toBeUndefined();
    expect(googleErrorReason(JSON.stringify({ error: { errors: [{ reason: "x".repeat(65) }] } }))).toBeUndefined();
    expect(googleErrorReason("null")).toBeUndefined();
    expect(googleErrorReason("[]")).toBeUndefined();
    expect(googleErrorReason(JSON.stringify({ error: "invalid_grant" }))).toBeUndefined();
  });
});
