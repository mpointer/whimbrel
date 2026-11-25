// Settings search for the redesigned tab: the reorganisation's registry and matching (settingsSearch.ts), with the
// words the redesign changed. "capture" still finds the Memory & Privacy section, and "memory" finds it too.
import { fold, SETTINGS_SEARCH_ENTRIES, type SettingsSearchEntry } from "../settingsSearch";
import { redesignSectionTitle } from "./sections";

const RENAMED: Record<string, { title: string; add: string[] }> = {
  "capture.capture": { title: "Memory", add: ["activity capture", "memory", "pause", "screen captures"] },
  "capture.encryption": { title: "Library encryption", add: ["database encryption", "your library"] },
  "capture.retention": { title: "Keeping data", add: ["keep forever"] },
};

/** The registry the redesigned tab searches: same ids (they are the rows' anchors), new titles where a name changed,
 *  and every Memory & Privacy entry also answers to "capture" and "memory". */
export const REDESIGN_SEARCH_ENTRIES: readonly SettingsSearchEntry[] = SETTINGS_SEARCH_ENTRIES.map((entry) => {
  const renamed = RENAMED[entry.id];
  const extra = entry.section === "capture" ? ["capture", "memory"] : [];
  const keywords = [...entry.keywords, ...(renamed?.add ?? []), ...extra];
  return { ...entry, title: renamed?.title ?? entry.title, keywords: [...new Set(keywords)] };
});

/** Title matches first, then keyword or section-title matches; case and accent insensitive; never a regular expression. */
export function searchRedesign(query: string): SettingsSearchEntry[] {
  const q = fold(query.trim());
  if (!q) return [];
  const titleHits: SettingsSearchEntry[] = [];
  const otherHits: SettingsSearchEntry[] = [];
  for (const entry of REDESIGN_SEARCH_ENTRIES) {
    if (fold(entry.title).includes(q)) titleHits.push(entry);
    else if (entry.keywords.some((k) => fold(k).includes(q)) || fold(redesignSectionTitle(entry.section)).includes(q)) otherHits.push(entry);
  }
  return [...titleHits, ...otherHits];
}
