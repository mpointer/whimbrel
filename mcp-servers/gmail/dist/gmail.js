/** Validates that a header field value contains no line breaks that could enable header injection.
 * Throws a clear error if \r or \n are found, preventing CRLF injection attacks. */
function assertNoHeaderInjection(value, fieldName) {
    if (/[\r\n]/.test(value)) {
        throw new Error(`${fieldName} must not contain line breaks`);
    }
}
/** Builds the base64url-encoded RFC 2822 message Gmail's `raw` field requires --
 * shared by `sendEmail` and `createDraft`, which differ only in which endpoint they
 * POST it to and whether a `threadId` accompanies it. `extraHeaders` lets a reply
 * draft carry `In-Reply-To`/`References` so Gmail threads it correctly; omitted by
 * `sendEmail`, whose output must stay byte-identical to before this parameter
 * existed. */
function buildRawMessage(to, subject, body, extraHeaders) {
    assertNoHeaderInjection(to, "to");
    assertNoHeaderInjection(subject, "subject");
    let headerLines = `To: ${to}\r\nSubject: ${subject}\r\n`;
    if (extraHeaders?.inReplyTo) {
        assertNoHeaderInjection(extraHeaders.inReplyTo, "in-reply-to");
        headerLines += `In-Reply-To: ${extraHeaders.inReplyTo}\r\n`;
    }
    if (extraHeaders?.references) {
        assertNoHeaderInjection(extraHeaders.references, "references");
        headerLines += `References: ${extraHeaders.references}\r\n`;
    }
    headerLines += `Content-Type: text/plain; charset="UTF-8"\r\n\r\n`;
    const message = headerLines + body;
    return Buffer.from(message, "utf-8").toString("base64url");
}
/** Sends an email via the Gmail API. Gmail's `messages.send` endpoint requires the
 * full RFC 2822 message base64url-encoded into a single `raw` field -- there's no
 * separate to/subject/body request shape. `fetchFn` defaults to the real global
 * `fetch` and is overridable for tests. */
export async function sendEmail(to, subject, body, accessToken, fetchFn = fetch) {
    const raw = buildRawMessage(to, subject, body);
    const url = new URL("https://gmail.googleapis.com/gmail/v1/users/me/messages/send");
    const response = await fetchFn(url, {
        method: "POST",
        headers: {
            Authorization: `Bearer ${accessToken}`,
            "Content-Type": "application/json",
        },
        body: JSON.stringify({ raw }),
    });
    if (!response.ok) {
        throw new Error(`Gmail API returned ${response.status}`);
    }
    return response.json();
}
const MAX_SEARCH_RESULTS = 25;
const DEFAULT_SEARCH_RESULTS = 10;
function getHeaderValue(headers, name) {
    return headers.find((h) => h.name.toLowerCase() === name.toLowerCase())?.value ?? "";
}
/** Searches Gmail threads using Gmail's own query syntax (`q`, e.g. "from:alice
 * newer_than:7d") -- no custom filter params, matching this codebase's thin-wrapper
 * convention. `threads.list` alone only returns `{id, snippet}` per thread, which
 * would be useless for triage without a subject/sender -- so this does a bounded
 * N+1: list, then a lightweight metadata fetch per result (capped at
 * MAX_SEARCH_RESULTS regardless of what's requested, to bound the extra API calls). */
export async function searchThreads(q, maxResults, accessToken, fetchFn = fetch) {
    const cappedMax = Math.min(maxResults ?? DEFAULT_SEARCH_RESULTS, MAX_SEARCH_RESULTS);
    const listUrl = new URL("https://gmail.googleapis.com/gmail/v1/users/me/threads");
    listUrl.searchParams.set("q", q);
    listUrl.searchParams.set("maxResults", String(cappedMax));
    const listResponse = await fetchFn(listUrl, { headers: { Authorization: `Bearer ${accessToken}` } });
    if (!listResponse.ok) {
        throw new Error(`Gmail API returned ${listResponse.status}`);
    }
    const listBody = (await listResponse.json());
    const rawThreads = listBody.threads ?? [];
    const threads = [];
    for (const t of rawThreads) {
        const metaUrl = new URL(`https://gmail.googleapis.com/gmail/v1/users/me/threads/${t.id}`);
        metaUrl.searchParams.set("format", "metadata");
        const metaResponse = await fetchFn(metaUrl, { headers: { Authorization: `Bearer ${accessToken}` } });
        if (!metaResponse.ok) {
            throw new Error(`Gmail API returned ${metaResponse.status}`);
        }
        const metaBody = (await metaResponse.json());
        const headers = metaBody.messages?.[0]?.payload?.headers ?? [];
        threads.push({
            id: t.id,
            snippet: t.snippet,
            subject: getHeaderValue(headers, "Subject"),
            from: getHeaderValue(headers, "From"),
            date: getHeaderValue(headers, "Date"),
        });
    }
    return { threads, resultSizeEstimate: listBody.resultSizeEstimate ?? 0 };
}
function decodeBase64Url(data) {
    return Buffer.from(data, "base64url").toString("utf-8");
}
/** Strips HTML down to plain text -- the fallback when a message has no text/plain
 * part. Deliberately simple (regex-based, not a real HTML parser): this only needs
 * to produce readable text for an LLM/human to read, not preserve structure. */
function stripHtml(html) {
    return html
        .replace(/<style[\s\S]*?<\/style>/gi, "")
        .replace(/<script[\s\S]*?<\/script>/gi, "")
        .replace(/<[^>]+>/g, " ")
        .replace(/&nbsp;/g, " ")
        .replace(/&amp;/g, "&")
        .replace(/&lt;/g, "<")
        .replace(/&gt;/g, ">")
        .replace(/&quot;/g, '"')
        .replace(/&#39;/g, "'")
        .replace(/\s+/g, " ")
        .trim();
}
/** Walks a MIME part tree collecting the best body text (preferring text/plain,
 * falling back to text/html) and every attachment's metadata -- never attachment
 * content. Recurses generically over `parts` since Gmail's actual multipart
 * structure varies by sending client; a single-part message has no `parts` array at
 * all, so this must also handle being called on just the top-level `payload` with no
 * children. An attachment part is detected by `part.filename` being present -- Gmail
 * always sets `filename` on attachment parts and never on plain body-text parts.
 * (Detecting via `body.attachmentId` instead is wrong: Gmail returns *small*
 * attachments with `body.data` populated directly and no `attachmentId`, which would
 * fall through into `bodyText`.) */
function walkParts(part, acc) {
    if (part.filename) {
        acc.attachments.push({ filename: part.filename, mimeType: part.mimeType, size: part.body?.size ?? 0 });
    }
    else if (part.mimeType === "text/plain" && part.body?.data && !acc.plainText) {
        acc.plainText = decodeBase64Url(part.body.data);
    }
    else if (part.mimeType === "text/html" && part.body?.data && !acc.htmlText) {
        acc.htmlText = decodeBase64Url(part.body.data);
    }
    for (const child of part.parts ?? []) {
        walkParts(child, acc);
    }
}
/** Extracts headers/body/attachments from one message's MIME payload -- shared by
 * `getThread` (which gets every message in a thread in one `threads.get` call) and
 * `syncMessages` (which fetches individual messages one at a time via
 * `messages.get`). Both call this with the same `payload` shape Gmail returns either
 * way, so there is exactly one place this parsing logic lives. */
function extractMessage(payload) {
    const headers = payload.headers ?? [];
    const acc = { attachments: [] };
    walkParts(payload, acc);
    return {
        messageId: getHeaderValue(headers, "Message-ID"),
        from: getHeaderValue(headers, "From"),
        to: getHeaderValue(headers, "To"),
        subject: getHeaderValue(headers, "Subject"),
        date: getHeaderValue(headers, "Date"),
        bodyText: acc.plainText ?? (acc.htmlText ? stripHtml(acc.htmlText) : ""),
        attachments: acc.attachments,
    };
}
/** Fetches a full Gmail thread -- every message's headers, MIME-walked body text,
 * and attachment metadata. No cleaning of quoted history/signatures happens here --
 * that's an L1-ingestion-time concern for a future sync pass, not this connector's
 * job (docs/superpowers/specs/2026-08-12-gmail-read-tool-design.md). */
export async function getThread(threadId, accessToken, fetchFn = fetch) {
    const url = new URL(`https://gmail.googleapis.com/gmail/v1/users/me/threads/${threadId}`);
    url.searchParams.set("format", "full");
    const response = await fetchFn(url, { headers: { Authorization: `Bearer ${accessToken}` } });
    if (!response.ok) {
        throw new Error(`Gmail API returned ${response.status}`);
    }
    const body = (await response.json());
    const messages = body.messages.map((m) => ({ id: m.id, ...extractMessage(m.payload) }));
    return { id: body.id, messages };
}
/** Incrementally syncs Gmail messages via `users.history.list`, or establishes a
 * fresh baseline via `users.getProfile` when no `historyId` is given -- zero
 * backfill, ever: the baseline call returns no messages, only the mailbox's current
 * `historyId` to sync forward from. `history.list` paginates on `nextPageToken`
 * until exhausted, merging `messagesAdded`/`messagesDeleted` ids across every page.
 * A message both added and deleted within the same range is excluded from
 * `messages` (fetching it would 404) but still reported in `deletedExternalIds`. A
 * 404 (expired `historyId`, Gmail's ~1-week floor) returns `needsFullResync: true`
 * instead of throwing, so the caller can cleanly re-baseline via the same
 * no-`historyId` path used on first connect. */
export async function syncMessages(historyId, accessToken, fetchFn = fetch) {
    if (!historyId) {
        const profileUrl = new URL("https://gmail.googleapis.com/gmail/v1/users/me/profile");
        const response = await fetchFn(profileUrl, { headers: { Authorization: `Bearer ${accessToken}` } });
        if (!response.ok) {
            throw new Error(`Gmail API returned ${response.status}`);
        }
        const body = (await response.json());
        return { messages: [], deletedExternalIds: [], nextHistoryId: body.historyId ?? "", needsFullResync: false };
    }
    const addedIds = new Set();
    const deletedIds = new Set();
    let pageToken;
    let latestHistoryId = historyId;
    do {
        const url = new URL("https://gmail.googleapis.com/gmail/v1/users/me/history");
        url.searchParams.set("startHistoryId", historyId);
        if (pageToken) {
            url.searchParams.set("pageToken", pageToken);
        }
        const response = await fetchFn(url, { headers: { Authorization: `Bearer ${accessToken}` } });
        if (response.status === 404) {
            return { messages: [], deletedExternalIds: [], nextHistoryId: "", needsFullResync: true };
        }
        if (!response.ok) {
            throw new Error(`Gmail API returned ${response.status}`);
        }
        const body = (await response.json());
        for (const record of body.history ?? []) {
            for (const added of record.messagesAdded ?? []) {
                addedIds.add(added.message.id);
            }
            for (const deleted of record.messagesDeleted ?? []) {
                deletedIds.add(deleted.message.id);
            }
        }
        pageToken = body.nextPageToken;
        if (body.historyId) {
            latestHistoryId = body.historyId;
        }
    } while (pageToken);
    for (const id of deletedIds) {
        addedIds.delete(id);
    }
    const messages = [];
    for (const id of addedIds) {
        const msgUrl = new URL(`https://gmail.googleapis.com/gmail/v1/users/me/messages/${id}`);
        msgUrl.searchParams.set("format", "full");
        const msgResponse = await fetchFn(msgUrl, { headers: { Authorization: `Bearer ${accessToken}` } });
        // A 404 means the message vanished between history.list and this fetch (a discarded
        // draft, a purged message). Skip it: throwing would leave the cursor un-advanced and
        // wedge every later poll on the same id.
        if (msgResponse.status === 404) {
            continue;
        }
        if (!msgResponse.ok) {
            throw new Error(`Gmail API returned ${msgResponse.status}`);
        }
        const msgBody = (await msgResponse.json());
        const extracted = extractMessage(msgBody.payload);
        messages.push({
            id: msgBody.id,
            threadId: msgBody.threadId,
            from: extracted.from,
            to: extracted.to,
            subject: extracted.subject,
            date: extracted.date,
            internalDate: msgBody.internalDate,
            bodyText: extracted.bodyText,
            attachments: extracted.attachments,
            labelIds: msgBody.labelIds ?? [],
            listUnsubscribe: getHeaderValue(msgBody.payload.headers ?? [], "List-Unsubscribe") !== "",
        });
    }
    return { messages, deletedExternalIds: [...deletedIds], nextHistoryId: latestHistoryId, needsFullResync: false };
}
/** Creates a reply-in-thread Gmail draft -- never sends anything, the draft sits in
 * the account's Drafts folder until the user sends it by hand. Reuses
 * `buildRawMessage`, the same RFC 2822 construction and header-injection guard
 * `sendEmail` uses, rather than a parallel implementation. Fetches lightweight thread
 * metadata first to get the last message's `Message-ID` and any existing
 * `References` chain, so the built reply carries `In-Reply-To`/`References` --
 * without these, Gmail has no way to insert the draft into the existing thread
 * (https://developers.google.com/workspace/gmail/api/guides/threads). */
export async function createDraft(threadId, to, subject, body, accessToken, fetchFn = fetch) {
    const threadUrl = new URL(`https://gmail.googleapis.com/gmail/v1/users/me/threads/${threadId}`);
    threadUrl.searchParams.set("format", "metadata");
    threadUrl.searchParams.append("metadataHeaders", "Message-ID");
    threadUrl.searchParams.append("metadataHeaders", "References");
    const threadResponse = await fetchFn(threadUrl, { headers: { Authorization: `Bearer ${accessToken}` } });
    if (!threadResponse.ok) {
        throw new Error(`Gmail API returned ${threadResponse.status}`);
    }
    const threadBody = (await threadResponse.json());
    const lastMessage = threadBody.messages?.[threadBody.messages.length - 1];
    const lastHeaders = lastMessage?.payload?.headers ?? [];
    const lastMessageId = getHeaderValue(lastHeaders, "Message-ID");
    const priorReferences = getHeaderValue(lastHeaders, "References");
    const references = [priorReferences, lastMessageId].filter(Boolean).join(" ");
    const replySubject = /^re:/i.test(subject) ? subject : `Re: ${subject}`;
    const raw = buildRawMessage(to, replySubject, body, lastMessageId ? { inReplyTo: lastMessageId, references } : undefined);
    const url = new URL("https://gmail.googleapis.com/gmail/v1/users/me/drafts");
    const response = await fetchFn(url, {
        method: "POST",
        headers: {
            Authorization: `Bearer ${accessToken}`,
            "Content-Type": "application/json",
        },
        body: JSON.stringify({ message: { raw, threadId } }),
    });
    if (!response.ok) {
        throw new Error(`Gmail API returned ${response.status}`);
    }
    return response.json();
}
