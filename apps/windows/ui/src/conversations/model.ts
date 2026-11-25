// Conversations (screen 6), the pure parts: where a question from the header goes, what a held answer keeps, a conversation's scope line,
// the topic chip's choices and the list's order. (docs/ui-redesign/screens/conversations.md)
// The Mac defines the routing rule in `WhimbrelTab.askTarget` (WhimbrelShellView.swift); the rest follows `AskAnswerRedesign.swift`,
// `ConversationRedesign.swift` and `ConversationsListRedesign.swift`.

import type { ContextItemRef, ConversationListRow, ConversationScopeView, MeetingAskExchange, RoutePreviewView, SavedExchangeInput, TopicRefView } from "../generated/types";
import type { Turn } from "../chat/chatModel";
import { t } from "../l10n";
import { askLabel } from "../library/redesign/findModel";
import type { SourcedTurn } from "../library/redesign/askSession";
import type { TabId } from "../tabs";

// --- where a question goes ------------------------------------------------------------------------------------------

export type AskTarget = { kind: "today" } | { kind: "conversation"; scope: ConversationScopeView } | { kind: "library" };

/** Where a question from the shell's Ask control goes (Ctrl+Enter; Enter always searches). Today answers under the bar. The Conversations
 *  section asks about everything, in a new conversation. A topic page (`topicId`: the topic shown) asks about that topic, in a new conversation.
 *  Everywhere else the Find page's own Ask, as before. On the Library tab the Find page decides for itself, so the shell leaves it alone.
 *  `filtered`: a People, Apps or Dates chip is set and showing (on Today). The Mac's rule covers a question with no search filter set; a section's
 *  own answer takes no filter, so a filtered question goes to the Find page, which obeys the chips and keeps the Dates and App with the
 *  conversation (docs/ui-redesign/screens/ask.md). A visible filter is never dropped. */
export function askTarget(tab: TabId, topicId: number | null, filtered = false): AskTarget {
  if (filtered) return { kind: "library" };
  switch (tab) {
    case "today":
      return { kind: "today" };
    case "conversations":
      return { kind: "conversation", scope: { kind: "everything" } };
    case "topics":
      return topicId === null ? { kind: "library" } : { kind: "conversation", scope: { kind: "topic", id: topicId } };
    default:
      return { kind: "library" };
  }
}

// --- the composer's button ------------------------------------------------------------------------------------------

/** The composer's button. Only an everything question is held to a place, so only it may name one ("Ask, on this PC"). A topic question is
 *  held to this PC when the control said so, but the page's own button names no place; a meeting question is never held. Both say "Send". */
export function composerLabel(scope: ConversationScopeView, preview: RoutePreviewView | null): string {
  if (scope.kind !== "everything") return t("Send");
  const named = askLabel(preview);
  return named === t("Ask") ? t("Send") : named;
}

// --- what a held answer keeps ---------------------------------------------------------------------------------------

/** The numbers an answer's `[n]` marks name, in the order first written, once each. Core lists citations in that order and drops a marker past
 *  the end of the context, so the distinct markers line up with the citations; when the text does not account for every citation it is 1, 2, 3...
 *  (the Mac's `AskSourceCardModel.markerNumbers`). */
export function markerNumbers(answer: string, citationCount: number): number[] {
  const seen: number[] = [];
  for (const m of answer.matchAll(/\[(\d+)\]/g)) {
    const n = Number(m[1]);
    if (n >= 1 && !seen.includes(n)) seen.push(n);
  }
  if (seen.length < citationCount) return Array.from({ length: citationCount }, (_, i) => i + 1);
  const kept = new Set([...seen].sort((a, b) => a - b).slice(0, citationCount));
  return seen.filter((n) => kept.has(n));
}

export interface Pointer {
  n: number;
  kind: string;
  id: number;
}

/** The pointers of a sourced answer: the cited numbers with the kind and id of the card each one names. Nothing else about a source. */
export function pointersOf(turn: Pick<SourcedTurn, "sources" | "cited">): Pointer[] {
  const byNumber = new Map(turn.sources.map((s) => [s.n, s]));
  const out: Pointer[] = [];
  for (const n of turn.cited ?? []) {
    const source = byNumber.get(n);
    if (source && !out.some((p) => p.n === n)) out.push({ n, kind: source.card.kind, id: source.card.id });
  }
  return out;
}

/** The pointers of a meeting answer, which has no numbered list: its citations numbered as the answer's marks. */
export function meetingPointers(answer: string, citations: readonly ContextItemRef[]): Pointer[] {
  const numbers = markerNumbers(answer, citations.length);
  return citations.map((c, i) => ({ n: numbers[i] ?? i + 1, kind: c.kind, id: c.id }));
}

/** Whether an Ollama model runs on Ollama's servers: core's `is_ollama_cloud_model` (llm/exclusion.rs), fail closed. Any word `cloud` in its
 *  name or tag counts, not only a trailing one (`glm-4.6:cloud`, `kimi-k2:cloud-1t`, `gpt-oss:120b-cloud`). */
export const isOllamaCloudModel = (model: string | null | undefined): boolean =>
  /(^|[^a-z0-9])cloud($|[^a-z0-9])/i.test(model ?? "");

/** Whether a model runs on this PC (core's `is_hosted`, the Rust shell's `made_here`): the bundled one, or an Ollama model that is not one of
 *  Ollama's cloud models. An unknown provider is not claimed as local. */
export function madeHere(provider: string | null | undefined, model: string | null | undefined): boolean {
  const name = (provider ?? "").trim();
  if (name === "local") return true;
  return name === "ollama" && !isOllamaCloudModel(model);
}

/** What the page asks the shell to keep: the question, the answer and the cited pointers, with what the answer's own events said about it.
 *  These come from the page and are not trusted: the shell saves pointers only, reads each source's words from its live row, and decides
 *  for itself (from the stored rows) whether the exchange must stay on this PC. Every field here can only make the exchange MORE private
 *  (a local answer, a held-local question, a reroute to this PC, a provider it does not know); none can make it less. So a missing
 *  provider is sent as empty, which the shell treats as local, and nothing is ever sent as "not local" that the page did not hear.
 *  `answeredLocally` here is "made on this PC for any reason". */
export function savedInput(turn: Turn, requireLocal: boolean, pointers: Pointer[], query: string = turn.query): SavedExchangeInput {
  return {
    query,
    answer: turn.text,
    cited: pointers,
    model: turn.model ?? "",
    provider: turn.provider ?? "",
    answeredLocally: turn.answeredLocally === true || madeHere(turn.provider, turn.model),
    leftOut: turn.leftOut ?? 0,
    requireLocal,
    routedLocally: turn.routedLocally === true,
  };
}

/** The finished exchanges as the meeting Ask's history, oldest first; one with no answer adds nothing. */
export function meetingHistory(turns: readonly Pick<SourcedTurn, "query" | "text" | "status">[]): MeetingAskExchange[] {
  return turns.filter((x) => x.status === "done" && x.text.trim() !== "").map((x) => ({ query: x.query, answer: x.text }));
}

// --- the scope line and the topic chip ------------------------------------------------------------------------------

/** "Pod 2 Team Call, Nov 19": a meeting named in the scope line. */
export function meetingLabel(title: string, startUnixMs: number, locale: string): string {
  const day = new Intl.DateTimeFormat(locale, { month: "short", day: "numeric" }).format(new Date(startUnixMs));
  return `${title}, ${day}`;
}

/** The one line under a conversation's title. Null for a topic or meeting with no name to show: the line is then hidden. */
export function scopeLine(scope: ConversationScopeView, names: { topic?: string | null; meeting?: string | null }): string | null {
  if (scope.kind === "everything") return t("Asking about everything");
  const name = (scope.kind === "topic" ? names.topic : names.meeting)?.trim() ?? "";
  return name === "" ? null : t("Asking about {name}", { name });
}

/** The topics the chip's menu offers: the ones the conversation's sources belong to, in the order cited, each once, without the one it is in. */
export function menuTopics(cited: readonly TopicRefView[], current: TopicRefView | null): TopicRefView[] {
  const seen = new Set<number>(current ? [current.topicId] : []);
  return cited.filter((topic) => !seen.has(topic.topicId) && !!seen.add(topic.topicId));
}

// --- the list -------------------------------------------------------------------------------------------------------

/** Starred first (whatever their age), then most recently updated, then the newer row: the order core lists them in. */
export function sortRows<R extends Pick<ConversationListRow, "id" | "starred" | "updatedAtUnixMs">>(rows: readonly R[]): R[] {
  return [...rows].sort((a, b) => Number(b.starred) - Number(a.starred) || b.updatedAtUnixMs - a.updatedAtUnixMs || b.id - a.id);
}

/** The rows whose title holds `query`, ignoring case and accents; every row for a blank query. */
export function filterRows<R extends Pick<ConversationListRow, "title">>(rows: readonly R[], query: string): R[] {
  const wanted = fold(query.trim());
  return wanted === "" ? [...rows] : rows.filter((r) => fold(r.title).includes(wanted));
}

const fold = (text: string): string => text.normalize("NFD").replace(/\p{M}/gu, "").toLowerCase();

/** When a conversation was last updated, in words ("2 hours ago", "yesterday"). */
export function updatedLabel(updatedAtUnixMs: number, now: number, locale: string): string {
  const seconds = Math.round((updatedAtUnixMs - now) / 1000);
  const rtf = new Intl.RelativeTimeFormat(locale, { numeric: "auto" });
  const steps: [Intl.RelativeTimeFormatUnit, number][] = [["day", 86_400], ["hour", 3_600], ["minute", 60]];
  for (const [unit, size] of steps) if (Math.abs(seconds) >= size) return rtf.format(Math.trunc(seconds / size), unit);
  return rtf.format(0, "second");
}
