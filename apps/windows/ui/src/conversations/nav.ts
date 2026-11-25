// Which conversation (or the list) the Conversations section shows, and the other things the section's screens tell each other: a request to
// show the section, a change to the data (a generated title landed), the topic the Topics tab shows. Kept outside the components so a button
// on Today or in a meeting note can open a conversation, and the tab that shows it is chosen once by the shell.

import { useSyncExternalStore } from "react";
import type { ConversationScopeView, RoutePreviewView } from "../generated/types";

export type ConversationView =
  | { kind: "list" }
  | { kind: "page"; id: number }
  /** A new conversation, not stored until its first answer is in. `question` is asked straight away. */
  | { kind: "draft"; key: number; scope: ConversationScopeView; name: string | null; question: string | null; shown: RoutePreviewView | null };

export interface NavState {
  view: ConversationView;
  /** Bumped each time something asks for the Conversations section to be shown. */
  request: number;
  /** Bumped when stored conversations changed under the screens (a title landed, a conversation was kept). */
  version: number;
}

let state: NavState = { view: { kind: "list" }, request: 0, version: 0 };
let drafts = 0;
let topicShown: { id: number; name: string } | null = null;
const listeners = new Set<() => void>();
const set = (next: Partial<NavState>) => {
  state = { ...state, ...next };
  listeners.forEach((l) => l());
};

export const useConversationNav = (): NavState => useSyncExternalStore((l) => (listeners.add(l), () => listeners.delete(l)), () => state);
export const navSnapshot = (): NavState => state;

/** A stored conversation's page. */
export function openConversation(id: number): void {
  set({ view: { kind: "page", id }, request: state.request + 1 });
}

/** A new conversation about `scope` (`name`: the topic's name, for its scope line), asking `question` first when there is one. `shown` is the route
 *  preview the Ask control was named from: a promise of this PC in it is kept by the first question (it goes held to this PC). */
export function startConversation(scope: ConversationScopeView, question: string | null, name: string | null = null, shown: RoutePreviewView | null = null): void {
  const words = question?.trim();
  set({ view: { kind: "draft", key: ++drafts, scope, name, question: words ? words : null, shown }, request: state.request + 1 });
}

/** The list. Does not ask for the section to be shown: it is for a page that has a way back. `from` is the conversation the page was showing: the list
 *  puts focus on its row when it comes back (null after a delete, or from a new conversation: focus goes to New conversation). */
export function showConversationList(from: number | null = null): void {
  listReturn = { id: from };
  set({ view: { kind: "list" } });
}

let listReturn: { id: number | null } | null = null;
/** Read once by the list when it appears: where focus goes, or null when the list was not reached from a page. */
export function takeListReturn(): { id: number | null } | null {
  const taken = listReturn;
  listReturn = null;
  return taken;
}

/** Puts focus in the shell's Ask field (the header's search and Ask field), where a person who cleared an answer goes on from. */
export function focusAskField(): void {
  requestAnimationFrame(() => document.querySelector<HTMLInputElement>('[data-section="composer"] input')?.focus());
}

/** Something stored changed: lists and an open page read again. */
export function conversationsChanged(): void {
  set({ version: state.version + 1 });
}

/** The topic the Topics tab shows now, or null when it shows the list (an Ask from a topic page asks about that topic). */
export function setTopicShown(topic: { id: number; name: string } | null): void {
  topicShown = topic;
}
export const topicShownNow = (): { id: number; name: string } | null => topicShown;

/** Tests only. */
export function resetNav(): void {
  state = { view: { kind: "list" }, request: 0, version: 0 };
  listReturn = null;
  drafts = 0;
  topicShown = null;
  listeners.forEach((l) => l());
}
