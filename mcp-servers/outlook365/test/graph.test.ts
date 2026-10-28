import { describe, expect, it, vi } from "vitest";
import { listEvents, readMail } from "../src/graph.js";

describe("listEvents", () => {
  it("calls Microsoft Graph's calendarview endpoint with the right URL and Bearer token", async () => {
    const fetchMock = vi.fn().mockResolvedValue({
      ok: true,
      json: async () => ({ value: [{ id: "evt-1", subject: "Standup" }] }),
    });

    const result = await listEvents("2026-07-29T00:00:00Z", "2026-07-30T00:00:00Z", "test-token", fetchMock);

    expect(fetchMock).toHaveBeenCalledTimes(1);
    const [url, options] = fetchMock.mock.calls[0];
    expect(url.toString()).toBe(
      "https://graph.microsoft.com/v1.0/me/calendarview?startDateTime=2026-07-29T00%3A00%3A00Z&endDateTime=2026-07-30T00%3A00%3A00Z&%24orderby=start%2FdateTime",
    );
    expect(options.headers.Authorization).toBe("Bearer test-token");
    expect(result).toEqual({ value: [{ id: "evt-1", subject: "Standup" }] });
  });

  it("throws when Graph returns a non-OK response", async () => {
    const fetchMock = vi.fn().mockResolvedValue({ ok: false, status: 401 });

    await expect(listEvents("2026-07-29T00:00:00Z", "2026-07-30T00:00:00Z", "bad-token", fetchMock)).rejects.toThrow(
      "Microsoft Graph calendarview returned 401",
    );
  });
});

describe("readMail", () => {
  it("calls Microsoft Graph's messages endpoint with the right URL, Bearer token, and default top", async () => {
    const fetchMock = vi.fn().mockResolvedValue({
      ok: true,
      json: async () => ({ value: [{ id: "msg-1", subject: "Hello" }] }),
    });

    const result = await readMail("test-token", fetchMock);

    expect(fetchMock).toHaveBeenCalledTimes(1);
    const [url, options] = fetchMock.mock.calls[0];
    expect(url.toString()).toBe(
      "https://graph.microsoft.com/v1.0/me/messages?%24top=20&%24orderby=receivedDateTime+desc&%24select=subject%2Cfrom%2CreceivedDateTime%2CbodyPreview",
    );
    expect(options.headers.Authorization).toBe("Bearer test-token");
    expect(result).toEqual({ value: [{ id: "msg-1", subject: "Hello" }] });
  });

  it("respects a custom top", async () => {
    const fetchMock = vi.fn().mockResolvedValue({ ok: true, json: async () => ({ value: [] }) });

    await readMail("test-token", fetchMock, 5);

    const [url] = fetchMock.mock.calls[0];
    expect(url.toString()).toContain("%24top=5");
  });

  it("throws when Graph returns a non-OK response", async () => {
    const fetchMock = vi.fn().mockResolvedValue({ ok: false, status: 403 });

    await expect(readMail("bad-token", fetchMock)).rejects.toThrow("Microsoft Graph messages returned 403");
  });
});
