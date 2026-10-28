export type FetchLike = (url: URL, init: { method?: string; headers: Record<string, string>; body?: string }) => Promise<{
  ok: boolean;
  status: number;
  json: () => Promise<unknown>;
}>;

/** Lists events on the signed-in user's Outlook calendar between `startIso` and
 * `endIso` (ISO 8601 timestamps), via Microsoft Graph's calendarview endpoint.
 * `fetchFn` defaults to the real global `fetch` (Node's native implementation) and is
 * overridable for tests. */
export async function listEvents(
  startIso: string,
  endIso: string,
  accessToken: string,
  fetchFn: FetchLike = fetch,
): Promise<unknown> {
  const url = new URL("https://graph.microsoft.com/v1.0/me/calendarview");
  url.searchParams.set("startDateTime", startIso);
  url.searchParams.set("endDateTime", endIso);
  url.searchParams.set("$orderby", "start/dateTime");
  const response = await fetchFn(url, { headers: { Authorization: `Bearer ${accessToken}` } });
  if (!response.ok) {
    throw new Error(`Microsoft Graph calendarview returned ${response.status}`);
  }
  return response.json();
}

/** Reads the signed-in user's most recent mail messages via Microsoft Graph, newest
 * first. `top` bounds how many are returned (default 20). `fetchFn` defaults to the
 * real global `fetch` and is overridable for tests. */
export async function readMail(
  accessToken: string,
  fetchFn: FetchLike = fetch,
  top = 20,
): Promise<unknown> {
  const url = new URL("https://graph.microsoft.com/v1.0/me/messages");
  url.searchParams.set("$top", String(top));
  url.searchParams.set("$orderby", "receivedDateTime desc");
  url.searchParams.set("$select", "subject,from,receivedDateTime,bodyPreview");
  const response = await fetchFn(url, { headers: { Authorization: `Bearer ${accessToken}` } });
  if (!response.ok) {
    throw new Error(`Microsoft Graph messages returned ${response.status}`);
  }
  return response.json();
}
