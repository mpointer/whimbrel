// What the Find page shows, decided from what the shell gives it. Pure: nothing here touches the DOM, the store of the page
// or the shell. (docs/ui-redesign/screens/ask.md)

import type { KeptFindFilter } from "../../api";
import { answeredLocallyLine, isBusy } from "../../chat/chatModel";
import type { FindFilterInput, FoundCard, FoundPage, LibraryRow, NumberedSourceView, RoutePreviewView } from "../../generated/types";
import { t } from "../../l10n";
import { offsetClock } from "../../meetings/redesign/noteModel";
import type { WIconName } from "../../system";
import { appIcon, appName, dateChoiceLabel, dateRange, startOfDay, type SearchFilter } from "../../today/redesign/homeModel";
import { dayLabel, episodeSummaryText, shortDate, timeLabel } from "../rows";
import type { SourcedTurn } from "./askSession";

// --- cards ------------------------------------------------------------------------------------------

/** The kind in words, as the spec names them. */
export function kindLabel(kind: string): string {
  switch (kind) {
    case "capture":
      return t("Seen on screen");
    case "transcript":
      return t("Transcript moment");
    case "snapshot":
      return t("Slide");
    case "thread":
      return t("Email thread");
    case "episode":
      return t("Episode");
    case "routine_run":
      return t("Automation run");
    case "meeting":
      return t("Meeting");
    default:
      return t("Result");
  }
}

/** The kind as an icon (the words are always beside it). A capture takes its app's icon. */
export function kindIcon(card: Pick<FoundCard, "kind" | "app">): WIconName {
  switch (card.kind) {
    case "capture":
    case "episode":
      return card.app ? appIcon(card.app) : "document";
    case "transcript":
      return "mic";
    case "snapshot":
      return "table";
    case "thread":
      return "mail";
    case "routine_run":
      return "automations";
    case "meeting":
      return "meetings";
    default:
      return "document";
  }
}

/** The app's name, or what stands in for one: a meeting, an email, an automation. */
export function cardApp(card: Pick<FoundCard, "kind" | "app">): string {
  switch (card.kind) {
    case "capture":
    case "episode":
      return card.app ? appName(card.app) : t("Screen");
    case "transcript":
      return t("Meeting audio");
    case "snapshot":
    case "meeting":
      return t("Meeting");
    case "thread":
      return t("Email");
    case "routine_run":
      return t("Automation");
    default:
      return t("Memory");
  }
}

export const cardKey = (card: Pick<FoundCard, "kind" | "id">): string => `${card.kind}-${card.id}`;

/** The title, or the app when the row has none. */
export function cardTitle(card: FoundCard): string {
  const title = card.title.trim();
  return title !== "" ? title : cardApp(card);
}

/** "Tue, Nov 10 · 4:10 PM", and for a transcript line "· 14:32 into the meeting"; the date alone when no time is known. */
export function cardWhen(card: FoundCard, now: Date, locale: string): string {
  const ts = card.tsUnixMs;
  const when = ts == null ? "" : `${shortDate(ts, now, locale)} · ${timeLabel(ts, locale)}`;
  if (card.kind === "transcript" && card.startCs != null) {
    const into = t("{time} into the meeting", { time: offsetClock(card.startCs) });
    return when ? `${when} · ${into}` : into;
  }
  return when;
}

/** "Thursday 4:10 PM", for a source mark's name. */
function weekdayTime(ts: number, locale: string): string {
  return `${new Date(ts).toLocaleDateString(locale, { weekday: "long" })} ${timeLabel(ts, locale)}`;
}

/** A source mark's name for assistive technology: "Source 2, Slack, Thursday 4:10 PM". A row the core could not read just now is not
 *  said to be gone: it may well exist. */
export function sourceMarkName(n: number, card: FoundCard | undefined, locale: string, unreadable = false): string {
  if (!card) return t("Source {number}", { number: n });
  if (!card.found) return unreadable ? t("Source {number}, could not be read just now", { number: n }) : t("Source {number}, no longer on this device", { number: n });
  const app = cardApp(card);
  return card.tsUnixMs != null ? t("Source {number}, {app}, {when}", { number: n, app, when: weekdayTime(card.tsUnixMs, locale) }) : t("Source {number}, {app}", { number: n, app });
}

/** What Open does for a card, or null where the spec gives none (an email thread, an automation run, a row that is gone). */
export type FindOpen = { kind: "capture" | "meeting" | "episode"; id: number; line?: number };

export function openTarget(card: FoundCard): FindOpen | null {
  if (!card.found) return null;
  switch (card.kind) {
    case "capture":
      return { kind: "capture", id: card.id };
    case "transcript":
      return card.meetingId != null ? { kind: "meeting", id: card.meetingId, line: card.id } : null;
    case "snapshot":
      return card.meetingId != null ? { kind: "meeting", id: card.meetingId } : null;
    case "meeting":
      return { kind: "meeting", id: card.id };
    case "episode":
      return { kind: "episode", id: card.id };
    default:
      return null;
  }
}

/** What Open will open, written beside the button. */
export function openHint(card: FoundCard): string {
  switch (card.kind) {
    case "capture":
      return t("Opens the saved capture");
    case "transcript":
      return card.startCs != null ? t("Opens the note at {time}", { time: offsetClock(card.startCs) }) : t("Opens the meeting");
    case "episode":
      return t("Opens the episode");
    default:
      return t("Opens the meeting");
  }
}

/** Open's name for assistive technology, told apart by time: "Open Design sync at 0:29" for a meeting's line (its offset), "Open
 *  Pricing.xlsx at 11:02 AM" for a row with a time, the title alone when none is known. */
export function openName(card: FoundCard, locale: string): string {
  const title = cardTitle(card);
  if (card.kind === "transcript" && card.startCs != null) return t("Open {title} at {time}", { title, time: offsetClock(card.startCs) });
  if (card.tsUnixMs != null) return t("Open {title} at {time}", { title, time: timeLabel(card.tsUnixMs, locale) });
  return t("Open {title}", { title });
}

/** Where the page's scroll container scrolls to put a card in its middle (the top of a card taller than it), never past either end.
 *  The page scrolls itself, never the window: scrolling the window would push the header and the side bar's top out of sight. */
export function centeredScrollTop(
  container: { scrollTop: number; scrollHeight: number; top: number; height: number },
  card: { top: number; height: number },
): number {
  const offset = card.top - container.top;
  const want = container.scrollTop + offset - (card.height < container.height ? (container.height - card.height) / 2 : 0);
  return Math.max(0, Math.min(want, container.scrollHeight - container.height));
}

// --- the answer's marks (placed on the rendered tree in sourceMarks.ts) --------------------------------

/** The source number a link made by `rehypeSourceMarks` names, or null for any other link. */
export function sourceOfHref(href: string | undefined): number | null {
  const m = /^#src-(\d+)$/.exec(href ?? "");
  return m ? Number(m[1]) : null;
}

/** The numbers in `text` that are sources, in the order first written, once each. */
export function citedIn(text: string, valid: ReadonlySet<number>): number[] {
  const out: number[] = [];
  for (const m of text.matchAll(/\[(\d+)\]/g)) {
    const n = Number(m[1]);
    if (valid.has(n) && !out.includes(n)) out.push(n);
  }
  return out;
}

// --- the time line -------------------------------------------------------------------------------------------

export interface DayGroup {
  key: string;
  label: string;
  /** The day's start, or null for rows with no date. */
  at: number | null;
  cards: FoundCard[];
}

function dayStart(ms: number): number {
  const d = new Date(ms);
  return new Date(d.getFullYear(), d.getMonth(), d.getDate()).getTime();
}

/** Cards grouped by local day. `newest` puts the latest day and the latest time first; `oldest` the reverse (a time order for
 *  an answer's sources). Rows with no time go last, in the order given. */
export function dayGroups(cards: readonly FoundCard[], now: Date, locale: string, order: "newest" | "oldest"): DayGroup[] {
  const dated = cards.filter((c) => c.tsUnixMs != null);
  const undated = cards.filter((c) => c.tsUnixMs == null);
  const sign = order === "newest" ? -1 : 1;
  const sorted = [...dated].sort((a, b) => sign * ((a.tsUnixMs as number) - (b.tsUnixMs as number)));
  const groups: DayGroup[] = [];
  for (const card of sorted) {
    const at = dayStart(card.tsUnixMs as number);
    const last = groups[groups.length - 1];
    if (last && last.at === at) last.cards.push(card);
    else groups.push({ key: String(at), label: dayLabel(card.tsUnixMs as number, now, locale), at, cards: [card] });
  }
  if (undated.length) groups.push({ key: "undated", label: t("No date"), at: null, cards: [...undated] });
  return groups;
}

/** An answer's evidence: the sources it cites that are in the list, with their numbers, oldest first (a row with no time last). */
export function evidenceOf(sources: readonly NumberedSourceView[], cited: readonly number[] | null): NumberedSourceView[] {
  if (!cited) return [];
  const byNumber = new Map(sources.map((s) => [s.n, s]));
  const picked = cited.map((n) => byNumber.get(n)).filter((s): s is NumberedSourceView => !!s);
  return [...picked].sort((a, b) => (a.card.tsUnixMs ?? Infinity) - (b.card.tsUnixMs ?? Infinity) || a.n - b.n);
}

/** The results that are not already shown as evidence. */
export function withoutEvidence(results: readonly FoundCard[], evidence: readonly NumberedSourceView[]): FoundCard[] {
  const shown = new Set(evidence.map((e) => cardKey(e.card)));
  return results.filter((c) => !shown.has(cardKey(c)));
}

// --- filters ---------------------------------------------------------------------------------------------------

/** The core's filter for the page's chips. */
export function filterInput(filter: SearchFilter, now: number): FindFilterInput {
  const range = filter.dates ? dateRange(filter.dates, now) : null;
  return { startUnixMs: range?.start ?? null, endUnixMs: range?.end ?? null, app: filter.app, person: filter.person, personKey: null };
}

const ymd = (ms: number): string => {
  const d = new Date(ms);
  return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}-${String(d.getDate()).padStart(2, "0")}`;
};
const isMidnight = (ms: number): boolean => startOfDay(ms) === ms;

/** The chips a reopened Find conversation shows and asks under: its app, and its kept days (whole days as those dates, anything else as
 *  that span). Never a person. */
export function keptChips(kept: KeptFindFilter | null): SearchFilter {
  const start = kept?.start_unix_ms ?? null;
  const end = kept?.end_unix_ms ?? null;
  let dates: SearchFilter["dates"] = null;
  if (start !== null && end !== null && end > start) {
    if (isMidnight(start) && isMidnight(end)) {
      dates = { kind: "custom", from: ymd(start), to: ymd(startOfDay(end - 1)) };
    } else {
      const at = (ms: number) => new Intl.DateTimeFormat(undefined, { month: "short", day: "numeric", hour: "numeric", minute: "2-digit" }).format(new Date(ms));
      dates = { kind: "span", start, end, label: t("{from} to {to}", { from: at(start), to: at(end) }) };
    }
  }
  return { person: null, app: kept?.app_bundle_id ?? null, dates };
}

/** The same filter as a string, to tell whether it changed. */
export const filterKey = (f: FindFilterInput): string => JSON.stringify([f.startUnixMs ?? null, f.endUnixMs ?? null, f.app ?? null, f.person ?? null]);

export const hasFindFilter = (f: FindFilterInput): boolean => (f.startUnixMs ?? f.endUnixMs ?? f.app ?? f.person ?? null) !== null;

/** What the chips do, in words, under the field: one sentence per chosen filter, by the rules in the spec's filter table. */
export function filterWords(filter: SearchFilter): string {
  const parts: string[] = [];
  if (filter.person) parts.push(t("Meetings {name} attended, lines {name} spoke, and email from or to {name}. Captures that mention {name} are listed apart.", { name: filter.person }));
  if (filter.app) parts.push(t("Captures from {app} only: meetings, email and the rest are hidden while an app is chosen.", { app: appName(filter.app) }));
  if (filter.dates) parts.push(t("Dates: {dates}.", { dates: dateChoiceLabel(filter.dates) }));
  return parts.join(" ");
}

/** The filters that are on, for "No results" ("People: Sam Okafor, Dates: Last 7 days"). */
export function filtersOnLine(filter: SearchFilter): string {
  const parts: string[] = [];
  if (filter.person) parts.push(t("People: {name}", { name: filter.person }));
  if (filter.app) parts.push(t("Apps: {name}", { name: appName(filter.app) }));
  if (filter.dates) parts.push(t("Dates: {dates}", { dates: dateChoiceLabel(filter.dates) }));
  return parts.join(", ");
}

/** "3 results", "1 result", "200+ results" when the search hit its pool. */
export function countLabel(page: Pick<FoundPage, "total" | "mentionsTotal" | "capped"> | null): string | null {
  if (!page) return null;
  const count = page.total + page.mentionsTotal;
  if (count === 1) return t("1 result");
  return page.capped ? t("{count}+ results", { count }) : t("{count} results", { count });
}

// --- the Ask button --------------------------------------------------------------------------------------------

const PROVIDERS: Record<string, string> = { anthropic: "Anthropic", openai: "OpenAI", google: "Google", openrouter: "OpenRouter", whimbrel: "Whimbrel AI", ollama: "Ollama" };

export function providerName(id: string | null | undefined): string | null {
  if (!id) return null;
  return PROVIDERS[id] ?? id.charAt(0).toUpperCase() + id.slice(1);
}

/** The Ask button's words. They come from the route preview and nothing else: "on this PC" only when the preview said the answer
 *  is made here, a provider's name only when the preview named one and no other can take over. With no preview, plain "Ask". */
/** Core refuses these only because the answer must stay on this PC and no model here can give it: the question is still one for this
 *  PC (it is sent with `requireLocal` and fails as such), never one a hosted model may take. */
const LOCAL_REFUSALS = new Set(["local_only_no_local_model", "excluded_client_no_local_model", "local_only_cloud_or_unknown_model"]);
const onThisPc = (preview: RoutePreviewView): boolean =>
  (preview.destination === "local" && preview.local) || (preview.destination === "refused" && LOCAL_REFUSALS.has(preview.reason));

export function askLabel(preview: RoutePreviewView | null): string {
  if (!preview) return t("Ask");
  if (onThisPc(preview)) return t("Ask, on this PC");
  const provider = providerName(preview.provider);
  if (preview.destination === "hosted" && provider && !preview.mayFallBack) return t("Ask {provider}", { provider });
  return t("Ask");
}

/** The small line beside the button, from the same preview; null when it knows too little to say anything. */
export function routeLine(preview: RoutePreviewView | null): string | null {
  if (!preview) return null;
  if (preview.destination === "local" && preview.local) {
    return preview.model ? t("Answers are written on this PC by {model}.", { model: preview.model }) : t("Answers are written on this PC.");
  }
  return null;
}

/** Whether the label named a place the answer must stay: the ask then goes with `requireLocal`, and fails rather than leave it. */
export const staysLocal = (preview: RoutePreviewView | null): boolean =>
  !!preview && ((preview.destination === "local" && preview.local && preview.requireLocal) || (preview.destination === "refused" && LOCAL_REFUSALS.has(preview.reason)));

/** What a question that had to stay on this PC says when no model here could answer it (the Mac's `AskRunner.localOnlyFailure`), in
 *  place of the general "Ollama isn't running ... or add an API key", which offers a hosted model the person ruled out. Null otherwise. */
export function localOnlyFailure(error: unknown, requireLocal: boolean): string | null {
  if (!requireLocal) return null;
  const e = error as { kind?: string; reason?: { kind?: string } } | null;
  if (e?.kind !== "provider_failure") return null;
  const why = e.reason?.kind;
  return why === "ollama_not_running" || why === "local_model_missing" || why === "no_provider_configured"
    ? t("Not sent: this question was to be answered on this PC, and no model on this PC could answer it. Set one up in Settings → AI.")
    : null;
}

/** What an answer drew on: "From your work memory", and the dates when the filter sets them ("Nov 9 to Nov 15"). */
export function drewOnLine(filter: FindFilterInput, locale: string, drew?: { startUnixMs: number; endUnixMs: number } | null): string {
  // The range the answer actually drew on, when the core said, outranks the filter's.
  const { startUnixMs: start, endUnixMs: end } = drew ?? filter;
  if (start == null || end == null) return t("From your work memory");
  const day = (ms: number) => new Intl.DateTimeFormat(locale, { month: "short", day: "numeric" }).format(new Date(ms));
  const first = day(start);
  const last = day(Math.max(start, end - 1));
  return first === last ? t("From your work memory, {day}", { day: first }) : t("From your work memory, {from} to {to}", { from: first, to: last });
}

/** A Recent, Episodes or search row as the same card the page draws for a result, so every view shares one look. */
export function cardOfRow(row: LibraryRow): FoundCard {
  const base = { found: true, speaker: null, speakerIsGuess: null, sender: null, via: null };
  switch (row.kind) {
    case "capture":
      return { ...base, kind: "capture", id: row.id, tsUnixMs: row.tsUnixMs, title: row.title || appName(row.app), text: row.snippet, app: row.app, device: row.device, meetingId: null, meetingTitle: null, meetingStartUnixMs: null, startCs: null };
    case "meeting":
      return { ...base, kind: "meeting", id: row.id, tsUnixMs: row.tsUnixMs, title: row.title, text: null, app: null, device: row.device, meetingId: row.id, meetingTitle: row.title, meetingStartUnixMs: row.tsUnixMs, startCs: null };
    case "episode":
      return { ...base, kind: "episode", id: row.id, tsUnixMs: row.tsUnixMs, title: row.title ?? "", text: episodeSummaryText(row), app: row.app, device: row.device, meetingId: null, meetingTitle: null, meetingStartUnixMs: null, startCs: null };
    case "transcript":
      return { ...base, kind: "transcript", id: row.id, tsUnixMs: null, title: row.meetingTitle, text: row.snippet, app: null, device: null, meetingId: row.meetingId, meetingTitle: row.meetingTitle, meetingStartUnixMs: null, startCs: row.startCs, speaker: row.speaker };
    case "snapshot":
      return { ...base, kind: "snapshot", id: row.id, tsUnixMs: null, title: row.meetingTitle, text: row.snippet, app: null, device: null, meetingId: row.meetingId, meetingTitle: row.meetingTitle, meetingStartUnixMs: null, startCs: null };
  }
}

// --- loading more, and what is searched ------------------------------------------------------------------------

/** Cards for a next page added to the list so far, leaving out any already listed (a row inserted between two pages moves every later
 *  row down one, so the next page can repeat the last one). A card is the same card by kind and id. */
export function appendUnique(existing: readonly FoundCard[], incoming: readonly FoundCard[]): FoundCard[] {
  const seen = new Set(existing.map(cardKey));
  const out = [...existing];
  for (const card of incoming) {
    const key = cardKey(card);
    if (seen.has(key)) continue;
    seen.add(key);
    out.push(card);
  }
  return out;
}

/** What to search for: the words searched, an empty search that lists a filter's holdings when a filter is on, and nothing when there
 *  are no words and no filter (the filters were cleared after a filter-only list). */
export function effectiveQuery(searched: string | null, filtered: boolean): string | null {
  if (searched !== null && (searched !== "" || filtered)) return searched;
  return filtered ? "" : null;
}

/** What was searched once the field reads `text`: an emptied field forgets the search, so the page shows its empty state (or, with a
 *  filter on, the filter's list) instead of results for words no longer there. New words keep the old results until Enter. */
export function searchedAfterEdit(text: string, searched: string | null): string | null {
  return text.trim() === "" ? null : searched;
}

/** A search asked for with the words already searched: the page searches again, since what is stored may have changed (new items, their
 *  meaning read since). Changed words search anyway; nothing to search is never searched again. */
export function searchesAgain(next: string | null, searched: string | null): boolean {
  return next !== null && next === searched;
}

/** Words that open a question in the languages the app ships. */
const QUESTION_WORDS = new Set(["who", "what", "where", "when", "why", "how", "which", "whose", "qué", "dónde", "donde", "cuándo", "cuando", "quién", "quien", "cómo", "cuál", "où", "quand", "qui", "comment", "pourquoi", "quel", "quelle", "quels", "quelles", "wo", "wann", "wer", "wie", "warum", "was", "welche", "welcher", "welches", "onde", "quando", "quem", "qual", "quais"]);

/** The order a new search opens on: a question ("Where is the kite festival?", "who owns the budget") opens on best matches first,
 *  since the newest row holding one of its words is rarely the answer; plain words open by day. The picker still changes it. */
export function defaultOrder(words: string | null): "time" | "rank" {
  const w = (words ?? "").trim().toLocaleLowerCase();
  if (w === "") return "time";
  if (w.endsWith("?") || w.endsWith("？")) return "rank";
  const first = w.replace(/^[¿¡"'“‘(]+/u, "").split(/[\s,]+/u)[0] ?? "";
  return QUESTION_WORDS.has(first) ? "rank" : "time";
}

// --- asking ------------------------------------------------------------------------------------------------------

/** Whether an answer can be written, from what the AI settings said; unknown until they have been read. */
export function askReadiness(state: { available: boolean; model?: string } | null): "unknown" | "ready" | "unavailable" {
  if (!state) return "unknown";
  return state.available ? "ready" : "unavailable";
}

/** A question from another screen waits until the page knows whether it can answer: with nothing known it would read as "not set up"
 *  and send the person to Settings, and the question would be lost. A search needs no model and never waits. */
export const requestHeld = (mode: "search" | "ask", readiness: "unknown" | "ready" | "unavailable"): boolean => mode === "ask" && readiness === "unknown";

/** The follow-up field is there once the newest answer is over, however it ended; a failed or stopped question's words wait in it. */
export const followFormShown = (last: Pick<SourcedTurn, "status"> | undefined): boolean => !!last && !isBusy(last as SourcedTurn);

/** Words put back into a field never replace what the person has typed since. */
export const restoreText = (current: string, restored: string): string => (current.trim() === "" ? restored : current);

/** What to do with words that are going back into a field holding `current`: put them there (it is empty), leave it (it holds the same
 *  words, or other words typed since and the words were a stopped or failed question that the person has moved on from), or offer them
 *  (a question that was waiting must end up asked or back where the person can see it, never silently replaced). */
export function placeRestored(current: string, restored: string, waiting: boolean): "put" | "already" | "offer" {
  if (current.trim() === "") return "put";
  if (current.trim() === restored.trim()) return "already";
  return waiting ? "offer" : "already";
}

/** What a button's label promised about where the answer comes from, as one word, or null when it promised nothing (plain "Ask").
 *  "local" is "Ask, on this PC"; "hosted:<id>" is "Ask <provider>" with no other provider able to take over. */
export function promiseOf(preview: RoutePreviewView | null): string | null {
  if (!preview) return null;
  if (onThisPc(preview)) return "local";
  if (preview.destination === "hosted" && providerName(preview.provider) && !preview.mayFallBack) return `hosted:${preview.provider}`;
  return null;
}

/** What a press of Ask was about: the words, the filters, the conversation it would continue. */
export interface SendPress {
  words: string;
  filterKey: string;
  conversationId: number | null;
}
export const samePress = (a: SendPress, b: SendPress): boolean => a.words === b.words && a.filterKey === b.filterKey && a.conversationId === b.conversationId;

export type SendDecision = { go: true; requireLocal: boolean } | { go: false; why: "route" | "changed" };

/** The last look just before a question goes out. It does not go when (`route`) the button promised a place (this PC, or one named
 *  provider with no other able to take over) and the route now says anything else, or the route cannot be read and the promise is not
 *  one the call itself enforces; or when (`changed`), while the route was being read, the words, the filters or the conversation the
 *  question would continue are no longer what was pressed. When the fresh read fails, `requireLocal` follows what the button showed
 *  (it fails closed). */
export async function resolveSend(
  shown: RoutePreviewView | null,
  readFresh: () => Promise<RoutePreviewView | null>,
  pressed?: SendPress,
  current?: () => SendPress,
): Promise<SendDecision> {
  let fresh: RoutePreviewView | null = null;
  try {
    fresh = await readFresh();
  } catch {
    fresh = null;
  }
  if (pressed && current && !samePress(pressed, current())) return { go: false, why: "changed" };
  const promised = promiseOf(shown);
  if (promised !== null) {
    if (!fresh) {
      // Unreadable: a promise of this PC is kept by the call itself (it fails rather than leave), a named provider is not.
      if (promised !== "local") return { go: false, why: "route" };
    } else if (promiseOf(fresh) !== promised) return { go: false, why: "route" };
  }
  return { go: true, requireLocal: fresh ? staysLocal(fresh) : staysLocal(shown) };
}

/** Wraps an async action so a second call made while the first is still waiting does nothing (a double Ctrl+Enter). Calls are told
 *  apart by `keyOf`: a press of another control runs at once and is never dropped because this one is busy. */
export function singleFlight<A extends unknown[]>(action: (...args: A) => Promise<void>, keyOf: (...args: A) => string = () => ""): (...args: A) => Promise<void> {
  const running = new Set<string>();
  return async (...args: A) => {
    const key = keyOf(...args);
    if (running.has(key)) return;
    running.add(key);
    try {
      await action(...args);
    } finally {
      running.delete(key);
    }
  };
}

/** The small line under a finished answer, or null. A live answer knows why it was made here (a client excluded from hosted models) and
 *  where. A reopened one has only the saved flag that a local model answered: not which PC, and not why. */
export function answeredLine(turn: Pick<SourcedTurn, "answeredLocally" | "routedLocally" | "excludedClient" | "saved">): string | null {
  if (!turn.answeredLocally) return null;
  if (turn.saved) return t("Answered by a local model.");
  return turn.routedLocally || turn.excludedClient ? answeredLocallyLine() : t("Answered on this PC.");
}
