// The conversation on the Find page, as pure steps: one turn and how the shell's events change it, a saved conversation as turns, and
// what a question sent while another is answering does. (docs/ui-redesign/screens/ask.md)

import { applyEvent, newTurn, type Turn } from "../../chat/chatModel";
import type { ChatEvent, ContextEntry, FindFilterInput, NumberedSourceView, SourcedConversationView, SourcedExchangeView } from "../../generated/types";
import { filterKey } from "./findModel";

/** One question and its answer on this page: the old turn, plus the numbered list the model was given and which numbers it cited. */
export interface SourcedTurn extends Turn {
  /** Every item the model was given, by number; for a saved answer, the cited ones. */
  sources: NumberedSourceView[];
  /** The numbers the answer cites; null until the answer is complete. */
  cited: number[] | null;
  /** An answer saved before numbered sources: it draws today's chips from these. */
  legacy: boolean;
  legacyCitations: ContextEntry[];
  /** The question names a client excluded from hosted models, so a model on this PC answered it (live answers only; it is not saved). */
  excludedClient: boolean;
  /** Read from a saved conversation, not made on this screen: only what was saved is known about it. */
  saved: boolean;
  /** The filters this question was asked under, so its "From your work memory" line stays true when the chips change while it answers.
   *  Null for a saved answer: its filters were not saved. */
  filter: FindFilterInput | null;
  /** The dates the answer actually drew on, when the core said (the `sourced` event); null when it did not. */
  drewRange: { startUnixMs: number; endUnixMs: number } | null;
}

export function newSourcedTurn(query: string, filter: FindFilterInput | null = null): SourcedTurn {
  return { ...newTurn(query), sources: [], cited: null, legacy: false, legacyCitations: [], excludedClient: false, saved: false, filter, drewRange: null };
}

/** The range the core says an answer drew on, read defensively: the field is optional and its shape is checked before it is believed. */
export function rangeOf(event: unknown): { startUnixMs: number; endUnixMs: number } | null {
  const r = (event as { dateRange?: { startUnixMs?: unknown; endUnixMs?: unknown } | null }).dateRange;
  if (!r || typeof r.startUnixMs !== "number" || typeof r.endUnixMs !== "number") return null;
  return { startUnixMs: r.startUnixMs, endUnixMs: r.endUnixMs };
}

/** How one event changes the turn it belongs to. The old events go through the old model; `sourced` replaces the numbered list
 *  (a stream that falls back to the blocking path numbers its items again), and `cited` says what the answer cites and where it
 *  was made. */
export function applySourcedEvent(turn: SourcedTurn, event: ChatEvent): SourcedTurn {
  switch (event.kind) {
    case "sourced":
      return { ...turn, sources: event.sources, provider: event.provider, answeredLocally: event.answeredLocally, drewRange: rangeOf(event) ?? turn.drewRange };
    case "cited":
      return { ...turn, cited: event.cited, provider: event.provider, answeredLocally: event.answeredLocally };
    case "answered_by":
      return { ...turn, ...applyEvent(turn, event), excludedClient: event.excludedClient };
    case "fell_back":
      return { ...turn, ...applyEvent(turn, event), sources: [], cited: null, drewRange: null };
    default:
      return { ...turn, ...applyEvent(turn, event) };
  }
}

function savedExchange(e: SourcedExchangeView): SourcedTurn {
  const base: SourcedTurn = { ...newSourcedTurn(e.query), saved: true, status: "done", text: e.answer, model: e.model ?? null, provider: e.provider ?? null };
  if (e.legacy) return { ...base, legacy: true, legacyCitations: e.citations, context: e.citations, citations: e.citations.map((c) => c.item) };
  const sources = e.sources.map((s): NumberedSourceView => ({ n: s.n, card: s.card }));
  return {
    ...base,
    sources,
    // The core says which numbers the saved answer cites; an answer saved before that field falls back to its saved sources, which are the cited ones.
    cited: Array.isArray(e.cited) ? e.cited : sources.map((s) => s.n),
    answeredLocally: e.answeredLocally ?? false,
    leftOut: e.leftOut && e.leftOut > 0 ? e.leftOut : null,
  };
}

/** A saved conversation as turns. */
export const turnsOf = (c: SourcedConversationView): SourcedTurn[] => c.exchanges.map(savedExchange);

// --- a question that arrives while another is answering ---------------------------------------------------

export interface QueuedAsk {
  query: string;
  filter: FindFilterInput;
  /** The route preview's word when the button was pressed: the answer must stay on this PC. */
  requireLocal: boolean;
  /** The field the words came from, so they go back there if the question is never asked. */
  from: AskField;
}

/** The two fields a question is typed in: the header's, and the follow-up field under the answers. */
export type AskField = "header" | "follow";

/** What to do with a question: start it now, let it wait its turn (one waits), or refuse it because one already waits. A refused
 *  question is not taken: its words stay in their field and the page says why. Nothing is replaced and nothing is dropped. */
export function placeAsk(busy: boolean, queued: QueuedAsk | null, next: QueuedAsk): { start: QueuedAsk | null; queued: QueuedAsk | null; refused: boolean } {
  if (!busy) return { start: next, queued, refused: false };
  if (queued) return { start: null, queued, refused: true };
  return { start: null, queued: next, refused: false };
}

// --- filters and conversations ---------------------------------------------------------------------------

/** Whether a question under `filter` may carry the conversation so far. An answer made under one set of filters is never fed into a
 *  question under another, so a conversation asked under other filters (or whose filters are not known: one reopened from the list)
 *  starts again. */
export function continuesConversation(conversationFilter: string | null, filter: FindFilterInput): boolean {
  return conversationFilter !== null && conversationFilter === filterKey(filter);
}
