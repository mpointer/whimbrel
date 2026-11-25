// Reads results for the Find page: the first page when the words, the filters or the order change, more pages on request. A slower
// answer for words that have since changed is dropped, and a card already listed is not listed twice. While new items still wait for
// their meaning (core's `meaningCatchingUp`), the rows shown are read again and replace what was shown, so the same words never stay on
// an older, shorter answer; rows Load more added are read again with them, never dropped.

import { useCallback, useEffect, useRef, useState } from "react";
import { libraryFind } from "../../api";
import { describeError, isCommandError } from "../../errors";
import type { FindFilterInput, FoundCard, FoundPage } from "../../generated/types";
import { appendUnique, filterKey } from "./findModel";

/** Results asked for at a time; "Load more" asks for the next page. */
export const PAGE_SIZE = 50;
/** The most rows one read returns (core's `FIND_MAX_LIMIT`). */
export const FIND_MAX_LIMIT = 200;

/** How many rows a catch-up read asks for when `loaded` rows have been asked for so far: all of them, at least a page. `null` past
 *  what one read returns, and then the catch-up reads stop, so the rows Load more added stay as they are. */
export function catchUpLimit(loaded: number): number | null {
  const n = Math.max(PAGE_SIZE, loaded);
  return n > FIND_MAX_LIMIT ? null : n;
}

/** The rows to show after a catch-up read that asked for the first `asked` rows: its answer, then the rows loaded past those (Load
 *  more can land while the read is on its way), each once. */
export function mergeCatchUp(shown: readonly FoundCard[], answer: readonly FoundCard[], asked: number): FoundCard[] {
  return appendUnique(answer, shown.slice(asked));
}
/** How many times at most a page is read again while meaning results are catching up: 5 quick reads, then one every 10 s for about
 *  ten minutes. The embedding sweep waits for mains power and a minute without input, so it can be minutes away; a read is one search. */
export const CATCH_UP_TRIES = 65;
/** How long to wait before the read after `tries` reads again. */
export const catchUpDelay = (tries: number): number => (tries < 5 ? 2000 : 10_000);

type Wait = (ms: number, f: () => void) => () => void;
const realWait: Wait = (ms, f) => {
  const id = setTimeout(f, ms);
  return () => clearTimeout(id);
};

/** Reads a page, shows it, and while core says meaning results are still catching up reads it again and shows that instead, at most
 *  [`CATCH_UP_TRIES`] more times. A `read` that returns `null` (nothing to ask for) ends it. Returns a cancel: after it nothing more is
 *  read or shown. */
export function searchUntilCaughtUp(read: () => Promise<FoundPage> | null, show: (page: FoundPage) => void, fail: (e: unknown) => void, wait: Wait = realWait): () => void {
  let live = true;
  let stopTimer: () => void = () => {};
  const run = (tries: number): void => {
    const asked = read();
    if (!asked) return;
    void asked.then(
      (page) => {
        if (!live) return;
        show(page);
        if (page.meaningCatchingUp && tries < CATCH_UP_TRIES) stopTimer = wait(catchUpDelay(tries), () => live && void run(tries + 1));
      },
      (e: unknown) => live && fail(e),
    );
  };
  run(0);
  return () => {
    live = false;
    stopTimer();
  };
}

export interface FindResults {
  results: FoundCard[];
  mentions: FoundCard[];
  /** The first page of the last search: its counts and whether the search model was there. */
  page: FoundPage | null;
  loading: boolean;
  loadingMore: boolean;
  more: boolean;
  error: string | null;
}

const NONE: FindResults = { results: [], mentions: [], page: null, loading: false, loadingMore: false, more: false, error: null };

const message = (e: unknown): string => (isCommandError(e) ? describeError(e) : String(e));

/** `query` null means nothing has been searched. An empty query with a filter lists what the filter holds, by time. */
export function useFindResults(query: string | null, filter: FindFilterInput, order: "time" | "rank"): FindResults & { loadMore: () => void; reload: () => void } {
  const [state, setState] = useState<FindResults>(NONE);
  const token = useRef(0);
  const offset = useRef(0);
  const [round, setRound] = useState(0);
  const key = filterKey(filter);
  const latest = useRef(filter);
  latest.current = filter;

  useEffect(() => {
    token.current += 1;
    const mine = token.current;
    offset.current = 0;
    if (query === null) {
      setState(NONE);
      return;
    }
    setState((s) => ({ ...s, loading: true, error: null }));
    const filter = latest.current;
    // The rows the read on its way asked for: the first read a page, a catch-up read every row loaded so far.
    let asked = PAGE_SIZE;
    return searchUntilCaughtUp(
      () => {
        const n = catchUpLimit(offset.current);
        if (n === null) return null;
        asked = n;
        return libraryFind(query, filter, order, 0, n);
      },
      (page) => {
        if (mine !== token.current) return;
        const first = offset.current === 0;
        // Rows Load more added past what this read asked for stay, and so does its word on whether there is more.
        const kept = !first && offset.current > asked;
        offset.current = Math.max(offset.current, asked);
        setState((s) => ({
          results: first ? page.results : mergeCatchUp(s.results, page.results, asked),
          mentions: first ? page.mentions : mergeCatchUp(s.mentions, page.mentions, asked),
          page,
          loading: false,
          loadingMore: s.loadingMore && !first,
          more: kept ? s.more : page.more,
          error: null,
        }));
      },
      (e: unknown) => mine === token.current && setState({ ...NONE, error: message(e) }),
    );
    // The filter is read through `latest`; its key says when it changed.
  }, [query, key, order, round]);

  const loadMore = useCallback(() => {
    if (query === null) return;
    const mine = token.current;
    setState((s) => ({ ...s, loadingMore: true }));
    libraryFind(query, latest.current, order, offset.current, PAGE_SIZE).then(
      (page) => {
        if (mine !== token.current) return;
        offset.current += PAGE_SIZE;
        setState((s) => ({ ...s, results: appendUnique(s.results, page.results), mentions: appendUnique(s.mentions, page.mentions), loadingMore: false, more: page.more }));
      },
      (e: unknown) => mine === token.current && setState((s) => ({ ...s, loadingMore: false, error: message(e) })),
    );
  }, [query, order]);

  return { ...state, loadMore, reload: () => setRound((n) => n + 1) };
}
