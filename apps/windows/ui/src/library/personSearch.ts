// Library search narrowed to a person (the People chip): captures whose text matches the name, and meeting results
// (transcript and slide hits) from meetings that person attended. Captures record no person, so the capture half is a
// text match: the name goes to the filtered capture search as an extra search term. The meeting half reads the
// attendees the existing meeting commands already give.

import { libraryMeeting, librarySearch, librarySearchFiltered, meetingList } from "../api";
import type { LibraryRow } from "../generated/types";
import type { SearchRequest } from "./searchRequest";

const MEETINGS_READ = 30;

/** The ids of recent meetings `person` attended, inside the request's dates when it has any. */
export async function meetingsWith(person: string, from: Pick<SearchRequest, "startUnixMs" | "endUnixMs">): Promise<Set<number>> {
  const rows = (await meetingList("")).filter((m) => (from.startUnixMs == null || m.tsUnixMs >= from.startUnixMs) && (from.endUnixMs == null || m.tsUnixMs < from.endUnixMs)).slice(0, MEETINGS_READ);
  const want = person.trim().toLowerCase();
  const found = await Promise.all(rows.map((m) => libraryMeeting(m.id).then((d) => ((d?.attendees ?? []).some((a) => a.trim().toLowerCase() === want) ? m.id : null), () => null)));
  return new Set(found.filter((id): id is number => id !== null));
}

/** The rows for a search with a person: capture hits that mention the name, then meeting hits from their meetings. */
export async function searchWithPerson(text: string, semantic: boolean, from: SearchRequest): Promise<LibraryRow[]> {
  const person = from.person ?? "";
  const [captures, hits, attended] = await Promise.all([
    librarySearchFiltered(`${text} ${person}`, semantic, from.app, from.startUnixMs, from.endUnixMs),
    librarySearch(text, semantic),
    meetingsWith(person, from),
  ]);
  const meetingHits = hits.filter((r) => (r.kind === "transcript" || r.kind === "snapshot") && attended.has(r.meetingId));
  return [...captures, ...meetingHits];
}
