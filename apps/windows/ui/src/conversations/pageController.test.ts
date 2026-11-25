import { beforeEach, describe, expect, it, vi } from "vitest";
import type { ChatEvent, ConversationPageView, ConversationScopeView, RoutePreviewView } from "../generated/types";
import { navSnapshot, resetNav } from "./nav";
import { carriedLocal, createPageController, pageDeps, turnsOfPage } from "./pageController";

type Sent = (e: ChatEvent) => void;
const asked: { kind: "sourced" | "topic" | "meeting"; args: unknown[]; send: Sent }[] = [];
const kept: { id: number | null; scope: ConversationScopeView; exchange: Record<string, unknown> }[] = [];

const topic = (topicId: number, name: string) => ({ topicId, key: `k${topicId}`, name });
const exchange = (query: string, answer: string, sources: unknown[] = []) => ({ query, answer, createdAtUnixMs: 1, model: "m", legacy: false, citations: [], provider: "ollama", answeredLocally: false, leftOut: 0, cited: sources.map((_, i) => i + 1), sources });

function page(over: Partial<ConversationPageView> = {}, exchanges = [exchange("first?", "First [1].", [{ n: 1, card: { kind: "capture", id: 4, found: true, title: "t" } }])]): ConversationPageView {
  return {
    conversation: { id: 5, title: "Pod 2 notes", starred: false, exchanges } as never,
    localOnly: exchanges.map(() => false),
    scope: { kind: "everything" },
    topic: null,
    topicSetByUser: false,
    citedTopics: [],
    topics: [],
    ...over,
  };
}

const local: RoutePreviewView = { destination: "local", local: true, provider: "ollama", model: "gemma", reason: "x", requireLocal: true, mayFallBack: false };
const finish = (c: { send: Sent }, text = "Done [1].", cited = [1]) => {
  c.send({ kind: "started", runId: "r1", provider: "ollama", model: "gemma", context: [] });
  c.send({ kind: "sourced", sources: [{ n: 1, card: { kind: "capture", id: 8, found: true, title: "t" } as never }], provider: "ollama", answeredLocally: true, dateRange: null });
  c.send({ kind: "cited", cited, answeredLocally: true, provider: "ollama", leftOut: 0 });
  c.send({ kind: "completed", answer: text, citations: [{ kind: "transcript", id: 77 }], conversationId: null, narrowed: false, leftOut: null });
};
const settle = () => new Promise((r) => setTimeout(r, 0));

beforeEach(() => {
  asked.length = 0;
  kept.length = 0;
  resetNav();
  pageDeps.page = vi.fn(() => Promise.resolve(page()));
  pageDeps.findFilter = vi.fn(() => Promise.resolve(null));
  pageDeps.preview = vi.fn(() => Promise.resolve({ ...local, destination: "hosted", local: false, requireLocal: false }));
  pageDeps.askSourced = ((...args: unknown[]) => (asked.push({ kind: "sourced", args, send: args[4] as Sent }), Promise.resolve())) as never;
  pageDeps.askTopic = ((...args: unknown[]) => (asked.push({ kind: "topic", args, send: args[4] as Sent }), Promise.resolve())) as never;
  pageDeps.askMeeting = ((...args: unknown[]) => (asked.push({ kind: "meeting", args, send: args[3] as Sent }), Promise.resolve())) as never;
  pageDeps.stop = vi.fn(() => Promise.resolve(true));
  pageDeps.save = ((id: number | null, scope: ConversationScopeView, ex: Record<string, unknown>) => {
    kept.push({ id, scope, exchange: ex });
    return Promise.resolve({ conversationId: id ?? 31, topic: topic(2, "FUEL"), citedTopics: [topic(2, "FUEL")] });
  }) as never;
  pageDeps.setTopic = vi.fn((_id: number, topicId: number | null) => Promise.resolve({ topic: topicId === null ? null : topic(topicId, `t${topicId}`), setByUser: true })) as never;
  pageDeps.rename = vi.fn(() => Promise.resolve([])) as never;
  pageDeps.star = vi.fn(() => Promise.resolve([])) as never;
  pageDeps.remove = vi.fn(() => Promise.resolve([])) as never;
  pageDeps.locale = () => "en-US";
});

describe("a stored conversation", () => {
  it("reads its page: exchanges, scope line, topic link", async () => {
    pageDeps.page = vi.fn(() => Promise.resolve(page({ scope: { kind: "topic", id: 2 }, scopeTopicName: "FUEL", topic: topic(2, "FUEL") }))) as never;
    const c = createPageController();
    await c.open(5);
    const s = c.state();
    expect(s.status).toBe("ready");
    expect(s.title).toBe("Pod 2 notes");
    expect(s.scopeLine).toBe("Asking about FUEL");
    expect(s.topic?.name).toBe("FUEL");
    expect(s.turns).toHaveLength(1);
    expect(s.turns[0].cited).toEqual([1]);
  });

  it("is missing when the conversation is gone", async () => {
    pageDeps.page = vi.fn(() => Promise.resolve(null));
    const c = createPageController();
    await c.open(5);
    expect(c.state().status).toBe("missing");
  });

  it("names a meeting in the scope line by title and day", async () => {
    pageDeps.page = vi.fn(() => Promise.resolve(page({ scope: { kind: "meeting", id: 3 }, scopeMeetingTitle: "Pod 2 Team Call", scopeMeetingStartUnixMs: Date.UTC(2026, 10, 19, 12) }))) as never;
    const c = createPageController();
    await c.open(5);
    expect(c.state().scopeLine).toBe("Asking about Pod 2 Team Call, Nov 19");
  });

  it("carries the Dates and App an everything conversation kept into its follow-ups, and says when a People filter was not kept", async () => {
    pageDeps.findFilter = vi.fn(() => Promise.resolve({ start_unix_ms: Date.UTC(2026, 10, 9), end_unix_ms: Date.UTC(2026, 10, 16), app_bundle_id: "com.slack", people_left_out: true }));
    const c = createPageController();
    await c.open(5);
    expect(c.state().scopeLine).toMatch(/^Asking about Apps: /);
    expect(c.state().filterNote).toMatch(/People filter was not kept/);
    await c.ask("and then?");
    expect(asked[0].kind).toBe("sourced");
    expect(asked[0].args[2]).toMatchObject({ app: "com.slack", startUnixMs: Date.UTC(2026, 10, 9), endUnixMs: Date.UTC(2026, 10, 16), person: null });
  });
});

describe("follow-ups carry the conversation's scope", () => {
  it("everything: asks the sourced Ask with the conversation id, held to this PC when the button said so, and lets the shell save it", async () => {
    pageDeps.preview = vi.fn(() => Promise.resolve(local));
    const c = createPageController();
    await c.open(5);
    await c.ask("and then?", local);
    expect(asked[0]).toMatchObject({ kind: "sourced" });
    expect(asked[0].args.slice(0, 4)).toEqual(["and then?", 5, expect.anything(), true]);
    // `save` is left out: the shell saves an everything answer in a stored conversation.
    expect(asked[0].args[5]).toBeUndefined();
    finish(asked[0]);
    await settle();
    expect(kept).toHaveLength(0);
    expect(pageDeps.page).toHaveBeenCalledTimes(2);
  });

  it("everything: sends nothing when the route changed under the button's promise", async () => {
    const c = createPageController();
    await c.open(5);
    await c.ask("and then?", local);
    expect(asked).toHaveLength(0);
    expect(c.state().note).toMatch(/has changed/);
    expect(c.state().busy).toBe(false);
  });

  it("topic: asks that topic with the conversation id and keeps the answer with its pointers; the topic it joined is shown", async () => {
    // The first read has no topic link; the read after the answer is kept shows the one the conversation joined.
    pageDeps.page = vi
      .fn()
      .mockResolvedValueOnce(page({ scope: { kind: "topic", id: 2 }, scopeTopicName: "FUEL" }))
      .mockResolvedValue(page({ scope: { kind: "topic", id: 2 }, scopeTopicName: "FUEL", topic: topic(2, "FUEL") })) as never;
    const c = createPageController();
    await c.open(5);
    expect(c.state().topic).toBeNull();
    await c.ask("what next?");
    expect(asked[0]).toMatchObject({ kind: "topic" });
    expect(asked[0].args.slice(0, 3)).toEqual([2, "what next?", 5]);
    finish(asked[0]);
    await settle();
    expect(kept[0]).toMatchObject({ id: 5, scope: { kind: "topic", id: 2 } });
    expect(kept[0].exchange).toMatchObject({ query: "what next?", cited: [{ n: 1, kind: "capture", id: 8 }], provider: "ollama", answeredLocally: true });
    expect(c.state().topic?.name).toBe("FUEL");
    expect(c.state().busy).toBe(false);
  });

  it("meeting: asks that meeting with the earlier exchanges as history and keeps citations numbered as the marks", async () => {
    pageDeps.page = vi.fn(() => Promise.resolve(page({ scope: { kind: "meeting", id: 3 } }))) as never;
    const c = createPageController();
    await c.open(5);
    await c.ask("who owns it?");
    expect(asked[0]).toMatchObject({ kind: "meeting" });
    expect(asked[0].args.slice(0, 3)).toEqual([3, "who owns it?", [{ query: "first?", answer: "First [1]." }]]);
    asked[0].send({ kind: "started", runId: "r2", provider: "ollama", model: "gemma", context: [] });
    asked[0].send({ kind: "completed", answer: "Sam does [1].", citations: [{ kind: "transcript", id: 77 }], conversationId: null, narrowed: false, leftOut: null });
    await settle();
    expect(kept[0]).toMatchObject({ id: 5, scope: { kind: "meeting", id: 3 } });
    expect(kept[0].exchange.cited).toEqual([{ n: 1, kind: "transcript", id: 77 }]);
  });

  it("a topic that holds nothing says so, and Ask about everything instead asks the same question about everything", async () => {
    pageDeps.page = vi.fn(() => Promise.resolve(page({ scope: { kind: "topic", id: 2 } }))) as never;
    const c = createPageController();
    await c.open(5);
    await c.ask("anything on pricing?");
    asked[0].send({ kind: "scope_empty" });
    expect(c.state().nothingInScope).toBe("anything on pricing?");
    expect(c.state().turns).toHaveLength(1);
    expect(await c.askEverythingInstead()).toBe(true);
    expect(asked[1]).toMatchObject({ kind: "sourced" });
    expect(asked[1].args.slice(0, 2)).toEqual(["anything on pricing?", 5]);
    expect(asked[1].args[3]).toBe(false);
    expect(c.state().scope).toEqual({ kind: "topic", id: 2 });
  });

  it("Ask about everything instead stays held to this PC when the topic question was held, and reads the route when it was not", async () => {
    pageDeps.page = vi.fn(() => Promise.resolve(page({ scope: { kind: "topic", id: 2 } }))) as never;
    // Held: the topic question went out held to this PC, so the everything question does too.
    pageDeps.preview = vi.fn(() => Promise.resolve(local));
    const held = createPageController();
    await held.open(5);
    await held.ask("what is blocked?", local);
    expect(asked[0].args[3]).toBe(true);
    asked[0].send({ kind: "scope_empty" });
    await held.askEverythingInstead();
    expect(asked[1]).toMatchObject({ kind: "sourced" });
    expect(asked[1].args.slice(0, 4)).toEqual(["what is blocked?", 5, expect.anything(), true]);
    // Held even when the route read now says hosted: the words were promised this PC.
    asked.length = 0;
    const kept2 = createPageController();
    pageDeps.preview = vi.fn(() => Promise.resolve(local));
    await kept2.open(5);
    await kept2.ask("again?", local);
    asked[0].send({ kind: "scope_empty" });
    pageDeps.preview = vi.fn(() => Promise.resolve({ ...local, destination: "hosted", local: false, requireLocal: false } as RoutePreviewView));
    await kept2.askEverythingInstead();
    expect(asked[1].args[3]).toBe(true);
    // Not held: the same route check every send makes (the route is read), and its answer decides.
    asked.length = 0;
    const open = createPageController();
    pageDeps.preview = vi.fn(() => Promise.resolve({ ...local, destination: "hosted", local: false, requireLocal: false } as RoutePreviewView));
    await open.open(5);
    await open.ask("not held", null);
    asked[0].send({ kind: "scope_empty" });
    pageDeps.preview = vi.fn(() => Promise.resolve(local));
    await open.askEverythingInstead();
    expect(pageDeps.preview).toHaveBeenCalledTimes(1);
    expect(asked[1].args[3]).toBe(true);
  });

  it("a retried everything-instead question asks everything again, not the topic", async () => {
    pageDeps.page = vi.fn(() => Promise.resolve(page({ scope: { kind: "topic", id: 2 } }))) as never;
    const c = createPageController();
    await c.open(5);
    await c.ask("q");
    asked[0].send({ kind: "scope_empty" });
    await c.askEverythingInstead();
    asked[1].send({ kind: "failed", error: { kind: "provider_failure" } as never, partial: false });
    c.retry();
    expect(asked[2]).toMatchObject({ kind: "sourced" });
  });

  it("a stopped question puts its words back and keeps nothing", async () => {
    const c = createPageController();
    await c.open(5);
    await c.ask("stop me");
    asked[0].send({ kind: "started", runId: "r5", provider: "ollama", model: "m", context: [] });
    c.stop();
    expect(pageDeps.stop).toHaveBeenCalledWith("r5");
    asked[0].send({ kind: "cancelled" });
    expect(c.state().turns).toHaveLength(1);
    expect(c.state().restore?.text).toBe("stop me");
    expect(c.state().busy).toBe(false);
  });

  it("a failed question can be tried again", async () => {
    const c = createPageController();
    await c.open(5);
    await c.ask("flaky");
    asked[0].send({ kind: "failed", error: { kind: "provider_failure" } as never, partial: false });
    expect(c.state().failed).toBe(true);
    c.retry();
    expect(asked).toHaveLength(2);
    expect(asked[1].args[0]).toBe("flaky");
  });

  it("asks one question at a time", async () => {
    const c = createPageController();
    await c.open(5);
    await c.ask("one");
    await c.ask("two");
    expect(asked).toHaveLength(1);
  });
});

describe("asking says whether the question went out", () => {
  it("is true when the question started and false when it was refused, so the composer keeps the words", async () => {
    const c = createPageController();
    await c.open(5);
    expect(await c.ask("   ")).toBe(false);
    // The route changed under the button's promise: nothing is sent, the note says why, and the answer is false.
    expect(await c.ask("and then?", local)).toBe(false);
    expect(asked).toHaveLength(0);
    expect(c.state().note).toMatch(/has changed/);
    expect(c.state().busy).toBe(false);
    expect(await c.ask("and then?", null)).toBe(true);
    expect(asked).toHaveLength(1);
    // One question at a time: a second is refused while the first runs.
    expect(await c.ask("two", null)).toBe(false);
  });
});

describe("a question that has no run id yet is stopped when the id arrives", () => {
  it("Stop before the run started stops it as soon as it has an id, once", async () => {
    const c = createPageController();
    await c.open(5);
    await c.ask("slow");
    c.stop();
    expect(pageDeps.stop).not.toHaveBeenCalled();
    asked[0].send({ kind: "started", runId: "r9", provider: "ollama", model: "m", context: [] });
    expect(pageDeps.stop).toHaveBeenCalledTimes(1);
    expect(pageDeps.stop).toHaveBeenCalledWith("r9");
    asked[0].send({ kind: "cancelled" });
    expect(c.state().restore?.text).toBe("slow");
  });

  it("leaving the page before the run started stops it when the id arrives, and its events change nothing", async () => {
    const c = createPageController();
    await c.open(5);
    await c.ask("slow");
    c.dispose();
    expect(pageDeps.stop).not.toHaveBeenCalled();
    asked[0].send({ kind: "started", runId: "r4", provider: "ollama", model: "m", context: [] });
    expect(pageDeps.stop).toHaveBeenCalledWith("r4");
    expect(kept).toHaveLength(0);
  });

  it("a run whose id is already known is stopped at once, and a finished one is not stopped", async () => {
    const c = createPageController();
    await c.open(5);
    await c.ask("one");
    asked[0].send({ kind: "started", runId: "r1", provider: "ollama", model: "m", context: [] });
    c.stop();
    expect(pageDeps.stop).toHaveBeenCalledWith("r1");
    asked[0].send({ kind: "cancelled" });
    (pageDeps.stop as ReturnType<typeof vi.fn>).mockClear();
    await c.ask("two");
    finish(asked[1], "Done [1].");
    await settle();
    c.stop();
    c.dispose();
    expect(pageDeps.stop).not.toHaveBeenCalled();
  });
});

describe("the local promise, per scope", () => {
  const hostedPreview = { ...local, destination: "hosted", local: false, requireLocal: false } as RoutePreviewView;
  it("holds a topic question to this PC when the route says local, and not otherwise", async () => {
    pageDeps.page = vi.fn(() => Promise.resolve(page({ scope: { kind: "topic", id: 2 }, scopeTopicName: "FUEL" }))) as never;
    pageDeps.preview = vi.fn(() => Promise.resolve(local));
    const c = createPageController();
    await c.open(5);
    await c.ask("what is blocked?", local);
    expect(asked[0]).toMatchObject({ kind: "topic" });
    expect(asked[0].args.slice(0, 4)).toEqual([2, "what is blocked?", 5, true]);
    finish(asked[0]);
    await settle();
    expect(kept[0].exchange.requireLocal).toBe(true);
    // Hosted route: not held.
    asked.length = 0;
    pageDeps.preview = vi.fn(() => Promise.resolve(hostedPreview));
    await c.ask("and then?", hostedPreview);
    expect(asked[0].args[3]).toBe(false);
  });

  it("does not send a topic question when the button promised this PC and the route no longer says so", async () => {
    pageDeps.page = vi.fn(() => Promise.resolve(page({ scope: { kind: "topic", id: 2 } }))) as never;
    pageDeps.preview = vi.fn(() => Promise.resolve(hostedPreview));
    const c = createPageController();
    await c.open(5);
    await c.ask("q", local);
    expect(asked).toHaveLength(0);
    expect(c.state().note).toMatch(/changed since the button was drawn/);
  });

  it("a new topic conversation started from the header keeps the header's promise", async () => {
    pageDeps.preview = vi.fn(() => Promise.resolve(local));
    const c = createPageController();
    c.startDraft({ kind: "topic", id: 2 }, "FUEL", "q", local);
    await settle();
    expect(asked[0].args.slice(0, 4)).toEqual([2, "q", null, true]);
  });

  it("never holds a meeting question (it names no place) and asks no route", async () => {
    pageDeps.page = vi.fn(() => Promise.resolve(page({ scope: { kind: "meeting", id: 3 } }))) as never;
    const c = createPageController();
    await c.open(5);
    await c.ask("q", null);
    expect(asked[0]).toMatchObject({ kind: "meeting" });
    expect(pageDeps.preview).not.toHaveBeenCalled();
    finish(asked[0]);
    await settle();
    expect(kept[0].exchange.requireLocal).toBe(false);
  });
});

describe("a new conversation", () => {
  it("asks its first question without saving, keeps the answer, and the stored conversation takes its place", async () => {
    const c = createPageController();
    c.startDraft({ kind: "topic", id: 2 }, "FUEL", "what is blocked?");
    await settle();
    expect(c.state().id).toBeNull();
    expect(c.state().scopeLine).toBe("Asking about FUEL");
    expect(asked[0]).toMatchObject({ kind: "topic" });
    expect(asked[0].args.slice(0, 3)).toEqual([2, "what is blocked?", null]);
    finish(asked[0]);
    await settle();
    expect(kept[0]).toMatchObject({ id: null, scope: { kind: "topic", id: 2 } });
    expect(navSnapshot().view).toEqual({ kind: "page", id: 31 });
  });

  it("an everything draft asks with save off, so only the page's keep makes the row", async () => {
    const c = createPageController();
    c.startDraft({ kind: "everything" }, null, null);
    await c.ask("hello");
    expect(asked[0].args[5]).toBe(false);
  });

  it("keeps nothing when the first answer could not be kept, and says so", async () => {
    pageDeps.save = (() => Promise.reject("The memory database could not be read.")) as never;
    const c = createPageController();
    c.startDraft({ kind: "everything" }, null, "q");
    await settle();
    finish(asked[0]);
    await settle();
    expect(c.state().failed).toBe(true);
    expect(c.state().turns[0].error).toBe("Couldn't save the conversation. Try again.");
    expect(navSnapshot().view).toEqual({ kind: "list" });
  });
});

describe("the header", () => {
  it("renames, stars and puts the old title back when the shell refuses", async () => {
    const c = createPageController();
    await c.open(5);
    await c.rename("  Weekly sync  ");
    expect(pageDeps.rename).toHaveBeenCalledWith(5, "Weekly sync");
    expect(c.state().title).toBe("Weekly sync");
    pageDeps.rename = vi.fn(() => Promise.reject("no")) as never;
    await c.rename("Other");
    expect(c.state().title).toBe("Weekly sync");
    await c.toggleStar();
    expect(c.state().starred).toBe(true);
  });

  it("sets the person's topic as final, and removes it by choosing none", async () => {
    const c = createPageController();
    await c.open(5);
    await c.setTopic(4);
    expect(pageDeps.setTopic).toHaveBeenCalledWith(5, 4);
    expect(c.state()).toMatchObject({ topic: { topicId: 4 }, topicSetByUser: true });
    await c.setTopic(null);
    expect(c.state()).toMatchObject({ topic: null, topicSetByUser: true });
  });

  it("refuses to delete while an answer is in flight, and leaves for the list after deleting", async () => {
    const c = createPageController();
    await c.open(5);
    await c.ask("busy");
    expect(await c.remove()).toBe(false);
    expect(pageDeps.remove).not.toHaveBeenCalled();
    asked[0].send({ kind: "cancelled" });
    expect(await c.remove()).toBe(true);
    expect(pageDeps.remove).toHaveBeenCalledWith(5);
    expect(navSnapshot().view).toEqual({ kind: "list" });
  });

  it("stops a question in flight when the page goes, and ignores what it says after", async () => {
    const c = createPageController();
    await c.open(5);
    await c.ask("leaving");
    asked[0].send({ kind: "started", runId: "r7", provider: "ollama", model: "m", context: [] });
    c.dispose();
    expect(pageDeps.stop).toHaveBeenCalledWith("r7");
    asked[0].send({ kind: "delta", text: "late" });
    expect(c.state().turns.at(-1)?.text).toBe("");
  });
});

describe("the excluded-client line", () => {
  it("shows on the exchange kept for an excluded client and on every later one", () => {
    expect(carriedLocal([false, true, false, false])).toEqual([false, true, true, true]);
    const p = page({ localOnly: [false, true, false] }, [exchange("a", "A"), exchange("b", "B"), exchange("c", "C")]);
    const turns = turnsOfPage(p);
    expect(turns.map((x) => x.excludedClient)).toEqual([false, true, true]);
    expect(turns.map((x) => x.answeredLocally)).toEqual([false, true, true]);
  });
});
