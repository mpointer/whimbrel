// One conversation page's state and its questions (screen 6, part 3 to 5), without React so it can be tested. A page is a stored conversation
// (`open`) or a new one that is stored when its first answer is in (`startDraft`). A follow-up is asked with the conversation's earlier
// exchanges as context and with its scope: everything (the Find page's sourced Ask, with the Dates and App the conversation kept), one topic
// (`chat_ask_topic`) or one meeting (`meeting_ask`). An everything answer in a stored conversation is saved by the shell itself; every other
// answer is kept by the page through `conversation_save_exchange`, which saves pointers to its sources and lets the conversation join a topic.

import {
  chatAskSourced,
  chatAskTopic,
  chatConversationFindFilter,
  chatDeleteConversation,
  chatRenameConversation,
  chatRoutePreview,
  chatStarConversation,
  chatStop,
  conversationPage,
  conversationSaveExchange,
  conversationSetTopic,
  meetingAsk,
} from "../api";
import type { ConversationPageView, ConversationScopeView, FindFilterInput, RoutePreviewView, TopicRefView } from "../generated/types";
import { t } from "../l10n";
import { newSourcedTurn, turnsOf, type SourcedTurn } from "../library/redesign/askSession";
import { filterInput, filtersOnLine, keptChips, resolveSend } from "../library/redesign/findModel";
import { meetingHistory, meetingLabel, meetingPointers, pointersOf, savedInput, scopeLine } from "./model";
import { conversationsChanged, openConversation, showConversationList } from "./nav";
import { inFlight, messageOf, NO_FILTER_INPUT, runTurn, type RunHandle, type TurnEnd } from "./runTurn";

export interface PageState {
  status: "loading" | "ready" | "missing";
  /** The stored conversation's id; null for a new one that is not stored yet. */
  id: number | null;
  scope: ConversationScopeView;
  title: string;
  starred: boolean;
  scopeLine: string | null;
  /** A conversation asked under a People filter says the filter was not kept. */
  filterNote: string | null;
  /** The Dates and App an everything conversation kept, which its follow-ups are asked under. */
  filter: FindFilterInput;
  topic: TopicRefView | null;
  topicSetByUser: boolean;
  citedTopics: TopicRefView[];
  topics: TopicRefView[];
  /** The exchanges, oldest first. The last may be in flight or failed. */
  turns: SourcedTurn[];
  /** The question in flight is held to this PC, so a follow-up never falls back (everything scope only). */
  busy: boolean;
  /** The last question failed (its words are in the last turn): Try again asks it once more. */
  failed: boolean;
  /** The topic held nothing for this question: the page offers to ask about everything instead. */
  nothingInScope: string | null;
  /** Why something the person asked for did not happen (a note or a failed keep), in words. */
  note: string | null;
  /** Words to put back in the composer (a stopped question), numbered so the same words twice still count. */
  restore: { seq: number; text: string } | null;
}

/** What the controller calls; tests replace them. */
export const pageDeps = {
  page: conversationPage,
  findFilter: chatConversationFindFilter,
  preview: chatRoutePreview,
  askSourced: chatAskSourced,
  askTopic: chatAskTopic,
  askMeeting: meetingAsk,
  stop: chatStop,
  save: conversationSaveExchange,
  setTopic: conversationSetTopic,
  rename: chatRenameConversation,
  star: chatStarConversation,
  remove: chatDeleteConversation,
  now: () => Date.now(),
  locale: () => (typeof navigator !== "undefined" ? navigator.language : "en-US"),
};

const EVERYTHING: ConversationScopeView = { kind: "everything" };

const initial = (): PageState => ({
  status: "loading",
  id: null,
  scope: EVERYTHING,
  title: "",
  starred: false,
  scopeLine: null,
  filterNote: null,
  filter: NO_FILTER_INPUT,
  topic: null,
  topicSetByUser: false,
  citedTopics: [],
  topics: [],
  turns: [],
  busy: false,
  failed: false,
  nothingInScope: null,
  note: null,
  restore: null,
});

/** Whether each saved exchange shows the answered-locally line: one the shell kept on this PC for an excluded client does, and so does every
 *  later one (the conversation holds that client's content, so every follow-up is answered here too). */
export function carriedLocal(own: readonly boolean[]): boolean[] {
  let any = false;
  return own.map((flag) => (any = any || flag));
}

/** The stored exchanges as turns, with the excluded-client line on the ones that earn it. */
export function turnsOfPage(page: ConversationPageView): SourcedTurn[] {
  const shown = carriedLocal(page.localOnly);
  return turnsOf(page.conversation).map((turn, i) => (shown[i] ? { ...turn, answeredLocally: true, excludedClient: true, saved: false } : turn));
}

export interface PageController {
  state(): PageState;
  subscribe(listener: () => void): () => void;
  open(id: number): Promise<void>;
  startDraft(scope: ConversationScopeView, name: string | null, question: string | null, shown?: RoutePreviewView | null): void;
  /** Re-reads a stored conversation (a title landed, a keep finished). Does nothing while a question is in flight. */
  refresh(): Promise<void>;
  /** True when the question went out; false when it was refused (busy, empty, or the route no longer says what the button promised). */
  ask(query: string, shown?: RoutePreviewView | null): Promise<boolean>;
  askEverythingInstead(): Promise<boolean>;
  retry(): void;
  stop(): void;
  rename(title: string): Promise<void>;
  toggleStar(): Promise<void>;
  setTopic(topicId: number | null): Promise<void>;
  /** Deletes the conversation and leaves the page for the list. Refused while a question is in flight. */
  remove(): Promise<boolean>;
  /** The page went away: a question in flight is stopped and its late events are dropped. */
  dispose(): void;
}

export function createPageController(): PageController {
  let state = initial();
  const listeners = new Set<() => void>();
  let disposed = false;
  /** Changes with every question, open and dispose, so late work for something no longer on screen is dropped. */
  let serial = 0;
  let restoreSeq = 0;
  /** The Dates and App an everything conversation kept (its follow-ups are asked under them). */
  let kept: FindFilterInput = NO_FILTER_INPUT;
  /** The question in flight was asked held to this PC. */
  let heldLocal = false;
  /** The scope the last question was asked with (not always the page's: "Ask about everything instead" asks everything). */
  let lastScope: ConversationScopeView = EVERYTHING;
  /** The run in flight, so Stop and leaving the page can stop it even before its id has arrived. */
  let current: RunHandle | null = null;
  const set = (next: Partial<PageState>) => {
    if (disposed) return;
    state = { ...state, ...next };
    listeners.forEach((l) => l());
  };
  const setLast = (turn: SourcedTurn) => {
    const turns = state.turns.slice();
    if (turns.length === 0) turns.push(turn);
    else turns[turns.length - 1] = turn;
    set({ turns });
  };

  const names = (p: { scopeTopicName?: string | null; scopeMeetingTitle?: string | null; scopeMeetingStartUnixMs?: number | null }) => ({
    topic: p.scopeTopicName ?? null,
    meeting: p.scopeMeetingTitle ? meetingLabel(p.scopeMeetingTitle, p.scopeMeetingStartUnixMs ?? 0, pageDeps.locale()) : null,
  });

  async function load(id: number, mark: number): Promise<void> {
    const [page, filter] = await Promise.all([pageDeps.page(id), pageDeps.findFilter(id).catch(() => null)]);
    if (mark !== serial || disposed) return;
    if (!page) return set({ status: "missing" });
    // Find filters belong to an everything conversation only.
    const keptFilter = page.scope.kind === "everything" ? filter : null;
    const chips = keptChips(keptFilter);
    kept = keptFilter ? filterInput(chips, pageDeps.now()) : NO_FILTER_INPUT;
    const filtersLine = keptFilter ? filtersOnLine(chips) : "";
    set({
      status: "ready",
      id,
      scope: page.scope,
      title: page.conversation.title,
      starred: page.conversation.starred,
      scopeLine: filtersLine ? t("Asking about {name}", { name: filtersLine }) : scopeLine(page.scope, names(page)),
      filter: kept,
      filterNote: keptFilter?.people_left_out === true ? t("The People filter was not kept with this conversation, so follow-ups are not limited to a person.") : null,
      topic: page.topic ?? null,
      topicSetByUser: page.topicSetByUser,
      citedTopics: page.citedTopics,
      topics: page.topics,
      turns: turnsOfPage(page),
    });
  }

  /** Runs one question in the conversation's scope. `scope` is the scope asked about (the page's own, or everything for "Ask about everything instead"). */
  function run(query: string, scope: ConversationScopeView, requireLocal: boolean): void {
    const mark = ++serial;
    heldLocal = requireLocal;
    lastScope = scope;
    const earlier = state.turns.filter((x) => x.status === "done");
    const id = state.id;
    const filter = scope.kind === "everything" ? kept : NO_FILTER_INPUT;
    // The shell saves an everything answer in a stored conversation; every other answer is kept by the page after it ends.
    const shellSaves = scope.kind === "everything" && id !== null;
    set({ busy: true, failed: false, nothingInScope: null, note: null, turns: [...earlier, newSourcedTurn(query)] });
    const start = (onEvent: Parameters<typeof pageDeps.askTopic>[4]) => {
      switch (scope.kind) {
        case "everything":
          return pageDeps.askSourced(query, id, filter, requireLocal, onEvent, shellSaves ? undefined : false);
        case "topic":
          return pageDeps.askTopic(scope.id, query, id, requireLocal, onEvent);
        case "meeting":
          return pageDeps.askMeeting(scope.id, query, meetingHistory(earlier), onEvent);
      }
    };
    current = runTurn(query, start, {
      requireLocal,
      stopRun: (runId) => void pageDeps.stop(runId),
      onTurn: (turn) => {
        if (mark === serial) setLast(turn);
      },
      onTitled: (conversationId, title) => {
        if (state.id === conversationId && !disposed) set({ title });
        conversationsChanged();
      },
      onEnd: (end, turn) => {
        if (mark === serial) void ended(end, turn, query, scope, shellSaves, mark);
      },
    });
  }

  async function ended(end: TurnEnd, turn: SourcedTurn, query: string, scope: ConversationScopeView, shellSaves: boolean, mark: number): Promise<void> {
    if (end === "failed") return set({ busy: false, failed: true });
    if (end === "cancelled") {
      // Stopped: nothing is kept, and the question goes back into the composer.
      return set({ busy: false, turns: state.turns.slice(0, -1), restore: { seq: ++restoreSeq, text: query } });
    }
    if (end === "scope_empty") return set({ busy: false, turns: state.turns.slice(0, -1), nothingInScope: query });
    // Completed.
    if (shellSaves && state.id !== null) {
      set({ busy: false });
      await load(state.id, mark);
      conversationsChanged();
      return;
    }
    const pointers = scope.kind === "meeting" ? meetingPointers(turn.text, turn.citations) : pointersOf(turn);
    try {
      const saved = await pageDeps.save(state.id, state.scope, savedInput(turn, heldLocal, pointers), (event) => {
        if (event.kind === "titled") conversationsChanged();
      });
      if (mark !== serial || disposed) return;
      conversationsChanged();
      if (state.id === null) {
        // The first answer of a new conversation: the stored conversation takes the page's place.
        set({ busy: false });
        return openConversation(saved.conversationId);
      }
      set({ busy: false, topic: saved.topic ?? state.topic, citedTopics: saved.citedTopics });
      await load(state.id, mark);
    } catch (e) {
      if (mark !== serial || disposed) return;
      if (state.id === null) {
        // Nothing was stored: the answer goes and the question waits with an error, so the next try keeps it with the row.
        set({ busy: false, failed: true, turns: [{ ...newSourcedTurn(query), status: "failed", error: t("Couldn't save the conversation. Try again.") }] });
      } else {
        set({ busy: false, note: messageOf(e) });
      }
    }
  }

  const controller: PageController = {
    state: () => state,
    subscribe(listener) {
      listeners.add(listener);
      return () => void listeners.delete(listener);
    },
    async open(id) {
      const mark = ++serial;
      set({ ...initial(), status: "loading", id });
      try {
        await load(id, mark);
      } catch {
        if (mark === serial) set({ status: "missing" });
      }
    },
    startDraft(scope, name, question, shown = null) {
      ++serial;
      kept = NO_FILTER_INPUT;
      state = {
        ...initial(),
        status: "ready",
        scope,
        title: t("New conversation"),
        scopeLine: scopeLine(scope, { topic: name, meeting: null }),
      };
      listeners.forEach((l) => l());
      if (question) void controller.ask(question, shown);
    },
    async refresh() {
      if (state.id === null || state.busy || state.status !== "ready") return;
      const mark = serial;
      try {
        await load(state.id, mark);
      } catch {
        /* the page keeps what it shows */
      }
    },
    async ask(query, shown = null) {
      const words = query.trim();
      if (!words || state.status !== "ready" || state.busy) return false;
      set({ busy: true, note: null });
      let requireLocal = false;
      if (state.scope.kind === "everything" || state.scope.kind === "topic") {
        // The Ask button's promise, checked against the route the call makes: a question held to this PC never reaches a hosted model.
        // A topic Ask is held the same way (`chat_ask_topic`'s `require_local`); a meeting Ask names no place and is never held.
        const decision = await resolveSend(shown, () => (state.scope.kind === "topic" ? pageDeps.preview(words, null, NO_FILTER_INPUT) : pageDeps.preview(words, state.id, kept)));
        if (disposed) return false;
        if (!decision.go) {
          set({ busy: false, note: t("Where this answer would come from has changed since the button was drawn. Check the button, then ask again.") });
          return false;
        }
        requireLocal = decision.requireLocal;
      }
      if (disposed) return false;
      run(words, state.scope, requireLocal);
      return true;
    },
    async askEverythingInstead() {
      const query = state.nothingInScope;
      if (!query || state.busy) return false;
      // The same question once more about everything. The conversation's scope stays. A topic question held to this PC stays held (the words
      // were promised a place, whatever scope they are asked in); otherwise the route is read the way every other send reads it.
      const held = heldLocal;
      set({ busy: true, note: null });
      const decision = await resolveSend(null, () => pageDeps.preview(query, state.id, kept));
      if (disposed) return false;
      if (!decision.go) {
        set({ busy: false, note: t("Where this answer would come from has changed since the button was drawn. Check the button, then ask again.") });
        return false;
      }
      run(query, EVERYTHING, held || decision.requireLocal);
      return true;
    },
    retry() {
      const last = state.turns[state.turns.length - 1];
      if (!state.failed || !last || state.busy) return;
      run(last.query, lastScope, heldLocal);
    },
    stop() {
      const last = state.turns[state.turns.length - 1];
      if (last && inFlight(last)) current?.stop();
    },
    async rename(title) {
      const next = title.trim();
      if (state.id === null || next === "" || next === state.title) return;
      const before = state.title;
      const id = state.id;
      set({ title: next });
      try {
        await pageDeps.rename(id, next);
        conversationsChanged();
      } catch {
        if (state.title === next) set({ title: before });
      }
    },
    async toggleStar() {
      if (state.id === null) return;
      const wanted = !state.starred;
      const id = state.id;
      set({ starred: wanted });
      try {
        await pageDeps.star(id, wanted);
        conversationsChanged();
      } catch {
        if (state.starred === wanted) set({ starred: !wanted });
      }
    },
    async setTopic(topicId) {
      if (state.id === null) return;
      try {
        const link = await pageDeps.setTopic(state.id, topicId);
        set({ topic: link.topic ?? null, topicSetByUser: link.setByUser });
        conversationsChanged();
      } catch (e) {
        set({ note: messageOf(e) });
      }
    },
    async remove() {
      if (state.id === null || state.busy) return false;
      try {
        await pageDeps.remove(state.id);
      } catch (e) {
        set({ note: messageOf(e) });
        return false;
      }
      conversationsChanged();
      // Deleted: the row it had is gone, so focus goes to New conversation.
      showConversationList(null);
      return true;
    },
    dispose() {
      ++serial;
      const last = state.turns[state.turns.length - 1];
      if (last && inFlight(last)) current?.stop();
      disposed = true;
      listeners.clear();
    },
  };
  return controller;
}
