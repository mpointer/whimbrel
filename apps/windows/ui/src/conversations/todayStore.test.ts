import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { ChatEvent, RoutePreviewView } from "../generated/types";
import { navSnapshot, resetNav } from "./nav";
import { askOnToday, canContinue, clearToday, continueToday, resetToday, retryToday, stopToday, todayDeps, todaySnapshot } from "./todayStore";

interface Call {
  query: string;
  conversationId: number | null;
  requireLocal: boolean;
  save: boolean | undefined;
  send: (e: ChatEvent) => void;
}
const calls: Call[] = [];
const saved: unknown[] = [];

const local: RoutePreviewView = { destination: "local", local: true, provider: "ollama", model: "gemma", reason: "x", requireLocal: true, mayFallBack: false };
const hosted: RoutePreviewView = { destination: "hosted", local: false, provider: "anthropic", model: "claude", reason: "x", requireLocal: false, mayFallBack: false };

const answer = (c: Call, text = "It was agreed [1].") => {
  c.send({ kind: "started", runId: "r1", provider: "ollama", model: "gemma", context: [] });
  c.send({ kind: "sourced", sources: [{ n: 1, card: { kind: "capture", id: 9, found: true, title: "t" } as never }], provider: "ollama", answeredLocally: true, dateRange: null });
  c.send({ kind: "cited", cited: [1], answeredLocally: true, provider: "ollama", leftOut: 0 });
  c.send({ kind: "completed", answer: text, citations: [], conversationId: null, narrowed: false, leftOut: null });
};
const tick = () => new Promise((r) => setTimeout(r, 0));

beforeEach(() => {
  calls.length = 0;
  saved.length = 0;
  resetToday();
  resetNav();
  todayDeps.ask = ((query, conversationId, _filter, requireLocal, send, save) => {
    calls.push({ query, conversationId, requireLocal, send, save });
    return Promise.resolve();
  }) as typeof todayDeps.ask;
  todayDeps.stop = vi.fn(() => Promise.resolve(true));
  todayDeps.preview = vi.fn(() => Promise.resolve(hosted));
  todayDeps.save = ((id, scope, exchange) => {
    saved.push({ id, scope, exchange });
    return Promise.resolve({ conversationId: 12, topic: null, citedTopics: [] });
  }) as typeof todayDeps.save;
});
afterEach(() => resetToday());

describe("Ask on Today", () => {
  it("asks about everything and saves nothing", async () => {
    await askOnToday("  What did Sam say?  ", null);
    expect(calls).toHaveLength(1);
    expect(calls[0]).toMatchObject({ query: "What did Sam say?", conversationId: null, save: false, requireLocal: false });
    expect(todaySnapshot().question).toBe("What did Sam say?");
  });

  it("holds the question to this PC when the control promised it and the route still says so", async () => {
    todayDeps.preview = vi.fn(() => Promise.resolve(local));
    await askOnToday("q", local);
    expect(calls[0].requireLocal).toBe(true);
  });

  it("sends nothing when where the answer would come from changed since the control was named", async () => {
    todayDeps.preview = vi.fn(() => Promise.resolve(hosted));
    await askOnToday("q", local);
    expect(calls).toHaveLength(0);
    expect(todaySnapshot().note).toMatch(/has changed/);
  });

  it("replaces the answer when asked again, and a late event from the old question changes nothing", async () => {
    await askOnToday("first", null);
    answer(calls[0], "First answer");
    await askOnToday("second", null);
    expect(todaySnapshot().question).toBe("second");
    expect(todaySnapshot().turn?.text).toBe("");
    // The old question's channel is still open: what it says is not drawn.
    calls[0].send({ kind: "delta", text: "stale" });
    expect(todaySnapshot().turn?.text).toBe("");
    answer(calls[1], "Second answer");
    expect(todaySnapshot().turn?.text).toBe("Second answer");
  });

  it("stops a question still streaming when another replaces it", async () => {
    await askOnToday("first", null);
    calls[0].send({ kind: "started", runId: "run-a", provider: "ollama", model: "m", context: [] });
    await askOnToday("second", null);
    expect(todayDeps.stop).toHaveBeenCalledWith("run-a");
  });

  it("clears: the area empties, a stream is stopped and late events are dropped", async () => {
    await askOnToday("q", null);
    calls[0].send({ kind: "started", runId: "r9", provider: "ollama", model: "m", context: [] });
    clearToday();
    expect(todaySnapshot().question).toBeNull();
    expect(todayDeps.stop).toHaveBeenCalledWith("r9");
    calls[0].send({ kind: "delta", text: "late" });
    expect(todaySnapshot().turn).toBeNull();
  });

  it("stops a run that has no id yet as soon as the id arrives: on a new question, on Clear and on Stop", async () => {
    // A new question replaces one that has not started.
    await askOnToday("first", null);
    await askOnToday("second", null);
    expect(todayDeps.stop).not.toHaveBeenCalled();
    calls[0].send({ kind: "started", runId: "run-a", provider: "ollama", model: "m", context: [] });
    expect(todayDeps.stop).toHaveBeenCalledWith("run-a");
    expect(todaySnapshot().turn?.query).toBe("second");
    // Clear before the run started.
    (todayDeps.stop as ReturnType<typeof vi.fn>).mockClear();
    clearToday();
    expect(todayDeps.stop).not.toHaveBeenCalled();
    calls[1].send({ kind: "started", runId: "run-b", provider: "ollama", model: "m", context: [] });
    expect(todayDeps.stop).toHaveBeenCalledWith("run-b");
    expect(todaySnapshot().question).toBeNull();
    // Stop before the run started.
    (todayDeps.stop as ReturnType<typeof vi.fn>).mockClear();
    await askOnToday("third", null);
    stopToday();
    expect(todayDeps.stop).not.toHaveBeenCalled();
    calls[2].send({ kind: "started", runId: "run-c", provider: "ollama", model: "m", context: [] });
    expect(todayDeps.stop).toHaveBeenCalledTimes(1);
    expect(todayDeps.stop).toHaveBeenCalledWith("run-c");
  });

  it("does not stop an answer that finished", async () => {
    await askOnToday("q", null);
    answer(calls[0]);
    clearToday();
    expect(todayDeps.stop).not.toHaveBeenCalled();
  });

  it("empties the area when the answer is stopped", async () => {
    await askOnToday("q", null);
    calls[0].send({ kind: "started", runId: "r1", provider: "ollama", model: "m", context: [] });
    calls[0].send({ kind: "cancelled" });
    expect(todaySnapshot().question).toBeNull();
  });

  it("retries a failed question held to this PC the way it was first asked", async () => {
    todayDeps.preview = vi.fn(() => Promise.resolve(local));
    await askOnToday("q", local);
    calls[0].send({ kind: "failed", error: { kind: "provider_failure" } as never, partial: false });
    expect(todaySnapshot().turn?.status).toBe("failed");
    retryToday();
    expect(calls).toHaveLength(2);
    expect(calls[1]).toMatchObject({ query: "q", requireLocal: true, save: false });
  });
});

describe("Continue as a conversation", () => {
  it("is offered for a finished answer only, once", async () => {
    await askOnToday("q", null);
    expect(canContinue(todaySnapshot())).toBe(false);
    answer(calls[0]);
    expect(canContinue(todaySnapshot())).toBe(true);
  });

  it("keeps the question and answer as a conversation with its pointers, no second model call, then clears Today and opens it", async () => {
    await askOnToday("What was agreed?", null);
    answer(calls[0]);
    const id = await continueToday();
    expect(id).toBe(12);
    expect(calls).toHaveLength(1);
    expect(saved).toEqual([
      {
        id: null,
        scope: { kind: "everything" },
        exchange: { query: "What was agreed?", answer: "It was agreed [1].", cited: [{ n: 1, kind: "capture", id: 9 }], model: "gemma", provider: "ollama", answeredLocally: true, leftOut: 0, requireLocal: false, routedLocally: false },
      },
    ]);
    expect(todaySnapshot().question).toBeNull();
    expect(navSnapshot().view).toEqual({ kind: "page", id: 12 });
    expect(navSnapshot().request).toBeGreaterThan(0);
  });

  it("never makes a second conversation from one answer", async () => {
    await askOnToday("q", null);
    answer(calls[0]);
    const first = continueToday();
    expect(await continueToday()).toBeNull();
    await first;
    expect(saved).toHaveLength(1);
  });

  it("puts the button back and says so when nothing was stored", async () => {
    todayDeps.save = (() => Promise.reject("The memory database could not be read.")) as typeof todayDeps.save;
    await askOnToday("q", null);
    answer(calls[0]);
    expect(await continueToday()).toBeNull();
    await tick();
    expect(todaySnapshot().continued).toBe(false);
    expect(todaySnapshot().note).toBeTruthy();
    expect(todaySnapshot().question).toBe("q");
  });
});
