// The Find page's conversation, kept outside the page so an answer keeps arriving while another tab shows, and a question sent
// while one is answering waits its turn instead of being dropped. Components read it with `useAsk`.

import { useSyncExternalStore } from "react";
import { chatAskSourced, chatConversationFindFilter, chatConversationSourced, chatConversations, chatStop } from "../../api";
import { describeError, isCommandError } from "../../errors";
import type { ChatEvent, ConversationRow, FindFilterInput } from "../../generated/types";
import { applySourcedEvent, continuesConversation, newSourcedTurn, placeAsk, turnsOf, type AskField, type QueuedAsk, type SourcedTurn } from "./askSession";
import { hasFilter, NO_FILTER } from "../../today/redesign/homeModel";
import { filterInput, filterKey, keptChips, localOnlyFailure } from "./findModel";
import { headerState, setHeaderFilter } from "./findStore";
import { isBusy } from "../../chat/chatModel";

export interface AskState {
  conversations: ConversationRow[];
  activeId: number | null;
  turns: SourcedTurn[];
  /** A question is in flight, whichever conversation is on screen. */
  asking: boolean;
  /** A question waiting for the answer in progress. */
  queued: QueuedAsk | null;
  /** The conversation was started over because the filters changed. */
  startedOver: boolean;
  /** A saved conversation that kept no filters was opened, and the filter chips that were on were cleared. */
  filtersCleared: boolean;
  /** The reopened conversation was asked under a People filter, which a conversation never keeps. */
  peopleNotKept: boolean;
  error: string | null;
  /** Words to put back in the field they came from (a stopped or failed question, one that was waiting when its conversation was left), with
   *  a number so the same words twice still count. Taken once. */
  restore: { seq: number; text: string; to: AskField; /** It was waiting to be asked: it must not be lost to words typed since. */ queued: boolean } | null;
  /** A saved conversation is being read. Asking waits for it: a question asked now would be drawn under, or over, what arrives. */
  loading: boolean;
  /** The filters the conversation on screen is asked under (their key), or null with no conversation. Recorded when a question starts, so a
   *  first answer still streaming has one before it has an id. A saved conversation keeps no filters: when one is opened the chips are
   *  cleared and this is the key of no filter. */
  conversationKey: string | null;
  /** The answer being written, and whether its conversation is the one on screen (the person may have left it). */
  inFlight: { conversationId: number | null; query: string; onScreen: boolean } | null;
}

const EMPTY: AskState = {
  conversations: [],
  activeId: null,
  turns: [],
  asking: false,
  queued: null,
  startedOver: false,
  filtersCleared: false,
  peopleNotKept: false,
  error: null,
  restore: null,
  loading: false,
  conversationKey: null,
  inFlight: null,
};

let state: AskState = EMPTY;
const listeners = new Set<() => void>();
let view = 0;
let restoreSeq = 0;
const NO_FILTER_KEY = filterKey({ startUnixMs: null, endUnixMs: null, app: null, person: null, personKey: null });

function set(next: Partial<AskState>): void {
  state = { ...state, ...next };
  listeners.forEach((l) => l());
}

const message = (e: unknown): string => (isCommandError(e) ? describeError(e) : String(e));

export const useAsk = (): AskState => useSyncExternalStore((l) => (listeners.add(l), () => listeners.delete(l)), () => state);

/** The conversation a question asked under the filters with this key would continue (the one on screen, once it has answers and was asked
 *  under the same filters), or null. A number, so a component that reads only this is not drawn again for every word that streams in. */
export const continuedConversation = (s: Pick<AskState, "turns" | "activeId" | "conversationKey">, key: string): number | null =>
  s.turns.length > 0 && s.conversationKey !== null && s.conversationKey === key ? s.activeId : null;
export const useContinuedConversation = (key: string): number | null =>
  useSyncExternalStore(
    (l) => (listeners.add(l), () => listeners.delete(l)),
    () => continuedConversation(state, key),
  );

export function loadConversations(): void {
  chatConversations().then(
    (rows) => set({ conversations: rows }),
    (e: unknown) => set({ error: message(e) }),
  );
}
export const setConversations = (rows: ConversationRow[]): void => set({ conversations: rows });
export const setError = (error: string | null): void => set({ error });

/** Words go back to the field they came from: a question that was waiting when its conversation was left, a stopped or failed question. */
function putBack(text: string, to: AskField, queued = false): Pick<AskState, "restore"> {
  return { restore: { seq: ++restoreSeq, text, to, queued } };
}

/** The conversation on screen starts over: nothing of it is drawn, and a stream still running for it is saved by the shell but not shown.
 *  A question waiting in it goes back to its field. */
function startOver(startedOver: boolean, filtersCleared = false): void {
  view += 1;
  const waiting = state.queued;
  set({
    activeId: null,
    turns: [],
    error: null,
    startedOver,
    filtersCleared,
    peopleNotKept: false,
    queued: null,
    loading: false,
    conversationKey: null,
    inFlight: state.inFlight ? { ...state.inFlight, onScreen: false } : null,
    restore: waiting ? putBack(waiting.query, waiting.from, true).restore : state.restore,
  });
}

export function newConversation(): void {
  startOver(false);
}

/** Opens a saved conversation. A Find conversation shows the Dates and Apps chips it kept and its follow-ups are asked under them; a People
 *  filter is never kept, and the page says so. One that kept no filters clears the chips that were on (the page says so). Only a filter
 *  chosen afterwards starts a new conversation. Every conversation on this PC asks about everything: none is scoped to a topic or meeting. */
export function openConversation(id: number): void {
  const cleared = hasFilter(headerState().filter);
  startOver(false, false);
  const token = view;
  if (cleared) setHeaderFilter(NO_FILTER);
  // Asking waits until the conversation has been read: `loading` disables it. A load that arrives after the person has gone elsewhere
  // (another conversation, New conversation, a filter change) finds the view has moved on and changes nothing.
  set({ activeId: id, conversationKey: NO_FILTER_KEY, loading: true });
  // A failed read of the kept filters is read as none: the chips stay cleared and the page says so.
  const keptRead = chatConversationFindFilter(id).catch(() => null);
  Promise.all([chatConversationSourced(id), keptRead]).then(
    ([c, kept]) => {
      if (token !== view) return;
      if (!c) return set({ activeId: null, conversationKey: null, loading: false, filtersCleared: cleared });
      const chips = keptChips(kept);
      // The key comes first, so the chips arriving next read as this conversation's own filters, not a change.
      set({
        turns: turnsOf(c),
        loading: false,
        conversationKey: filterKey(filterInput(chips, Date.now())),
        filtersCleared: cleared && !hasFilter(chips),
        peopleNotKept: kept?.people_left_out === true,
      });
      if (hasFilter(chips)) setHeaderFilter(chips);
    },
    (e: unknown) => {
      if (token === view) set({ error: message(e), loading: false, filtersCleared: cleared });
    },
  );
}

/** The filters changed. An answer made under one set of filters is never fed into a question under another, so a conversation that
 *  has answers starts over, and the page says so. A conversation with no recorded filters, or asked under these ones, is left alone. */
export function filtersChanged(filter: FindFilterInput): void {
  if (state.turns.length === 0 || state.conversationKey === null) return;
  // An answer still being written finishes and stays on screen under the filters it was asked with; the new conversation starts at the
  // next question, which says so (see `begin`).
  if (state.inFlight?.onScreen) return;
  if (!continuesConversation(state.conversationKey, filter)) startOver(true);
}

function begin(q: QueuedAsk): void {
  const same = state.turns.length === 0 || continuesConversation(state.conversationKey, q.filter);
  if (!same) startOver(true);
  // A failed question that is asked again takes the failed one's place instead of standing beside it.
  const before = state.turns;
  const kept = before.length > 0 && before[before.length - 1].status === "failed" ? before.slice(0, -1) : before;
  const token = view;
  const conversationId = state.activeId;
  const index = kept.length;
  /** The turn this question owns: an event for it never lands on another. */
  const update = (f: (turn: SourcedTurn) => SourcedTurn) => {
    if (token !== view || !state.turns[index]) return;
    set({ turns: state.turns.map((turn, i) => (i === index ? f(turn) : turn)) });
  };
  set({
    asking: true,
    inFlight: { conversationId, query: q.query, onScreen: true },
    error: null,
    startedOver: state.startedOver && !same,
    filtersCleared: false,
    peopleNotKept: false,
    conversationKey: filterKey(q.filter),
    turns: [...kept, newSourcedTurn(q.query, q.filter)],
    restore: null,
  });
  let over = false;
  /** The question has ended, however it ended: the next one in line starts, after everything this one changed is in place. */
  const finished = () => {
    if (over) return;
    over = true;
    set({ asking: false, inFlight: null });
    const next = state.queued;
    if (next) {
      set({ queued: null });
      begin(next);
    }
  };
  const fail = (text: string) => {
    if (over) return;
    if (token === view) {
      update((turn) => ({ ...turn, status: "failed", error: text }));
      set(putBack(q.query, q.from));
    }
    finished();
  };
  const handle = (event: ChatEvent) => {
    // The title is made after the answer has completed: the list is read again for it, though the question has ended.
    if (event.kind === "titled") return loadConversations();
    if (over) return;
    const terminal = event.kind === "completed" || event.kind === "cancelled" || event.kind === "failed";
    if (event.kind === "completed") loadConversations();
    if (token === view) {
      if (event.kind === "completed" && conversationId === null && event.conversationId != null) set({ activeId: event.conversationId });
      if (event.kind === "cancelled") {
        // No bubble; the question goes back into the field to edit or send again.
        set({ turns: state.turns.filter((_, i) => i !== index), ...putBack(q.query, q.from) });
      } else {
        if (event.kind === "failed") set(putBack(q.query, q.from));
        // A question that had to stay on this PC never hears it could have gone to a hosted model.
        const local = event.kind === "failed" ? localOnlyFailure(event.error, q.requireLocal) : null;
        update((turn) => {
          const next = applySourcedEvent(turn, event);
          return local ? { ...next, error: local } : next;
        });
      }
    }
    if (terminal) finished();
  };
  // The shell starts the answer and returns at once: the reply says only that it started, never that it ended. The question ends on the
  // channel's own terminal event (completed, cancelled, failed), however long the model takes; only a refused call ends it here.
  chatAskSourced(q.query, conversationId, q.filter, q.requireLocal, handle).catch((e: unknown) => fail(message(e)));
}

export type AskResult = "started" | "queued" | "loading" | "queue_full" | "empty";

/** Sends a question, or lets it wait its turn when another is answering. Only one waits: a second is refused (`queue_full`) and its words
 *  stay where they are. While a saved conversation is being read nothing is asked (`loading`). The result says what happened, so the
 *  caller clears its field only for a question that was taken. */
export function ask(query: string, filter: FindFilterInput, requireLocal: boolean, from: AskField = "header"): AskResult {
  const text = query.trim();
  if (!text) return "empty";
  if (state.loading) return "loading";
  const placed = placeAsk(state.asking, state.queued, { query: text, filter, requireLocal, from });
  if (placed.refused) return "queue_full";
  if (placed.start) {
    begin(placed.start);
    return "started";
  }
  set({ queued: placed.queued });
  return "queued";
}

/** Takes back a question that is waiting: its words go back to the field they came from. */
export function cancelQueued(): void {
  const waiting = state.queued;
  set({ queued: null, ...(waiting ? putBack(waiting.query, waiting.from, true) : {}) });
}

/** Words to put back in a field, once: the page that takes them applies them and they are gone, so coming back to the tab does not
 *  put them there again over whatever was typed since. */
export function takeRestore(): AskState["restore"] {
  const r = state.restore;
  if (r) set({ restore: null });
  return r;
}

/** Stops the streaming answer; it ends with `cancelled`, and the question goes back into the field. */
export function stop(): void {
  const last = state.turns[state.turns.length - 1];
  if (last && isBusy(last) && last.runId) void chatStop(last.runId);
}

/** The state, read without React (a press that has waited on something reads what is true now). */
export const askSnapshot = (): AskState => state;

/** Tests only: the state, read without React. */
export const useAskForTest = (): AskState => state;

/** Tests only: a fresh store. */
export function resetAskStore(): void {
  state = EMPTY;
  view = 0;
  listeners.forEach((l) => l());
}
