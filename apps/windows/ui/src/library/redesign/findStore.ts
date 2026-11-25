// The field in the shell's header and the filters under it belong to the Find page as much as to the header: the words typed and the
// chips chosen are read by both, on Home and on Find, and a search asked for from any tab reaches the page even when it is not yet
// on screen. This is that shared state, kept outside React so neither side has to hand it to the other.

import { useSyncExternalStore } from "react";
import { NO_FILTER, type SearchFilter } from "../../today/redesign/homeModel";

export interface FindRequest {
  /** Bumped for each request, so the same words twice still run twice. */
  seq: number;
  /** `search` shows results and sends nothing anywhere; `ask` also writes an answer. */
  mode: "search" | "ask";
}

export interface HeaderState {
  /** The words in the header's field. */
  text: string;
  filter: SearchFilter;
  request: FindRequest | null;
  /** The Find page is showing: the header draws the filter chips and the count. */
  findShown: boolean;
  /** The count under the chips, from the page's last search. */
  count: string | null;
  /** When the filter was chosen: the dates it means are worked out from this moment, by the header and by the page alike. */
  filterAt: number;
  /** The words the page last searched, kept here so coming back to the tab shows the same results. Null: nothing searched. */
  searched: string | null;
  order: "time" | "rank";
}

const fresh = (): HeaderState => ({ text: "", filter: NO_FILTER, request: null, findShown: false, count: null, filterAt: 0, searched: null, order: "time" });
let state: HeaderState = fresh();
const listeners = new Set<() => void>();
let seq = 0;

function set(next: Partial<HeaderState>): void {
  state = { ...state, ...next };
  listeners.forEach((l) => l());
}

export const useHeader = (): HeaderState => useSyncExternalStore((l) => (listeners.add(l), () => listeners.delete(l)), () => state);
export const headerState = (): HeaderState => state;

export function setHeaderText(text: string): void {
  if (state.text !== text) set({ text });
}
export const setHeaderFilter = (filter: SearchFilter): void => set({ filter, filterAt: Date.now() });
export const setSearched = (searched: string | null): void => {
  if (state.searched !== searched) set({ searched });
};
export const setOrder = (order: HeaderState["order"]): void => {
  if (state.order !== order) set({ order });
};
export function setFindShown(findShown: boolean): void {
  if (state.findShown !== findShown) set({ findShown, ...(findShown ? {} : { count: null }) });
}
export function setFindCount(count: string | null): void {
  if (state.count !== count) set({ count });
}

/** Asks the page to search (or ask) with the words and filters held now. */
export function requestFind(mode: FindRequest["mode"]): void {
  set({ request: { seq: ++seq, mode } });
}

/** A search or a question from somewhere else (Home's starters, the tray, a memory block's Open): the words and filters go into the
 *  header and the page runs them. */
export function requestFindWith(text: string, mode: FindRequest["mode"], filter?: SearchFilter): void {
  set({ text, ...(filter ? { filter, filterAt: Date.now() } : {}) });
  requestFind(mode);
}

/** The request waiting for the page, if any, without taking it (a question that has to wait for the AI settings to be read). */
export function peekRequest(): FindRequest | null {
  const r = state.request;
  return r && r.seq > handled ? r : null;
}

/** The request the page has already run, kept here so a page that mounts after the request (the tab was not showing) runs it once. */
let handled = 0;
export function takeRequest(): FindRequest | null {
  const r = state.request;
  if (!r || r.seq <= handled) return null;
  handled = r.seq;
  return r;
}

/** Tests only. */
export function resetFindStore(): void {
  state = fresh();
  handled = 0;
  seq = 0;
  listeners.forEach((l) => l());
}
