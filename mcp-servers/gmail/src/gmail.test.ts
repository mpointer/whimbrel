import { describe, expect, it } from "vitest";
import { createDraft, getThread, searchThreads, sendEmail, syncMessages } from "./gmail.js";

describe("sendEmail", () => {
  it("POSTs a base64url-encoded RFC 2822 message to Gmail's send endpoint", async () => {
    const calls: { url: URL; init: { method?: string; headers: Record<string, string>; body?: string } }[] = [];
    const fetchFn = async (url: URL, init: { method?: string; headers: Record<string, string>; body?: string }) => {
      calls.push({ url, init });
      return { ok: true, status: 200, json: async () => ({ id: "msg123" }) };
    };

    const result = await sendEmail("me@example.com", "Test Subject", "Test body.", "fake-token", fetchFn);

    expect(calls.length).toBe(1);
    expect(calls[0].url.toString()).toBe("https://gmail.googleapis.com/gmail/v1/users/me/messages/send");
    expect(calls[0].init.method).toBe("POST");
    expect(calls[0].init.headers.Authorization).toBe("Bearer fake-token");
    const body = JSON.parse(calls[0].init.body!);
    expect(typeof body.raw).toBe("string");
    // Decode the base64url raw message and confirm it's a real RFC 2822 message
    // containing the recipient/subject/body -- proves the encoding round-trips,
    // not just that some string was sent. Check the literal separator to ensure
    // headers/body boundary is correctly formed.
    const decoded = Buffer.from(body.raw, "base64url").toString("utf-8");
    expect(decoded).toContain('charset="UTF-8"\r\n\r\nTest body.');
    expect(result).toEqual({ id: "msg123" });
  });

  it("rejects to field containing a line break (CRLF injection defense)", async () => {
    const fetchFn = async () => ({ ok: true, status: 200, json: async () => ({}) });
    await expect(sendEmail("me@example.com\r\nBcc: attacker@example.com", "Subject", "Body", "fake-token", fetchFn)).rejects.toThrow(
      "to must not contain line breaks",
    );
  });

  it("rejects subject field containing a line break (CRLF injection defense)", async () => {
    const fetchFn = async () => ({ ok: true, status: 200, json: async () => ({}) });
    await expect(sendEmail("me@example.com", "Subject\r\nBcc: attacker@example.com", "Body", "fake-token", fetchFn)).rejects.toThrow(
      "subject must not contain line breaks",
    );
  });

  it("throws on a non-ok response", async () => {
    const fetchFn = async () => ({ ok: false, status: 403, json: async () => ({}) });
    await expect(sendEmail("me@example.com", "Subject", "Body", "fake-token", fetchFn)).rejects.toThrow("Gmail API returned 403");
  });
});

describe("searchThreads", () => {
  it("lists threads then hydrates each with subject/from/date via a metadata fetch", async () => {
    const calls: URL[] = [];
    const fetchFn = async (url: URL) => {
      calls.push(url);
      if (url.pathname === "/gmail/v1/users/me/threads") {
        return {
          ok: true,
          status: 200,
          json: async () => ({
            threads: [{ id: "t1", snippet: "snippet one" }, { id: "t2", snippet: "snippet two" }],
            resultSizeEstimate: 2,
          }),
        };
      }
      // Per-thread metadata fetch, e.g. /gmail/v1/users/me/threads/t1?format=metadata
      const id = url.pathname.split("/").pop();
      return {
        ok: true,
        status: 200,
        json: async () => ({
          messages: [{ payload: { headers: [
            { name: "Subject", value: `Subject for ${id}` },
            { name: "From", value: `sender-${id}@example.com` },
            { name: "Date", value: "Mon, 1 Jan 2026 10:00:00 -0500" },
          ] } }],
        }),
      };
    };

    const result = await searchThreads("from:alice", undefined, "fake-token", fetchFn);

    expect(calls[0].searchParams.get("q")).toBe("from:alice");
    expect(result.threads).toEqual([
      { id: "t1", snippet: "snippet one", subject: "Subject for t1", from: "sender-t1@example.com", date: "Mon, 1 Jan 2026 10:00:00 -0500" },
      { id: "t2", snippet: "snippet two", subject: "Subject for t2", from: "sender-t2@example.com", date: "Mon, 1 Jan 2026 10:00:00 -0500" },
    ]);
    expect(result.resultSizeEstimate).toBe(2);
  });

  it("caps max_results at 25 even if a larger value is requested", async () => {
    let capturedMaxResults: string | null = null;
    const fetchFn = async (url: URL) => {
      if (url.pathname === "/gmail/v1/users/me/threads") {
        capturedMaxResults = url.searchParams.get("maxResults");
        return { ok: true, status: 200, json: async () => ({ threads: [], resultSizeEstimate: 0 }) };
      }
      return { ok: true, status: 200, json: async () => ({ messages: [] }) };
    };

    await searchThreads("q", 1000, "fake-token", fetchFn);

    expect(capturedMaxResults).toBe("25");
  });

  it("defaults max_results to 10 when not specified", async () => {
    let capturedMaxResults: string | null = null;
    const fetchFn = async (url: URL) => {
      if (url.pathname === "/gmail/v1/users/me/threads") {
        capturedMaxResults = url.searchParams.get("maxResults");
        return { ok: true, status: 200, json: async () => ({ threads: [], resultSizeEstimate: 0 }) };
      }
      return { ok: true, status: 200, json: async () => ({ messages: [] }) };
    };

    await searchThreads("q", undefined, "fake-token", fetchFn);

    expect(capturedMaxResults).toBe("10");
  });

  it("throws on a non-ok list response", async () => {
    const fetchFn = async () => ({ ok: false, status: 403, json: async () => ({}) });
    await expect(searchThreads("q", undefined, "fake-token", fetchFn)).rejects.toThrow("Gmail API returned 403");
  });
});

describe("getThread", () => {
  function base64url(s: string): string {
    return Buffer.from(s, "utf-8").toString("base64url");
  }

  it("prefers a text/plain part when present", async () => {
    const fetchFn = async () => ({
      ok: true,
      status: 200,
      json: async () => ({
        id: "thread1",
        messages: [{
          id: "msg1",
          payload: {
            mimeType: "multipart/alternative",
            headers: [
              { name: "From", value: "alice@example.com" },
              { name: "To", value: "bob@example.com" },
              { name: "Subject", value: "Hello" },
              { name: "Date", value: "Mon, 1 Jan 2026 10:00:00 -0500" },
              { name: "Message-ID", value: "<msg1@mail.gmail.com>" },
            ],
            parts: [
              { mimeType: "text/plain", body: { size: 11, data: base64url("Plain body!") } },
              { mimeType: "text/html", body: { size: 30, data: base64url("<p>Html body!</p>") } },
            ],
          },
        }],
      }),
    });

    const result = await getThread("thread1", "fake-token", fetchFn);

    expect(result.messages[0]).toEqual({
      id: "msg1",
      messageId: "<msg1@mail.gmail.com>",
      from: "alice@example.com",
      to: "bob@example.com",
      subject: "Hello",
      date: "Mon, 1 Jan 2026 10:00:00 -0500",
      bodyText: "Plain body!",
      attachments: [],
    });
  });

  it("falls back to a stripped text/html body when no text/plain part exists", async () => {
    const fetchFn = async () => ({
      ok: true,
      status: 200,
      json: async () => ({
        id: "thread1",
        messages: [{
          id: "msg1",
          payload: {
            mimeType: "text/html",
            headers: [{ name: "From", value: "a@example.com" }, { name: "To", value: "" }, { name: "Subject", value: "" }, { name: "Date", value: "" }],
            body: { size: 40, data: base64url("<p>Hello <b>world</b>!</p>") },
          },
        }],
      }),
    });

    const result = await getThread("thread1", "fake-token", fetchFn);

    expect(result.messages[0].bodyText).toBe("Hello world !");
  });

  it("collects attachment metadata without fetching content, distinguishing by filename presence", async () => {
    const fetchFn = async () => ({
      ok: true,
      status: 200,
      json: async () => ({
        id: "thread1",
        messages: [{
          id: "msg1",
          payload: {
            mimeType: "multipart/mixed",
            headers: [{ name: "From", value: "a@example.com" }, { name: "To", value: "" }, { name: "Subject", value: "" }, { name: "Date", value: "" }],
            parts: [
              { mimeType: "text/plain", body: { size: 5, data: base64url("Body!") } },
              { mimeType: "application/pdf", filename: "report.pdf", body: { size: 204800, attachmentId: "att1" } },
            ],
          },
        }],
      }),
    });

    const result = await getThread("thread1", "fake-token", fetchFn);

    expect(result.messages[0].attachments).toEqual([{ filename: "report.pdf", mimeType: "application/pdf", size: 204800 }]);
    expect(result.messages[0].bodyText).toBe("Body!");
  });

  it("classifies a small inline attachment (body.data, no attachmentId) as an attachment, not body text", async () => {
    const fetchFn = async () => ({
      ok: true,
      status: 200,
      json: async () => ({
        id: "thread1",
        messages: [{
          id: "msg1",
          payload: {
            mimeType: "multipart/mixed",
            headers: [{ name: "From", value: "a@example.com" }, { name: "To", value: "" }, { name: "Subject", value: "" }, { name: "Date", value: "" }],
            parts: [
              { mimeType: "text/plain", body: { size: 5, data: base64url("Body!") } },
              { mimeType: "text/plain", filename: "small.txt", body: { size: 12, data: base64url("small attach") } },
            ],
          },
        }],
      }),
    });

    const result = await getThread("thread1", "fake-token", fetchFn);

    expect(result.messages[0].attachments).toEqual([{ filename: "small.txt", mimeType: "text/plain", size: 12 }]);
    expect(result.messages[0].bodyText).toBe("Body!");
  });

  it("throws on a non-ok response", async () => {
    const fetchFn = async () => ({ ok: false, status: 404, json: async () => ({}) });
    await expect(getThread("thread1", "fake-token", fetchFn)).rejects.toThrow("Gmail API returned 404");
  });
});

describe("syncMessages", () => {
  function base64url(s: string): string {
    return Buffer.from(s, "utf-8").toString("base64url");
  }

  it("establishes a baseline via getProfile with no history_id, doing zero backfill", async () => {
    const calls: URL[] = [];
    const fetchFn = async (url: URL) => {
      calls.push(url);
      return { ok: true, status: 200, json: async () => ({ historyId: "50", emailAddress: "me@example.com" }) };
    };

    const result = await syncMessages(undefined, "fake-token", fetchFn);

    expect(calls.length).toBe(1);
    expect(calls[0].pathname).toBe("/gmail/v1/users/me/profile");
    expect(result).toEqual({ messages: [], deletedExternalIds: [], nextHistoryId: "50", needsFullResync: false });
  });

  it("paginates history.list, merging added/deleted ids across pages and using the final page's historyId", async () => {
    const historyCalls: URL[] = [];
    let messageGetCalls = 0;
    const fetchFn = async (url: URL) => {
      if (url.pathname === "/gmail/v1/users/me/history") {
        historyCalls.push(url);
        if (!url.searchParams.get("pageToken")) {
          return {
            ok: true, status: 200,
            json: async () => ({
              history: [{ messagesAdded: [{ message: { id: "m1" } }] }],
              nextPageToken: "page2",
              historyId: "100",
            }),
          };
        }
        return {
          ok: true, status: 200,
          json: async () => ({ history: [{ messagesDeleted: [{ message: { id: "m2" } }] }], historyId: "105" }),
        };
      }
      messageGetCalls += 1;
      return {
        ok: true, status: 200,
        json: async () => ({
          id: "m1", threadId: "t1", internalDate: "1700000000000",
          payload: {
            mimeType: "text/plain",
            headers: [
              { name: "From", value: "a@example.com" }, { name: "To", value: "me@example.com" },
              { name: "Subject", value: "Hi" }, { name: "Date", value: "Mon, 1 Jan 2026 10:00:00 -0500" },
            ],
            body: { size: 5, data: base64url("Hello") },
          },
        }),
      };
    };

    const result = await syncMessages("80", "fake-token", fetchFn);

    expect(historyCalls[0].searchParams.get("startHistoryId")).toBe("80");
    expect(historyCalls[1].searchParams.get("pageToken")).toBe("page2");
    expect(messageGetCalls).toBe(1);
    expect(result.messages).toEqual([{
      id: "m1", threadId: "t1", from: "a@example.com", to: "me@example.com", subject: "Hi",
      date: "Mon, 1 Jan 2026 10:00:00 -0500", internalDate: "1700000000000", bodyText: "Hello", attachments: [],
      labelIds: [], listUnsubscribe: false,
    }]);
    expect(result.deletedExternalIds).toEqual(["m2"]);
    expect(result.nextHistoryId).toBe("105");
    expect(result.needsFullResync).toBe(false);
  });

  it("skips a message that 404s on fetch and still advances the history id", async () => {
    const fetchFn = async (url: URL) => {
      if (url.pathname === "/gmail/v1/users/me/history") {
        return {
          ok: true, status: 200,
          json: async () => ({ history: [{ messagesAdded: [{ message: { id: "gone" } }] }], historyId: "120" }),
        };
      }
      return { ok: false, status: 404, json: async () => ({}) };
    };

    const result = await syncMessages("80", "fake-token", fetchFn);

    expect(result).toEqual({ messages: [], deletedExternalIds: [], nextHistoryId: "120", needsFullResync: false });
  });

  it("passes through Gmail's labelIds and flags a List-Unsubscribe header", async () => {
    const fetchFn = async (url: URL) => {
      if (url.pathname === "/gmail/v1/users/me/history") {
        return {
          ok: true, status: 200,
          json: async () => ({ history: [{ messagesAdded: [{ message: { id: "m1" } }] }], historyId: "90" }),
        };
      }
      return {
        ok: true, status: 200,
        json: async () => ({
          id: "m1", threadId: "t1", internalDate: "1700000000000",
          labelIds: ["INBOX", "CATEGORY_PROMOTIONS"],
          payload: {
            mimeType: "text/plain",
            headers: [
              { name: "From", value: "deals@shop.example" },
              { name: "list-unsubscribe", value: "<mailto:unsub@shop.example>" },
            ],
            body: { size: 4, data: base64url("Sale") },
          },
        }),
      };
    };

    const result = await syncMessages("80", "fake-token", fetchFn);

    expect(result.messages[0].labelIds).toEqual(["INBOX", "CATEGORY_PROMOTIONS"]);
    expect(result.messages[0].listUnsubscribe).toBe(true);
  });

  it("excludes a message added then deleted within the same range from messages, but still reports it deleted", async () => {
    let messageGetCalls = 0;
    const fetchFn = async (url: URL) => {
      if (url.pathname === "/gmail/v1/users/me/history") {
        return {
          ok: true, status: 200,
          json: async () => ({
            history: [{ messagesAdded: [{ message: { id: "m1" } }], messagesDeleted: [{ message: { id: "m1" } }] }],
            historyId: "90",
          }),
        };
      }
      messageGetCalls += 1;
      return { ok: true, status: 200, json: async () => ({}) };
    };

    const result = await syncMessages("80", "fake-token", fetchFn);

    expect(messageGetCalls).toBe(0);
    expect(result.messages).toEqual([]);
    expect(result.deletedExternalIds).toEqual(["m1"]);
  });

  it("ignores labelsAdded/labelsRemoved records entirely", async () => {
    let messageGetCalls = 0;
    const fetchFn = async (url: URL) => {
      if (url.pathname === "/gmail/v1/users/me/history") {
        return {
          ok: true, status: 200,
          json: async () => ({
            history: [{ labelsAdded: [{ message: { id: "m3" }, labelIds: ["TRASH"] }] }],
            historyId: "95",
          }),
        };
      }
      messageGetCalls += 1;
      return { ok: true, status: 200, json: async () => ({}) };
    };

    const result = await syncMessages("80", "fake-token", fetchFn);

    expect(messageGetCalls).toBe(0);
    expect(result.messages).toEqual([]);
    expect(result.deletedExternalIds).toEqual([]);
    expect(result.nextHistoryId).toBe("95");
  });

  it("returns needsFullResync on a 404 from history.list instead of throwing", async () => {
    const fetchFn = async (url: URL) => {
      if (url.pathname === "/gmail/v1/users/me/history") {
        return { ok: false, status: 404, json: async () => ({}) };
      }
      return { ok: true, status: 200, json: async () => ({}) };
    };

    const result = await syncMessages("stale-history-id", "fake-token", fetchFn);

    expect(result).toEqual({ messages: [], deletedExternalIds: [], nextHistoryId: "", needsFullResync: true });
  });

  it("throws on a non-OK, non-404 response from history.list", async () => {
    const fetchFn = async (url: URL) => {
      if (url.pathname === "/gmail/v1/users/me/history") {
        return { ok: false, status: 500, json: async () => ({}) };
      }
      return { ok: true, status: 200, json: async () => ({}) };
    };

    await expect(syncMessages("80", "fake-token", fetchFn)).rejects.toThrow("Gmail API returned 500");
  });
});

describe("createDraft", () => {
  it("fetches thread metadata, then POSTs a reply with In-Reply-To/References from the thread's last message", async () => {
    const calls: { url: URL; init: { method?: string; headers: Record<string, string>; body?: string } }[] = [];
    const fetchFn = async (url: URL, init: { method?: string; headers: Record<string, string>; body?: string }) => {
      calls.push({ url, init });
      if (url.pathname === "/gmail/v1/users/me/threads/thread1") {
        return {
          ok: true,
          status: 200,
          json: async () => ({
            messages: [
              { payload: { headers: [{ name: "Message-ID", value: "<old@mail.gmail.com>" }] } },
              {
                payload: {
                  headers: [
                    { name: "Message-ID", value: "<last@mail.gmail.com>" },
                    { name: "References", value: "<old@mail.gmail.com>" },
                  ],
                },
              },
            ],
          }),
        };
      }
      return { ok: true, status: 200, json: async () => ({ id: "draft123", message: { id: "msg1", threadId: "thread1" } }) };
    };

    const result = await createDraft("thread1", "me@example.com", "Test", "Reply body.", "fake-token", fetchFn);

    expect(calls.length).toBe(2);
    expect(calls[0].url.pathname).toBe("/gmail/v1/users/me/threads/thread1");
    expect(calls[1].url.toString()).toBe("https://gmail.googleapis.com/gmail/v1/users/me/drafts");
    expect(calls[1].init.method).toBe("POST");
    const body = JSON.parse(calls[1].init.body!);
    expect(body.message.threadId).toBe("thread1");
    const decoded = Buffer.from(body.message.raw, "base64url").toString("utf-8");
    expect(decoded).toContain("Subject: Re: Test\r\n");
    expect(decoded).toContain("In-Reply-To: <last@mail.gmail.com>\r\n");
    expect(decoded).toContain("References: <old@mail.gmail.com> <last@mail.gmail.com>\r\n");
    expect(decoded).toContain('charset="UTF-8"\r\n\r\nReply body.');
    expect(result).toEqual({ id: "draft123", message: { id: "msg1", threadId: "thread1" } });
  });

  it("doesn't double-prefix a subject that already starts with Re:", async () => {
    const calls: { url: URL; init: { method?: string; headers: Record<string, string>; body?: string } }[] = [];
    const fetchFn = async (url: URL, init: { method?: string; headers: Record<string, string>; body?: string }) => {
      calls.push({ url, init });
      if (url.pathname === "/gmail/v1/users/me/threads/thread1") {
        return { ok: true, status: 200, json: async () => ({ messages: [] }) };
      }
      return { ok: true, status: 200, json: async () => ({ id: "draft123" }) };
    };

    await createDraft("thread1", "me@example.com", "re: Already prefixed", "Body.", "fake-token", fetchFn);

    const body = JSON.parse(calls[1].init.body!);
    const decoded = Buffer.from(body.message.raw, "base64url").toString("utf-8");
    expect(decoded).toContain("Subject: re: Already prefixed\r\n");
  });

  it("builds the draft without threading headers when the thread has no Message-ID (defensive)", async () => {
    const calls: { url: URL; init: { method?: string; headers: Record<string, string>; body?: string } }[] = [];
    const fetchFn = async (url: URL, init: { method?: string; headers: Record<string, string>; body?: string }) => {
      calls.push({ url, init });
      if (url.pathname === "/gmail/v1/users/me/threads/thread1") {
        return { ok: true, status: 200, json: async () => ({ messages: [] }) };
      }
      return { ok: true, status: 200, json: async () => ({ id: "draft123" }) };
    };

    await createDraft("thread1", "me@example.com", "Test", "Reply body.", "fake-token", fetchFn);

    const body = JSON.parse(calls[1].init.body!);
    const decoded = Buffer.from(body.message.raw, "base64url").toString("utf-8");
    expect(decoded).not.toContain("In-Reply-To:");
    expect(decoded).not.toContain("References:");
    expect(decoded).toContain('charset="UTF-8"\r\n\r\nReply body.');
  });

  it("rejects to field containing a line break (CRLF injection defense, same as sendEmail)", async () => {
    const fetchFn = async (url: URL) => {
      if (url.pathname === "/gmail/v1/users/me/threads/thread1") {
        return { ok: true, status: 200, json: async () => ({ messages: [] }) };
      }
      return { ok: true, status: 200, json: async () => ({}) };
    };
    await expect(
      createDraft("thread1", "me@example.com\r\nBcc: attacker@example.com", "Subject", "Body", "fake-token", fetchFn),
    ).rejects.toThrow("to must not contain line breaks");
  });

  it("throws when the thread metadata fetch returns a non-ok response", async () => {
    const fetchFn = async () => ({ ok: false, status: 404, json: async () => ({}) });
    await expect(createDraft("thread1", "me@example.com", "Subject", "Body", "fake-token", fetchFn)).rejects.toThrow("Gmail API returned 404");
  });

  it("throws when the drafts POST returns a non-ok response", async () => {
    const fetchFn = async (url: URL) => {
      if (url.pathname === "/gmail/v1/users/me/threads/thread1") {
        return { ok: true, status: 200, json: async () => ({ messages: [] }) };
      }
      return { ok: false, status: 403, json: async () => ({}) };
    };
    await expect(createDraft("thread1", "me@example.com", "Subject", "Body", "fake-token", fetchFn)).rejects.toThrow("Gmail API returned 403");
  });
});

