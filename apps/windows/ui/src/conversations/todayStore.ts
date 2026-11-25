// Ask on Today (screen 6, part 1 and 2): one answer under the bar. Asking again replaces it; it is held here until it is cleared, replaced or the
// app quits, so it stays while another section shows. Nothing is saved unless it is continued as a conversation (then through the shell's keep
// call, with no second model call). Kept outside the components for that reason.

import { useSyncExternalStore } from "react";
import { chatAskSourced, chatRoutePreview, chatStop, conversationSaveExchange } from "../api";
import type { RoutePreviewView } from "../generated/types";
import { t } from "../l10n";
import { newSourcedTurn, type SourcedTurn } from "../library/redesign/askSession";
import { resolveSend } from "../library/redesign/findModel";
import { conversationsChanged, openConversation } from "./nav";
import { pointersOf, savedInput } from "./model";
import { inFlight, messageOf, NO_FILTER_INPUT, runTurn, type RunHandle } from "./runTurn";

export interface TodayState {
  /** The question on screen ("You asked"); null while the area is empty. */
  question: string | null;
  /** The answer as it streams and once it is in; null before the first event of a question. */
  turn: SourcedTurn | null;
  /** The question went out held to this PC (the route preview promised it). */
  requireLocal: boolean;
  /** Continue was pressed for this answer: never a second conversation from it. */
  continued: boolean;
  /** Why the last ask or Continue did not go, in words. */
  note: string | null;
}

const EMPTY: TodayState = { question: null, turn: null, requireLocal: false, continued: false, note: null };
let state: TodayState = EMPTY;
const listeners = new Set<() => void>();
/** Changes with every ask and clear, so late work for an answer no longer on screen is dropped. */
let serial = 0;

/** The run on screen (or the last one), so Stop, Clear and a new question can stop it even before its id has arrived. */
let current: RunHandle | null = null;

const set = (next: Partial<TodayState>) => {
  state = { ...state, ...next };
  listeners.forEach((l) => l());
};

/** What the store calls; tests replace them. */
export const todayDeps = { ask: chatAskSourced, stop: chatStop, preview: chatRoutePreview, save: conversationSaveExchange };

export const useToday = (): TodayState => useSyncExternalStore((l) => (listeners.add(l), () => listeners.delete(l)), () => state);
export const todaySnapshot = (): TodayState => state;

/** Continue is offered for a finished answer, and once. */
export const canContinue = (s: Pick<TodayState, "turn" | "continued">): boolean => !!s.turn && s.turn.status === "done" && s.turn.text.trim() !== "" && !s.continued;

/** The answer in flight can be stopped. */
export const busy = (s: Pick<TodayState, "turn">): boolean => inFlight(s.turn);

/** Shows the question and starts it. A question still streaming is stopped; its late events are dropped (they belong to an earlier serial). */
function begin(words: string, requireLocal: boolean, mark: number): void {
  stopRunning();
  set({ question: words, turn: newSourcedTurn(words), requireLocal, continued: false, note: null });
  current = runTurn(words, (onEvent) => todayDeps.ask(words, null, NO_FILTER_INPUT, requireLocal, onEvent, false), {
    requireLocal,
    stopRun: (runId) => void todayDeps.stop(runId),
    onTurn: (turn) => {
      if (mark === serial) set({ turn });
    },
    // Stopped: nothing is kept and the area empties.
    onEnd: (end) => {
      if (end === "cancelled" && mark === serial) clearToday();
    },
  });
}

/** Asks `text` about everything, replacing what the area shows. `shown` is the route preview the Ask control was named from: if where the answer
 *  would come from has changed since (or the control promised this PC and the route no longer says so), nothing is sent and a note says why. */
export async function askOnToday(text: string, shown: RoutePreviewView | null = null): Promise<void> {
  const words = text.trim();
  if (!words) return;
  const mark = ++serial;
  const route = await resolveSend(shown, () => todayDeps.preview(words, null, NO_FILTER_INPUT));
  // Cleared, or asked again, while the route was read.
  if (mark !== serial) return;
  if (!route.go) {
    set({ note: t("Where this answer would come from has changed since the button was drawn. Check the button, then ask again.") });
    return;
  }
  begin(words, route.requireLocal, mark);
}

/** The failed question, once more. */
export function retryToday(): void {
  // The same question, held to this PC again when it was.
  if (state.question) begin(state.question, state.requireLocal, ++serial);
}

/** Stops the run in flight. One whose id has not arrived yet is remembered as stopped and stopped the moment it does. */
function stopRunning(): void {
  if (state.turn && inFlight(state.turn)) current?.stop();
}

/** Stops the answer in flight (it ends as stopped and the area empties). */
export function stopToday(): void {
  stopRunning();
}

/** Empties the area; a question still streaming is stopped. */
export function clearToday(): void {
  serial += 1;
  stopRunning();
  state = EMPTY;
  listeners.forEach((l) => l());
}

/** The shown question and answer become a conversation's first exchange (no second model call), the conversation opens and the area clears.
 *  Pressed once: the button is off from the first press, so one row at most. A failure puts the button back and says so. */
export async function continueToday(): Promise<number | null> {
  if (!canContinue(state) || !state.turn) return null;
  const mark = serial;
  const exchange = savedInput(state.turn, state.requireLocal, pointersOf(state.turn));
  set({ continued: true, note: null });
  try {
    // The page supplies what the answer's own events said (provider, model, answered here); the shell saves pointers only.
    const kept = await todayDeps.save(null, { kind: "everything" }, exchange, (event) => {
      if (event.kind === "titled") conversationsChanged();
    });
    if (mark === serial) clearToday();
    conversationsChanged();
    openConversation(kept.conversationId);
    return kept.conversationId;
  } catch (e) {
    if (mark === serial) set({ continued: false, note: messageOf(e) });
    return null;
  }
}

/** Tests only. */
export function resetToday(): void {
  serial += 1;
  current = null;
  state = EMPTY;
  listeners.forEach((l) => l());
}
