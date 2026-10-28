export type FetchLike = (url: URL, init: { method?: string; headers: Record<string, string>; body?: string }) => Promise<{
  ok: boolean;
  status: number;
  json: () => Promise<unknown>;
}>;

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
    throw new Error(`Google Calendar API returned ${response.status}`);
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
    throw new Error(`Google Calendar API returned ${response.status}`);
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
      throw new Error(`Google Calendar API returned ${response.status}`);
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
      throw new Error(`Google Calendar API returned ${response.status}`);
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
