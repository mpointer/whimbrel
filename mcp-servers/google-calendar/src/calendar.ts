export type FetchLike = (url: URL, init: { method?: string; headers: Record<string, string>; body?: string }) => Promise<{
  ok: boolean;
  status: number;
  json: () => Promise<unknown>;
  text?: () => Promise<string>;
  body?: ReadableStream<Uint8Array> | null;
}>;

/** Most of an error body we look at: Google's error JSON is small, anything bigger is not it. */
const ERROR_BODY_CAP = 16 * 1024;

/** The machine-readable reason in a Google API error body, or undefined. Order:
 * `error.errors[0].reason` (e.g. `insufficientPermissions`, `accessNotConfigured`),
 * then `error.details[].reason` (e.g. `ACCESS_TOKEN_SCOPE_INSUFFICIENT`), then
 * `error.status` (e.g. `PERMISSION_DENIED`). Only a short `[A-Za-z0-9_.-]` token is
 * returned, so no message text, email or token can ride along into logs. */
export function googleErrorReason(body: string): string | undefined {
  let parsed: unknown;
  try {
    parsed = JSON.parse(body.slice(0, ERROR_BODY_CAP));
  } catch {
    return undefined;
  }
  const error = (parsed as { error?: unknown } | null)?.error;
  if (!error || typeof error !== "object") return undefined;
  const e = error as { errors?: unknown; details?: unknown; status?: unknown };
  const candidates: unknown[] = [];
  if (Array.isArray(e.errors)) candidates.push((e.errors[0] as { reason?: unknown } | undefined)?.reason);
  if (Array.isArray(e.details)) {
    for (const d of e.details) candidates.push((d as { reason?: unknown } | null)?.reason);
  }
  candidates.push(e.status);
  for (const c of candidates) {
    if (typeof c === "string" && /^[A-Za-z0-9_.-]{1,64}$/.test(c)) return c;
  }
  return undefined;
}

/** At most `ERROR_BODY_CAP` bytes of a response body, as text; "" when there is none.
 * A stream is cancelled once the cap is reached, so a huge body is never read whole. */
async function readCappedBody(response: { text?: () => Promise<string>; body?: ReadableStream<Uint8Array> | null }): Promise<string> {
  const stream = response.body;
  if (stream && typeof stream.getReader === "function") {
    const reader = stream.getReader();
    const chunks: Uint8Array[] = [];
    let total = 0;
    try {
      while (total < ERROR_BODY_CAP) {
        const { done, value } = await reader.read();
        if (done || !value) break;
        chunks.push(value);
        total += value.byteLength;
      }
    } finally {
      await reader.cancel().catch(() => {});
    }
    return Buffer.concat(chunks).subarray(0, ERROR_BODY_CAP).toString("utf8");
  }
  if (typeof response.text === "function") {
    return (await response.text()).slice(0, ERROR_BODY_CAP);
  }
  return "";
}

/** The error for a non-OK Google Calendar response: `Google Calendar API returned
 * <status>`, plus ` (<reason>)` when the body names one. The body is read once,
 * capped, and never put in the message. */
async function apiError(response: {
  status: number;
  text?: () => Promise<string>;
  body?: ReadableStream<Uint8Array> | null;
}): Promise<Error> {
  let reason: string | undefined;
  try {
    reason = googleErrorReason(await readCappedBody(response));
  } catch {
    reason = undefined;
  }
  return new Error(`Google Calendar API returned ${response.status}${reason ? ` (${reason})` : ""}`);
}

/** Fetches events on the primary Google Calendar between `timeMin` and
 * `timeMax` (ISO 8601 timestamps). `fetchFn` defaults to the real global
 * `fetch` (Node's native implementation) and is overridable for tests. */
export async function listEvents(
  timeMin: string,
  timeMax: string,
  accessToken: string,
  fetchFn: FetchLike = fetch,
): Promise<unknown> {
  const url = new URL("https://www.googleapis.com/calendar/v3/calendars/primary/events");
  url.searchParams.set("timeMin", timeMin);
  url.searchParams.set("timeMax", timeMax);
  // Google's events.list defaults to singleEvents=false, which returns recurring event
  // MASTERS (one row per series, at the series' original start date) rather than the
  // individual occurrences a meeting-detection consumer actually needs.
  url.searchParams.set("singleEvents", "true");
  url.searchParams.set("orderBy", "startTime");
  const response = await fetchFn(url, { headers: { Authorization: `Bearer ${accessToken}` } });
  if (!response.ok) {
    throw await apiError(response);
  }
  return response.json();
}

/** Creates an event on the primary Google Calendar. `startIso`/`endIso` are full
 * ISO 8601 datetime strings (with timezone offset) -- Google's Events resource
 * requires `start.dateTime`/`end.dateTime` in this form, not bare dates. */
export async function createEvent(
  title: string,
  startIso: string,
  endIso: string,
  accessToken: string,
  fetchFn: FetchLike = fetch,
): Promise<unknown> {
  const url = new URL("https://www.googleapis.com/calendar/v3/calendars/primary/events");
  const response = await fetchFn(url, {
    method: "POST",
    headers: {
      Authorization: `Bearer ${accessToken}`,
      "Content-Type": "application/json",
    },
    body: JSON.stringify({
      summary: title,
      start: { dateTime: startIso },
      end: { dateTime: endIso },
    }),
  });
  if (!response.ok) {
    throw await apiError(response);
  }
  return response.json();
}

export async function syncEvents(
  syncToken: string | undefined,
  accessToken: string,
  fetchFn: FetchLike = fetch,
): Promise<{ needsFullResync: true } | unknown> {
  const endpoint = "https://www.googleapis.com/calendar/v3/calendars/primary/events";
  const headers = { Authorization: `Bearer ${accessToken}` };

  if (syncToken) {
    const url = new URL(endpoint);
    // singleEvents must be identical on EVERY call of a sync sequence, including the
    // token-based ones -- omitting it here makes Google return recurring-event masters
    // whose ids don't match the expanded occurrences the baseline call stored, silently
    // breaking deletion for recurring meetings. showDeleted is deliberately NOT set:
    // with a syncToken deleted events are always included regardless, and Google
    // rejects the combination.
    url.searchParams.set("singleEvents", "true");
    url.searchParams.set("syncToken", syncToken);
    const response = await fetchFn(url, { headers });
    if (response.status === 410) {
      return { needsFullResync: true };
    }
    if (!response.ok) {
      throw await apiError(response);
    }
    // ponytail: incremental calls return a single page only -- a busy interval just
    // catches up over the next poll or two, nothing is lost. The baseline branch below
    // does page fully, because only its last page carries nextSyncToken.
    return response.json();
  }

  // Initial baseline sync: bounded to the same "recent past onward" scope listEvents
  // already covers, rather than pulling the account's entire event history. timeMin is
  // safe on the baseline call -- timeMax is not, since an upper bound would limit what
  // future changes the resulting sync token can ever report.
  const timeMin = new Date(Date.now() - 15 * 60 * 1000).toISOString();
  const items: unknown[] = [];
  let pageToken: string | undefined;
  for (;;) {
    const url = new URL(endpoint);
    url.searchParams.set("showDeleted", "true");
    url.searchParams.set("singleEvents", "true");
    url.searchParams.set("timeMin", timeMin);
    if (pageToken) {
      url.searchParams.set("pageToken", pageToken);
    }
    const response = await fetchFn(url, { headers });
    if (!response.ok) {
      throw await apiError(response);
    }
    const page = (await response.json()) as { items?: unknown[]; nextPageToken?: string };
    items.push(...(page.items ?? []));
    // Only the last page carries nextSyncToken, so return that page's other fields.
    if (!page.nextPageToken) {
      return { ...page, items };
    }
    pageToken = page.nextPageToken;
  }
}
