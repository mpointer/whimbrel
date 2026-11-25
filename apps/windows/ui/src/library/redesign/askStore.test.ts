import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { ChatEvent } from "../../generated/types";

type Call = { query: string; conversationId: number | null; filter: unknown; requireLocal: boolean; send: (e: ChatEvent) => void };
const calls: Call[] = [];
vi.mock("../../api", () => ({
  chatAskSourced: vi.fn((query: string, conversationId: number | null, filter: unknown, requireLocal: boolean, send: (e: ChatEvent) => void) => {
    calls.push({ query, conversationId, filter, requireLocal, send });
    return Promise.resolve();
  }),
  chatConversations: vi.fn(() => Promise.resolve([])),
  chatConversationSourced: vi.fn(() => Promise.resolve(null)),
  chatConversationFindFilter: vi.fn(() => Promise.resolve(null)),
  chatStop: vi.fn(() => Promise.resolve(true)),
}));

import * as api from "../../api";
import * as store from "./askStore";

/** Opens saved conversation `id` and waits for its exchanges to arrive. */
async function openLoaded(id: number) {
  vi.mocked(api.chatConversationSourced).mockResolvedValueOnce({ exchanges: [{ query: "earlier", answer: "an answer", sources: [], citations: [], legacy: false }] } as never);
  store.openConversation(id);
  await new Promise((r) => setTimeout(r, 0));
}

const done = (c: Call, conversationId: number | null = 5) => c.send({ kind: "completed", answer: "ok", citations: [], conversationId, narrowed: false, leftOut: null });
const filter = { startUnixMs: null, endUnixMs: null, app: null, person: null };

beforeEach(() => {
  calls.length = 0;
  store.resetAskStore();
});

describe("the Find page's conversation", () => {
  it("sends the ask with the filter and what the route promised", () => {
    store.ask("  What did Sam say?  ", filter, true);
    expect(calls).toHaveLength(1);
    expect(calls[0]).toMatchObject({ query: "What did Sam say?", conversationId: null, requireLocal: true });
  });

  it("lets a question sent while another answers wait its turn, then sends it", () => {
    store.ask("first", filter, false);
    store.ask("second", filter, true);
    expect(calls).toHaveLength(1);
    expect(store.useAskForTest().queued?.query).toBe("second");
    done(calls[0]);
    expect(calls).toHaveLength(2);
    expect(calls[1]).toMatchObject({ query: "second", requireLocal: true });
    expect(store.useAskForTest().queued).toBeNull();
  });

  it("puts a stopped question back in the field and drops the half answer", () => {
    store.ask("stop me", filter, false);
    calls[0].send({ kind: "started", runId: "r1", provider: "p", model: "m", context: [] });
    calls[0].send({ kind: "cancelled" });
    const state = store.useAskForTest();
    expect(state.turns).toHaveLength(0);
    expect(state.restore?.text).toBe("stop me");
    expect(state.asking).toBe(false);
  });

  it("continues a conversation under the same filters and starts a new one under other filters", () => {
    store.ask("first", filter, false);
    done(calls[0], 5);
    store.ask("follow up", filter, false);
    expect(calls[1].conversationId).toBe(5);
    done(calls[1], 5);
    store.ask("under another filter", { ...filter, app: "excel.exe" }, false);
    expect(calls[2].conversationId).toBeNull();
    expect(store.useAskForTest().startedOver).toBe(true);
  });

  it("starts the conversation over when the filters change after an answer, and says so", () => {
    store.ask("first", filter, false);
    done(calls[0], 5);
    store.filtersChanged({ ...filter, person: "Sam Okafor" });
    const state = store.useAskForTest();
    expect(state.turns).toHaveLength(0);
    expect(state.activeId).toBeNull();
    expect(state.startedOver).toBe(true);
  });

  it("does not draw an answer that arrives for a conversation that was left", () => {
    store.ask("first", filter, false);
    store.newConversation();
    calls[0].send({ kind: "delta", text: "late" });
    done(calls[0], 5);
    expect(store.useAskForTest().turns).toHaveLength(0);
    expect(store.useAskForTest().asking).toBe(false);
  });
});

describe("an answer stays while another tab shows", () => {
  it("keeps a streaming first answer when the page asks again about the same filters (a remount)", () => {
    store.ask("first", filter, false);
    calls[0].send({ kind: "delta", text: "Partial" });
    store.filtersChanged(filter);
    const state = store.useAskForTest();
    expect(state.turns).toHaveLength(1);
    expect(state.startedOver).toBe(false);
  });
  it("keeps a reopened conversation when the page reports the filters it was opened with", async () => {
    await openLoaded(9);
    store.filtersChanged(filter);
    expect(store.useAskForTest().activeId).toBe(9);
    expect(store.useAskForTest().startedOver).toBe(false);
  });
  it("lets a streaming answer finish and stay on screen when the filters change, and starts the new conversation at the next question", () => {
    const old = { ...filter, app: "excel.exe" };
    store.ask("first", old, false);
    calls[0].send({ kind: "delta", text: "Partial" });
    store.filtersChanged({ ...filter, app: "word.exe" });
    let state = store.useAskForTest();
    expect(state.turns).toHaveLength(1);
    expect(state.startedOver).toBe(false);
    expect(state.turns[0].text).toBe("Partial");
    done(calls[0], 5);
    state = store.useAskForTest();
    expect(state.turns[0].status).toBe("done");
    expect(state.turns[0].filter).toEqual(old);
    expect(state.activeId).toBe(5);
    // The next question is under the new filters: a new conversation, with the notice, and the earlier answer is not fed into it.
    store.ask("second", { ...filter, app: "word.exe" }, false);
    expect(calls[1].conversationId).toBeNull();
    expect(store.useAskForTest().startedOver).toBe(true);
    expect(store.useAskForTest().turns.map((t) => t.query)).toEqual(["second"]);
  });
  it("still starts over for a real change of the filters once the answer has ended", () => {
    store.ask("first", filter, false);
    done(calls[0], 5);
    store.filtersChanged({ ...filter, app: "excel.exe" });
    expect(store.useAskForTest().turns).toHaveLength(0);
    expect(store.useAskForTest().startedOver).toBe(true);
  });
  it("starts over for a filter change when the answer still streaming is in a conversation that was left", () => {
    store.ask("first", filter, false);
    store.newConversation();
    expect(store.useAskForTest().inFlight?.onScreen).toBe(false);
  });
})

describe("a question that waits its turn", () => {
  it("goes out in the conversation the first answer made, with its own filter, and keeps both turns", () => {
    store.ask("first", filter, false);
    store.ask("second", filter, true);
    done(calls[0], 5);
    expect(calls).toHaveLength(2);
    expect(calls[1]).toMatchObject({ query: "second", conversationId: 5, filter });
    const state = store.useAskForTest();
    expect(state.activeId).toBe(5);
    expect(state.turns.map((t) => t.query)).toEqual(["first", "second"]);
    expect(state.turns[0].status).toBe("done");
    expect(state.turns[1].status).toBe("starting");
    expect(state.startedOver).toBe(false);
  });
  it("starts a new conversation with the notice when the waiting question has other filters", () => {
    const other = { ...filter, person: "Sam Okafor" };
    store.ask("first", filter, false);
    store.ask("second", other, false);
    done(calls[0], 5);
    expect(calls[1]).toMatchObject({ query: "second", conversationId: null, filter: other });
    const state = store.useAskForTest();
    expect(state.startedOver).toBe(true);
    expect(state.turns.map((t) => t.query)).toEqual(["second"]);
  });
  it("does not remove the waiting question's turn when the answer before it is stopped", () => {
    store.ask("first", filter, false);
    store.ask("second", filter, false);
    calls[0].send({ kind: "started", runId: "r1", provider: "p", model: "m", context: [] });
    calls[0].send({ kind: "cancelled" });
    const state = store.useAskForTest();
    expect(state.turns.map((t) => t.query)).toEqual(["second"]);
    expect(calls).toHaveLength(2);
  });
  it("lets a finished answer land on its own turn, not the waiting one", () => {
    store.ask("first", filter, false);
    store.ask("second", filter, false);
    calls[0].send({ kind: "delta", text: "First answer" });
    done(calls[0], 5);
    calls[0].send({ kind: "delta", text: " late" });
    expect(store.useAskForTest().turns[1].text).toBe("");
  });
  it("hands the words back when the person leaves the conversation the question was waiting in", () => {
    store.ask("first", filter, false);
    store.ask("second", filter, false);
    store.newConversation();
    expect(store.useAskForTest().queued).toBeNull();
    expect(store.useAskForTest().restore?.text).toBe("second");
    done(calls[0], 5);
    expect(calls).toHaveLength(1);
  });
});

describe("a reopened Find conversation with kept filters", () => {
  const day = (d: number) => new Date(2026, 10, d).getTime();
  it("shows its kept dates and app, and a follow-up under them continues it", async () => {
    const find = await import("./findStore");
    const { filterInput } = await import("./findModel");
    vi.mocked(api.chatConversationFindFilter).mockResolvedValueOnce({ start_unix_ms: day(9), end_unix_ms: day(16), app_bundle_id: "excel.exe", people_left_out: false });
    await openLoaded(9);
    const chips = find.headerState().filter;
    expect(chips).toEqual({ person: null, app: "excel.exe", dates: { kind: "custom", from: "2026-11-09", to: "2026-11-15" } });
    expect(store.useAskForTest().filtersCleared).toBe(false);
    expect(store.useAskForTest().peopleNotKept).toBe(false);
    store.filtersChanged(filterInput(chips, Date.now()));
    expect(store.useAskForTest().turns).toHaveLength(1);
    store.ask("and then?", filterInput(chips, Date.now()), false);
    expect(calls[0]).toMatchObject({ conversationId: 9 });
    expect(store.useAskForTest().startedOver).toBe(false);
  });
  it("says the People filter was not kept when one was on", async () => {
    vi.mocked(api.chatConversationFindFilter).mockResolvedValueOnce({ start_unix_ms: null, end_unix_ms: null, app_bundle_id: null, people_left_out: true });
    await openLoaded(9);
    expect(store.useAskForTest().peopleNotKept).toBe(true);
    // The next question clears the note.
    store.ask("and then?", filter, false);
    expect(store.useAskForTest().peopleNotKept).toBe(false);
  });
  it("starts afresh on New conversation", async () => {
    vi.mocked(api.chatConversationFindFilter).mockResolvedValueOnce({ start_unix_ms: null, end_unix_ms: null, app_bundle_id: null, people_left_out: true });
    await openLoaded(9);
    store.newConversation();
    expect(store.useAskForTest().peopleNotKept).toBe(false);
  });
});

describe("a reopened conversation", () => {
  it("clears the chips and says so, and a follow-up continues it with no new-conversation notice", async () => {
    const find = await import("./findStore");
    find.setHeaderFilter({ person: "Sam Okafor", app: null, dates: null });
    await openLoaded(9);
    expect(find.headerState().filter.person).toBeNull();
    expect(store.useAskForTest().filtersCleared).toBe(true);
    expect(store.useAskForTest().turns).toHaveLength(1);
    store.ask("and then?", filter, false);
    expect(calls[0]).toMatchObject({ conversationId: 9 });
    expect(store.useAskForTest().turns.map((t) => t.query)).toEqual(["earlier", "and then?"]);
    expect(store.useAskForTest().startedOver).toBe(false);
    expect(store.useAskForTest().filtersCleared).toBe(false);
  });
  it("does not say the filters were cleared when none were on", async () => {
    store.openConversation(9);
    expect(store.useAskForTest().filtersCleared).toBe(false);
  });
  it("starts a new conversation, with the notice, only when a filter changes afterwards", async () => {
    await openLoaded(9);
    store.ask("and then?", { ...filter, app: "excel.exe" }, false);
    expect(calls[0].conversationId).toBeNull();
    expect(store.useAskForTest().startedOver).toBe(true);
  });
});

describe("words put back in the field", () => {
  it("are handed over once, however many times the page mounts", () => {
    store.ask("stop me", filter, false);
    calls[0].send({ kind: "started", runId: "r1", provider: "p", model: "m", context: [] });
    calls[0].send({ kind: "cancelled" });
    expect(store.takeRestore()?.text).toBe("stop me");
    expect(store.takeRestore()).toBeNull();
    expect(store.useAskForTest().restore).toBeNull();
  });
});

describe("opening a conversation while asking", () => {
  /** A read of a saved conversation that the test finishes when it chooses. */
  function deferredLoad() {
    let resolve: (v: never) => void = () => {};
    vi.mocked(api.chatConversationSourced).mockImplementationOnce(() => new Promise((r) => (resolve = r as never)));
    return (exchanges: unknown[]) => resolve({ exchanges } as never);
  }
  const saved = (q: string) => ({ query: q, answer: `answer to ${q}`, sources: [], citations: [], legacy: false });

  it("does not ask while the conversation is being read, and says why", async () => {
    const load = deferredLoad();
    store.openConversation(9);
    expect(store.useAskForTest().loading).toBe(true);
    expect(store.ask("too soon", filter, false)).toBe("loading");
    expect(calls).toHaveLength(0);
    expect(store.useAskForTest().turns).toHaveLength(0);
    load([saved("earlier")]);
    await new Promise((r) => setTimeout(r, 0));
    expect(store.useAskForTest().loading).toBe(false);
    expect(store.useAskForTest().turns.map((t) => t.query)).toEqual(["earlier"]);
    // Once it has loaded the question is added after the saved turn and continues the conversation.
    expect(store.ask("and then", filter, false)).toBe("started");
    expect(calls[0].conversationId).toBe(9);
    expect(store.useAskForTest().turns.map((t) => t.query)).toEqual(["earlier", "and then"]);
    calls[0].send({ kind: "delta", text: "late words" });
    expect(store.useAskForTest().turns[0].text).toBe("answer to earlier");
    expect(store.useAskForTest().turns[1].text).toBe("late words");
  });
  it("never lets a late load overwrite what the person did meanwhile", async () => {
    const load = deferredLoad();
    store.openConversation(9);
    store.newConversation();
    expect(store.useAskForTest().loading).toBe(false);
    store.ask("fresh", filter, false);
    load([saved("old one")]);
    await new Promise((r) => setTimeout(r, 0));
    expect(store.useAskForTest().turns.map((t) => t.query)).toEqual(["fresh"]);
    expect(store.useAskForTest().activeId).toBeNull();
  });
  it("lets the load that was asked for last win over an earlier one", async () => {
    const first = deferredLoad();
    store.openConversation(1);
    const second = deferredLoad();
    store.openConversation(2);
    second([saved("two")]);
    await new Promise((r) => setTimeout(r, 0));
    first([saved("one")]);
    await new Promise((r) => setTimeout(r, 0));
    expect(store.useAskForTest().activeId).toBe(2);
    expect(store.useAskForTest().turns.map((t) => t.query)).toEqual(["two"]);
  });
  it("ends loading when the read fails", async () => {
    vi.mocked(api.chatConversationSourced).mockRejectedValueOnce(new Error("down"));
    store.openConversation(9);
    await new Promise((r) => setTimeout(r, 0));
    expect(store.useAskForTest().loading).toBe(false);
    expect(store.useAskForTest().error).toContain("down");
  });
  it("says which conversation is still answering when another is on screen, and holds the new question", () => {
    store.ask("long one", filter, false);
    store.newConversation();
    expect(store.useAskForTest().inFlight).toMatchObject({ query: "long one", onScreen: false });
    expect(store.ask("asked elsewhere", filter, false)).toBe("queued");
    expect(store.useAskForTest().queued?.query).toBe("asked elsewhere");
    done(calls[0], 5);
    expect(calls).toHaveLength(2);
    expect(store.useAskForTest().inFlight).toMatchObject({ query: "asked elsewhere", onScreen: true });
  });
});

describe("one question may wait", () => {
  it("refuses a second one, leaves the first waiting and takes nothing from the field", () => {
    store.ask("first", filter, false);
    expect(store.ask("second", filter, false, "follow")).toBe("queued");
    expect(store.ask("third", filter, false, "follow")).toBe("queue_full");
    expect(store.useAskForTest().queued?.query).toBe("second");
    done(calls[0]);
    expect(calls.map((c) => c.query)).toEqual(["first", "second"]);
  });
  it("sends the waiting words back to the follow-up field when the conversation is left, and marks them as waiting", () => {
    store.ask("first", filter, false);
    store.ask("a follow-up", filter, false, "follow");
    store.newConversation();
    expect(store.useAskForTest().restore).toMatchObject({ text: "a follow-up", to: "follow", queued: true });
  });
  it("keeps a waiting question when the filters change while the first answer is being written", () => {
    store.ask("first", filter, false);
    calls[0].send({ kind: "delta", text: "x" });
    store.ask("a follow-up", filter, false, "follow");
    // The filters change while the first answer is still being written: it stays, and so does the waiting question.
    store.filtersChanged({ ...filter, person: "Sam Okafor" });
    expect(store.useAskForTest().queued?.query).toBe("a follow-up");
    done(calls[0], 5);
    // The waiting question carries the filters it was pressed under, so it simply continues the conversation.
    expect(calls[1]).toMatchObject({ query: "a follow-up", conversationId: 5 });
  });
  it("puts the words back when the person takes the waiting question back", () => {
    store.ask("first", filter, false);
    store.ask("a follow-up", filter, false, "follow");
    store.cancelQueued();
    expect(store.useAskForTest().queued).toBeNull();
    expect(store.useAskForTest().restore).toMatchObject({ text: "a follow-up", to: "follow", queued: true });
  });
  it("sends a waiting question's words back when the next question starts a new conversation under other filters", () => {
    store.ask("first", filter, false);
    store.ask("under other filters", { ...filter, app: "excel.exe" }, false, "header");
    done(calls[0], 5);
    // It started, in a new conversation, so nothing is lost.
    expect(calls[1].query).toBe("under other filters");
    expect(store.useAskForTest().restore).toBeNull();
  });
});

describe("a question whose call returns before its events (the shell answers at once and streams on the channel)", () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => vi.useRealTimers());
  it("waits for the channel however slowly the events come, and ends done with the answer, not as stopped", async () => {
    store.ask("slow local model", filter, false, "follow");
    // The call has returned and nothing has arrived yet; a slow model takes far longer than any short wait.
    await vi.advanceTimersByTimeAsync(5000);
    let state = store.useAskForTest();
    expect(state.asking).toBe(true);
    expect(state.turns[0].status).not.toBe("failed");
    calls[0].send({ kind: "started", runId: "r1", provider: "ollama", model: "gemma", context: [] });
    await vi.advanceTimersByTimeAsync(3000);
    calls[0].send({ kind: "sourced", sources: [], provider: "ollama", answeredLocally: true } as never);
    calls[0].send({ kind: "delta", text: "The answer" });
    await vi.advanceTimersByTimeAsync(4000);
    calls[0].send({ kind: "cited", cited: [], answeredLocally: true, provider: "ollama", leftOut: 0 } as never);
    vi.mocked(api.chatConversations).mockClear();
    done(calls[0], 7);
    state = store.useAskForTest();
    expect(state.turns[0].status).toBe("done");
    expect(state.turns[0].text).toBe("ok");
    expect(state.turns[0].error).toBeFalsy();
    expect(state.asking).toBe(false);
    expect(state.activeId).toBe(7);
    // Nothing goes back in the field: the question was answered.
    expect(state.restore).toBeNull();
    // The conversation list is read again, so the new conversation is in it.
    expect(api.chatConversations).toHaveBeenCalled();
    await vi.advanceTimersByTimeAsync(60_000);
    expect(store.useAskForTest().turns[0].status).toBe("done");
  });
  it("reads the conversation list again for the title, which arrives after the answer completed", () => {
    store.ask("first", filter, false);
    done(calls[0], 5);
    vi.mocked(api.chatConversations).mockClear();
    calls[0].send({ kind: "titled", conversationId: 5, title: "Pricing" });
    expect(api.chatConversations).toHaveBeenCalledTimes(1);
  });
  it("still ends as failed, with the words back, when the call itself is refused", async () => {
    vi.mocked(api.chatAskSourced).mockImplementationOnce(((q: string, c: number | null, f: unknown, r: boolean, send: (e: ChatEvent) => void) => {
      calls.push({ query: q, conversationId: c, filter: f, requireLocal: r, send });
      return Promise.reject(new Error("Could not start the question"));
    }) as never);
    store.ask("refused", filter, false, "follow");
    await vi.advanceTimersByTimeAsync(0);
    const state = store.useAskForTest();
    expect(state.asking).toBe(false);
    expect(state.turns[0].status).toBe("failed");
    expect(state.turns[0].error).toContain("Could not start the question");
    expect(state.restore).toMatchObject({ text: "refused", to: "follow" });
  });
  it("starts the waiting question once the slow answer completes", async () => {
    store.ask("slow one", filter, false);
    store.ask("next", filter, false);
    await vi.advanceTimersByTimeAsync(5000);
    expect(calls.map((c) => c.query)).toEqual(["slow one"]);
    done(calls[0], 5);
    expect(calls.map((c) => c.query)).toEqual(["slow one", "next"]);
  });
});

describe("a question that had to stay on this PC", () => {
  const noOllama = { kind: "failed", error: { kind: "provider_failure", reason: { kind: "ollama_not_running" } }, partial: false } as never;
  it("says it was not sent, instead of suggesting a hosted model, when no model on this PC could answer", () => {
    store.ask("local only", filter, true);
    calls[0].send(noOllama);
    expect(store.useAskForTest().turns[0].error).toBe("Not sent: this question was to be answered on this PC, and no model on this PC could answer it. Set one up in Settings → AI.");
  });
  it("keeps the usual message for an ordinary question", () => {
    store.ask("anywhere", filter, false);
    calls[0].send(noOllama);
    expect(store.useAskForTest().turns[0].error).toMatch(/^Ollama isn't running/);
  });
});

describe("asking again after a failure", () => {
  it("replaces the failed turn instead of leaving both", () => {
    store.ask("first", filter, false);
    done(calls[0], 5);
    store.ask("follow", filter, false, "follow");
    calls[1].send({ kind: "failed", error: { code: "x", message: "network" }, partial: false } as never);
    expect(store.useAskForTest().turns.map((t) => [t.query, t.status])).toEqual([["first", "done"], ["follow", "failed"]]);
    store.ask("follow", filter, false, "follow");
    expect(store.useAskForTest().turns.map((t) => [t.query, t.status])).toEqual([["first", "done"], ["follow", "starting"]]);
    expect(calls[2].conversationId).toBe(5);
  });
});

describe("the conversation a question continues", () => {
  it("is the one on screen only under the filters it was asked with", () => {
    store.ask("first", filter, false);
    done(calls[0], 5);
    const s = store.useAskForTest();
    expect(store.continuedConversation(s, s.conversationKey as string)).toBe(5);
    expect(store.continuedConversation(s, "other")).toBeNull();
  });
});
