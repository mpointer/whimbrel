import { describe, expect, it } from "vitest";
import type { FoundPage } from "../../generated/types";
import { CATCH_UP_TRIES, catchUpDelay, catchUpLimit, mergeCatchUp, PAGE_SIZE, searchUntilCaughtUp } from "./useFind";

const page = (ids: number[], meaningCatchingUp: boolean): FoundPage => ({
  results: ids.map((id) => ({ kind: "mcp_item", id, found: true, title: `Item ${id}`, text: "", tsUnixMs: id })),
  mentions: [],
  total: ids.length,
  mentionsTotal: 0,
  counts: {},
  semanticUsed: true,
  semanticAvailable: true,
  more: false,
  capped: false,
  meaningCatchingUp,
});

/** Runs the timers by hand: each scheduled call waits until `tick`. */
function clock() {
  const queue: (() => void)[] = [];
  return { wait: (_ms: number, f: () => void) => (queue.push(f), () => queue.splice(queue.indexOf(f), 1)), tick: () => queue.shift()?.(), pending: () => queue.length };
}
const flush = () => new Promise((r) => setTimeout(r, 0));

describe("a search while new items wait for their meaning", () => {
  it("searches again and replaces the keyword-only answer with the caught-up one", async () => {
    const answers = [page([1, 2], true), page([3, 4, 5], false)];
    const shown: number[][] = [];
    const c = clock();
    searchUntilCaughtUp(() => Promise.resolve(answers.shift()!), (p) => shown.push(p.results.map((r) => r.id)), () => {}, c.wait);
    await flush();
    expect(shown).toEqual([[1, 2]]);
    expect(c.pending()).toBe(1);
    c.tick();
    await flush();
    expect(shown).toEqual([[1, 2], [3, 4, 5]]);
    expect(c.pending()).toBe(0);
  });

  it("stops when the words change, and gives up after a bounded number of tries", async () => {
    const c = clock();
    let calls = 0;
    const cancel = searchUntilCaughtUp(() => (calls++, Promise.resolve(page([1], true))), () => {}, () => {}, c.wait);
    await flush();
    cancel();
    expect(c.pending()).toBe(0);
    c.tick();
    await flush();
    expect(calls).toBe(1);

    calls = 0;
    const d = clock();
    searchUntilCaughtUp(() => (calls++, Promise.resolve(page([1], true))), () => {}, () => {}, d.wait);
    for (let i = 0; i < CATCH_UP_TRIES + 5; i++) {
      await flush();
      d.tick();
    }
    await flush();
    expect(calls).toBe(CATCH_UP_TRIES + 1);
  });

  it("reads quickly at first, then slowly for minutes: the sweep waits for an idle, plugged-in PC", () => {
    expect(catchUpDelay(0)).toBe(2000);
    expect(catchUpDelay(4)).toBe(2000);
    expect(catchUpDelay(5)).toBe(10_000);
    const total = Array.from({ length: CATCH_UP_TRIES }, (_, i) => catchUpDelay(i)).reduce((a, b) => a + b, 0);
    expect(total).toBeGreaterThanOrEqual(10 * 60_000);
  });
});

describe("a catch-up read after Load more", () => {
  const cards = (ids: number[]) => page(ids, false).results;
  const ids = (c: { id: number }[]) => c.map((r) => r.id);

  it("reads every row already loaded, at least a page, and stops past what one read returns", () => {
    expect(catchUpLimit(0)).toBe(PAGE_SIZE);
    expect(catchUpLimit(PAGE_SIZE)).toBe(PAGE_SIZE);
    expect(catchUpLimit(2 * PAGE_SIZE)).toBe(2 * PAGE_SIZE);
    expect(catchUpLimit(4 * PAGE_SIZE)).toBe(200);
    expect(catchUpLimit(5 * PAGE_SIZE)).toBeNull();
  });

  it("keeps the rows loaded since the read was asked for, after its answer", () => {
    // The read asked for 2 rows; Load more had added two more meanwhile.
    expect(ids(mergeCatchUp(cards([1, 2, 3, 4]), cards([9, 1]), 2))).toEqual([9, 1, 3, 4]);
    // A read that covered every row replaces them.
    expect(ids(mergeCatchUp(cards([1, 2]), cards([9, 1]), 2))).toEqual([9, 1]);
    // A row the answer moved up is not listed twice.
    expect(ids(mergeCatchUp(cards([1, 2, 3, 4]), cards([3, 1]), 2))).toEqual([3, 1, 4]);
  });

  it("stops reading when the read says there is nothing to ask for", async () => {
    const c = clock();
    let calls = 0;
    const shown: number[][] = [];
    let loaded = 0;
    searchUntilCaughtUp(
      () => (calls++ === 0 ? Promise.resolve(page([1], true)) : null),
      (p) => {
        shown.push(ids(p.results));
        loaded = 300;
      },
      () => {},
      c.wait,
    );
    await flush();
    expect(loaded).toBe(300);
    c.tick();
    await flush();
    expect(calls).toBe(2);
    expect(shown).toEqual([[1]]);
    expect(c.pending()).toBe(0);
  });
});
