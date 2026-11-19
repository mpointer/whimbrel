/** A search Home hands to Library: the words, and the person, app and dates its filter chips chose. Library runs it
 *  once when it opens (the redesigned Home only; nothing else makes one). */
export interface SearchRequest {
  id: number;
  query: string;
  /** An attendee of recent meetings, or null. */
  person: string | null;
  app: string | null;
  startUnixMs: number | null;
  endUnixMs: number | null;
  /** The filters in words ("Excel, Last 7 days"), for the line under the field. */
  summary: string;
}
