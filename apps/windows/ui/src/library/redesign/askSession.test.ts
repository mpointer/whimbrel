import { describe, expect, it } from "vitest";
import type { ChatEvent, NumberedSourceView, SourcedConversationView } from "../../generated/types";
import { applySourcedEvent, continuesConversation, newSourcedTurn, placeAsk, rangeOf, turnsOf } from "./askSession";
import { filterKey } from "./findModel";

const src = (n: number): NumberedSourceView => ({ n, card: { kind: "capture", id: n, found: true, title: `T${n}` } });
const feed = (events: ChatEvent[]) => events.reduce(applySourcedEvent, newSourcedTurn("What did Sam say?"));

describe("a sourced answer", () => {
  it("collects the numbered list, the cited numbers and where it was made, then completes", () => {
    const turn = feed([
      { kind: "started", runId: "r1", provider: "ollama", model: "gemma4", context: [] },
      { kind: "sourced", sources: [src(1), src(2)], provider: "ollama", answeredLocally: true },
      { kind: "delta", text: "Sam wants a tier [1]." },
      { kind: "cited", cited: [1], answeredLocally: true, provider: "ollama", leftOut: 0 },
      { kind: "completed", answer: "Sam wants a tier [1].", citations: [{ kind: "capture", id: 1 }], conversationId: 4, narrowed: false, leftOut: null },
    ]);
    expect(turn.status).toBe("done");
    expect(turn.sources.map((s) => s.n)).toEqual([1, 2]);
    expect(turn.cited).toEqual([1]);
    expect(turn.answeredLocally).toBe(true);
    expect(turn.text).toBe("Sam wants a tier [1].");
  });
  it("replaces the list when the stream falls back to the blocking path", () => {
    const turn = feed([
      { kind: "started", runId: "r1", provider: "ollama", model: "m", context: [] },
      { kind: "sourced", sources: [src(1)], provider: "ollama", answeredLocally: true },
      { kind: "fell_back" },
      { kind: "sourced", sources: [src(1), src(2), src(3)], provider: "anthropic", answeredLocally: false },
    ]);
    expect(turn.sources).toHaveLength(3);
    expect(turn.status).toBe("blocking");
    expect(turn.answeredLocally).toBe(false);
  });
  it("says a question that names an excluded client was answered on this PC", () => {
    const turn = feed([
      { kind: "started", runId: null, provider: "anthropic", model: "m", context: [] },
      { kind: "answered_by", model: "gemma4", excludedClient: true },
    ]);
    expect(turn.excludedClient).toBe(true);
  });
  it("keeps a failure's error and keeps the text only if some had streamed", () => {
    const turn = feed([
      { kind: "started", runId: "r", provider: "p", model: "m", context: [] },
      { kind: "delta", text: "Half" },
      { kind: "failed", error: { kind: "provider_failure", reason: null }, partial: true },
    ]);
    expect(turn.status).toBe("failed");
    expect(turn.text).toBe("Half");
  });
});

describe("a saved conversation", () => {
  const conversation: SourcedConversationView = {
    id: 1,
    title: "t",
    starred: false,
    exchanges: [
      { query: "old", answer: "Old answer.", createdAtUnixMs: 1, legacy: true, cited: [], model: "m", citations: [{ item: { kind: "capture", id: 3 }, label: "Excel", tsUnixMs: 1, open: { kind: "capture", id: 3 } }], sources: [] },
      { query: "new", answer: "New [1].", createdAtUnixMs: 2, legacy: false, cited: [1], model: "m", provider: "ollama", answeredLocally: true, leftOut: 2, citations: [], sources: [{ n: 1, card: { kind: "capture", id: 1, found: false, title: "" } }] },
    ],
  };
  it("draws an answer saved before numbered sources with today's chips, and a newer one from its cited sources", () => {
    const [old, next] = turnsOf(conversation);
    expect(old.legacy).toBe(true);
    expect(old.legacyCitations).toHaveLength(1);
    expect(old.sources).toEqual([]);
    expect(next.legacy).toBe(false);
    expect(next.cited).toEqual([1]);
    expect(next.sources[0].card.found).toBe(false);
    expect(next.answeredLocally).toBe(true);
    expect(next.leftOut).toBe(2);
  });
});

describe("questions and filters", () => {
  const ask = { query: "q", filter: {}, requireLocal: false, from: "header" as const };
  it("starts a question at once, or lets it wait its turn while another answers", () => {
    expect(placeAsk(false, null, ask)).toEqual({ start: ask, queued: null, refused: false });
    expect(placeAsk(true, null, ask)).toEqual({ start: null, queued: ask, refused: false });
  });
  it("lets only one question wait: a second is refused, and the first is neither replaced nor dropped", () => {
    const second = { ...ask, query: "second" };
    expect(placeAsk(true, ask, second)).toEqual({ start: null, queued: ask, refused: true });
  });
  it("reads the range an answer drew on only when it is whole", () => {
    expect(rangeOf({ kind: "sourced" })).toBeNull();
    expect(rangeOf({ dateRange: null })).toBeNull();
    expect(rangeOf({ dateRange: { startUnixMs: 1 } })).toBeNull();
    expect(rangeOf({ dateRange: { startUnixMs: 1, endUnixMs: 2 } })).toEqual({ startUnixMs: 1, endUnixMs: 2 });
  });
  it("keeps the filters a question was asked under with its turn, and the range when the core says one", () => {
    const f = { startUnixMs: 1, endUnixMs: 2, app: null, person: null, personKey: null };
    const turn = newSourcedTurn("q", f);
    expect(turn.filter).toBe(f);
    const sourced = applySourcedEvent(turn, { kind: "sourced", sources: [], provider: "p", answeredLocally: false, dateRange: { startUnixMs: 5, endUnixMs: 9 } } as never);
    expect(sourced.drewRange).toEqual({ startUnixMs: 5, endUnixMs: 9 });
    expect(applySourcedEvent(turn, { kind: "sourced", sources: [], provider: "p", answeredLocally: false } as never).drewRange).toBeNull();
  });
  it("never feeds an answer made under one set of filters into a question under another", () => {
    const a = { startUnixMs: 1, endUnixMs: 2 };
    expect(continuesConversation(filterKey(a), a)).toBe(true);
    expect(continuesConversation(filterKey(a), { ...a, app: "excel.exe" })).toBe(false);
    // A conversation reopened from the list: its filters were not saved.
    expect(continuesConversation(null, a)).toBe(false);
  });
});
