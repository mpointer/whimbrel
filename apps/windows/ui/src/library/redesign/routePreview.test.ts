import { describe, expect, it, vi } from "vitest";
import type { FindFilterInput, RoutePreviewView } from "../../generated/types";
import { createPreviewCache, previewFor, previewKey } from "./routePreview";

const filter: FindFilterInput = { startUnixMs: null, endUnixMs: null, app: null, person: null, personKey: null };
const view = (destination: string): RoutePreviewView => ({ destination, local: destination === "local", requireLocal: destination === "local", mayFallBack: false, provider: "ollama", model: "m" }) as RoutePreviewView;

function setup() {
  let clock = 1000;
  const pending: Array<{ resolve: (v: RoutePreviewView) => void; reject: (e: unknown) => void }> = [];
  const fetch = vi.fn(() => new Promise<RoutePreviewView>((resolve, reject) => pending.push({ resolve, reject })));
  const cache = createPreviewCache(fetch, () => clock, 5000);
  return { cache, fetch, pending, tick: (ms: number) => (clock += ms) };
}

describe("the route preview cache", () => {
  it("shares one request among callers asking the same thing at once", async () => {
    const { cache, fetch, pending } = setup();
    const a = cache.read("hello", null, filter);
    const b = cache.read("hello", null, filter);
    expect(fetch).toHaveBeenCalledTimes(1);
    pending[0].resolve(view("local"));
    expect((await a)?.destination).toBe("local");
    expect((await b)?.destination).toBe("local");
  });
  it("answers from the last result for a few seconds, then asks again", async () => {
    const { cache, fetch, pending, tick } = setup();
    const first = cache.read("hello", null, filter);
    pending[0].resolve(view("local"));
    await first;
    await cache.read("hello", null, filter);
    expect(fetch).toHaveBeenCalledTimes(1);
    tick(6000);
    const again = cache.read("hello", null, filter);
    expect(fetch).toHaveBeenCalledTimes(2);
    pending[1].resolve(view("local"));
    await again;
  });
  it("keeps a result for the words, the conversation and the filter it was read for, and no other", async () => {
    const { cache, fetch, pending } = setup();
    const a = cache.read("hello", null, filter);
    pending[0].resolve(view("local"));
    await a;
    void cache.read("hello there", null, filter);
    void cache.read("hello", 4, filter);
    void cache.read("hello", null, { ...filter, app: "excel.exe" });
    expect(fetch).toHaveBeenCalledTimes(4);
  });
  it("never lets a cached local outlive a settings change", async () => {
    const { cache, fetch, pending } = setup();
    const a = cache.read("hello", null, filter);
    pending[0].resolve(view("local"));
    await a;
    cache.clear();
    const b = cache.read("hello", null, filter);
    expect(fetch).toHaveBeenCalledTimes(2);
    pending[1].resolve(view("hosted"));
    expect((await b)?.destination).toBe("hosted");
  });
  it("drops a read that was still on its way when the settings changed", async () => {
    const { cache, fetch, pending } = setup();
    const stale = cache.read("hello", null, filter);
    cache.clear();
    pending[0].resolve(view("local"));
    await stale;
    void cache.read("hello", null, filter);
    expect(fetch).toHaveBeenCalledTimes(2);
  });
  it("reads fresh when asked to, whatever is cached or on its way", async () => {
    const { cache, fetch, pending } = setup();
    const a = cache.read("hello", null, filter);
    pending[0].resolve(view("local"));
    await a;
    const fresh = cache.read("hello", null, filter, { fresh: true });
    expect(fetch).toHaveBeenCalledTimes(2);
    pending[1].resolve(view("hosted"));
    expect((await fresh)?.destination).toBe("hosted");
  });
  it("does not keep a failed read", async () => {
    const { cache, fetch, pending } = setup();
    const a = cache.read("hello", null, filter);
    pending[0].reject(new Error("down"));
    await expect(a).rejects.toThrow("down");
    void cache.read("hello", null, filter);
    expect(fetch).toHaveBeenCalledTimes(2);
  });
});

describe("a label's preview", () => {
  it("is shown only for the words, the filters and the conversation it was read for", () => {
    const entry = { key: previewKey("hello", 5, filter), view: view("local") };
    expect(previewFor(entry, previewKey("hello", 5, filter))?.destination).toBe("local");
    expect(previewFor(entry, previewKey(" hello ", 5, filter))?.destination).toBe("local");
    expect(previewFor(entry, previewKey("hello again", 5, filter))).toBeNull();
    expect(previewFor(entry, previewKey("hello", null, filter))).toBeNull();
    expect(previewFor(entry, previewKey("hello", 5, { ...filter, app: "excel.exe" }))).toBeNull();
    expect(previewFor(null, previewKey("hello", 5, filter))).toBeNull();
  });
});

describe("clearing the cache", () => {
  it("tells whoever is showing a kept preview, so it reads again, and stops telling one that left", () => {
    const { cache } = setup();
    const told = vi.fn();
    const off = cache.subscribe(told);
    cache.clear();
    expect(told).toHaveBeenCalledTimes(1);
    off();
    cache.clear();
    expect(told).toHaveBeenCalledTimes(1);
  });
});

describe("a label and the route the press read", () => {
  it("tells whoever watches about every read, a fresh one included, so the button names what the question was sent as", async () => {
    const { cache, pending } = setup();
    const seen: Array<[string, string | undefined]> = [];
    cache.watch((key, v) => seen.push([key, v?.destination]));
    const fresh = cache.read("hello", null, filter, { fresh: true });
    pending[0].resolve(view("local"));
    await fresh;
    expect(seen).toEqual([[previewKey("hello", null, filter), "local"]]);
  });
});
