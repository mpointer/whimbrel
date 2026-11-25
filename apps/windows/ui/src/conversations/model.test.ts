import { describe, expect, it } from "vitest";
import type { ConversationListRow, TopicRefView } from "../generated/types";
import { newSourcedTurn } from "../library/redesign/askSession";
import { askTarget, composerLabel, filterRows, madeHere, markerNumbers, meetingHistory, meetingLabel, meetingPointers, menuTopics, pointersOf, savedInput, scopeLine, sortRows, updatedLabel } from "./model";

const card = (kind: string, id: number) => ({ kind, id, found: true, title: "", tsUnixMs: null }) as never;
const row = (id: number, starred: boolean, updatedAtUnixMs: number, title = `c${id}`): ConversationListRow => ({ id, title, starred, updatedAtUnixMs, topic: null, latestAnswerLine: null });
const topic = (topicId: number, name = `t${topicId}`): TopicRefView => ({ topicId, key: `k${topicId}`, name });

describe("the composer's button names a place only for an everything question", () => {
  const local = { destination: "local", local: true, provider: "ollama", model: "g", reason: "x", requireLocal: true, mayFallBack: false } as never;
  it("says Ask, on this PC for everything, and Send for a topic or a meeting", () => {
    expect(composerLabel({ kind: "everything" }, local)).toBe("Ask, on this PC");
    expect(composerLabel({ kind: "everything" }, null)).toBe("Send");
    expect(composerLabel({ kind: "topic", id: 1 }, local)).toBe("Send");
    expect(composerLabel({ kind: "meeting", id: 1 }, local)).toBe("Send");
  });
});

describe("where a question from the header goes (the Mac's WhimbrelTab.askTarget)", () => {
  it("answers on Today, asks everything from Conversations, asks the topic shown on a topic page, and leaves the rest to Find", () => {
    expect(askTarget("today", null)).toEqual({ kind: "today" });
    expect(askTarget("today", 7)).toEqual({ kind: "today" });
    expect(askTarget("conversations", null)).toEqual({ kind: "conversation", scope: { kind: "everything" } });
    expect(askTarget("topics", 7)).toEqual({ kind: "conversation", scope: { kind: "topic", id: 7 } });
    expect(askTarget("topics", null)).toEqual({ kind: "library" });
    for (const tab of ["meetings", "library", "automations", "settings"] as const) expect(askTarget(tab, 7)).toEqual({ kind: "library" });
  });

  it("sends a question asked under a visible filter to Find, which obeys the chips, wherever it would have gone", () => {
    expect(askTarget("today", null, true)).toEqual({ kind: "library" });
    expect(askTarget("conversations", null, true)).toEqual({ kind: "library" });
    expect(askTarget("topics", 7, true)).toEqual({ kind: "library" });
    expect(askTarget("today", null, false)).toEqual({ kind: "today" });
  });
});

describe("what a held answer keeps", () => {
  it("numbers citations as the answer's own marks, first written first, and falls back to 1, 2, 3 when the text does not account for them", () => {
    expect(markerNumbers("So [2] and then [1] and [2] again", 2)).toEqual([2, 1]);
    // A marker past the context was dropped by core: the largest distinct numbers go.
    expect(markerNumbers("see [1] and [9]", 1)).toEqual([1]);
    expect(markerNumbers("no marks here", 3)).toEqual([1, 2, 3]);
    expect(markerNumbers("[0] is not a mark", 0)).toEqual([]);
  });

  it("keeps pointers (number, kind, id) for the cited sources only, once each", () => {
    const turn = { ...newSourcedTurn("q"), sources: [1, 2, 3].map((n) => ({ n, card: card(n === 2 ? "transcript" : "capture", n * 10) })), cited: [3, 2, 3, 9] };
    expect(pointersOf(turn)).toEqual([
      { n: 3, kind: "capture", id: 30 },
      { n: 2, kind: "transcript", id: 20 },
    ]);
    expect(pointersOf({ ...turn, cited: null })).toEqual([]);
  });

  it("points a meeting answer's citations at the numbers of its marks", () => {
    const citations = [
      { kind: "transcript", id: 41 },
      { kind: "transcript", id: 77 },
    ] as const;
    expect(meetingPointers("It was agreed [2] after [1].", [...citations])).toEqual([
      { n: 2, kind: "transcript", id: 41 },
      { n: 1, kind: "transcript", id: 77 },
    ]);
  });

  it("says whether a model runs on this PC", () => {
    expect(madeHere("local", "x")).toBe(true);
    expect(madeHere("ollama", "gemma3:4b")).toBe(true);
    expect(madeHere("ollama", "gpt-oss:120b-cloud")).toBe(false);
    // Any `cloud` word is Ollama's cloud, as in core (not only a trailing one), whatever the case.
    for (const model of ["glm-4.6:cloud", "kimi-k2:cloud-1t", "Qwen3:Cloud", "x_cloud_y", "cloud"]) expect(madeHere("ollama", model)).toBe(false);
    for (const model of ["gemma3:4b", "cloudy-name:7b", "mycloud:1b", ""]) expect(madeHere("ollama", model)).toBe(true);
    expect(madeHere(" ollama ", "gemma3:4b")).toBe(true);
    expect(madeHere("anthropic", "claude")).toBe(false);
    expect(madeHere(null, null)).toBe(false);
  });

  it("builds what the shell is asked to keep from the held answer, with the left-out count and where it was made", () => {
    const turn = { ...newSourcedTurn("What changed?"), text: "Two things [1].", model: "m", provider: "anthropic", answeredLocally: false, leftOut: 2 };
    expect(savedInput(turn, true, [{ n: 1, kind: "capture", id: 5 }])).toEqual({
      query: "What changed?",
      answer: "Two things [1].",
      cited: [{ n: 1, kind: "capture", id: 5 }],
      model: "m",
      provider: "anthropic",
      answeredLocally: false,
      leftOut: 2,
      requireLocal: true,
      routedLocally: false,
    });
    expect(savedInput({ ...turn, provider: "ollama", model: "gemma3:4b", leftOut: null }, false, [], "shown").answeredLocally).toBe(true);
    expect(savedInput({ ...turn, provider: "ollama", model: "gemma3:4b", leftOut: null }, false, [], "shown").query).toBe("shown");
  });

  it("sends only what makes the exchange more private: a reroute, a missing provider, a held-local question", () => {
    const turn = { ...newSourcedTurn("What changed?"), text: "Two things [1].", model: "m", provider: "anthropic", answeredLocally: false };
    const sent = (over: object, requireLocal = false) => savedInput({ ...turn, ...over } as never, requireLocal, []);
    // The shell decides locality from the stored rows; the page never says "not local" about anything it did not hear.
    expect(sent({ routedLocally: true }).routedLocally).toBe(true);
    expect(sent({ answeredLocally: true }).answeredLocally).toBe(true);
    expect(sent({}, true).requireLocal).toBe(true);
    // An answer with no provider or model is sent with empty strings, which the shell reads as unknown and keeps local.
    const unknown = sent({ provider: null, model: undefined });
    expect([unknown.provider, unknown.model]).toEqual(["", ""]);
    // Hosted, not rerouted, not held: still sent as exactly that, so the shell's own check is what decides.
    expect(sent({})).toMatchObject({ answeredLocally: false, requireLocal: false, routedLocally: false });
  });

  it("sends only finished answers as the meeting Ask's history", () => {
    const turns = [
      { query: "a", text: "A", status: "done" },
      { query: "b", text: "", status: "failed" },
      { query: "c", text: "C", status: "done" },
    ] as const;
    expect(meetingHistory([...turns])).toEqual([
      { query: "a", answer: "A" },
      { query: "c", answer: "C" },
    ]);
  });
});

describe("the scope line and the topic chip", () => {
  it("says what a conversation asks about, and nothing for a topic or meeting with no name", () => {
    expect(scopeLine({ kind: "everything" }, {})).toBe("Asking about everything");
    expect(scopeLine({ kind: "topic", id: 1 }, { topic: "FUEL" })).toBe("Asking about FUEL");
    expect(scopeLine({ kind: "meeting", id: 1 }, { meeting: "Pod 2 Team Call, Nov 19" })).toBe("Asking about Pod 2 Team Call, Nov 19");
    expect(scopeLine({ kind: "topic", id: 1 }, { topic: "  " })).toBeNull();
    expect(scopeLine({ kind: "meeting", id: 1 }, {})).toBeNull();
  });

  it("names a meeting by its title and day", () => {
    expect(meetingLabel("Pod 2 Team Call", Date.UTC(2026, 10, 19, 12), "en-US")).toBe("Pod 2 Team Call, Nov 19");
  });

  it("offers the topics the sources belong to, once each and without the current one", () => {
    expect(menuTopics([topic(1), topic(2), topic(1), topic(3)], topic(2)).map((x) => x.topicId)).toEqual([1, 3]);
    expect(menuTopics([], null)).toEqual([]);
  });
});

describe("the list", () => {
  it("orders starred first whatever their age, then most recently updated, then the newer row", () => {
    const rows = [row(1, false, 300), row(2, true, 100), row(3, false, 300), row(4, true, 200)];
    expect(sortRows(rows).map((r) => r.id)).toEqual([4, 2, 3, 1]);
  });

  it("filters by title ignoring case and accents, and shows every row for a blank search", () => {
    const rows = [row(1, false, 1, "Café order"), row(2, false, 1, "Pod 2 notes")];
    expect(filterRows(rows, "CAFE").map((r) => r.id)).toEqual([1]);
    expect(filterRows(rows, "  ").length).toBe(2);
    expect(filterRows(rows, "zzz")).toEqual([]);
  });

  it("words the last update", () => {
    const now = Date.UTC(2026, 10, 18, 12);
    expect(updatedLabel(now - 2 * 3_600_000, now, "en-US")).toBe("2 hours ago");
    expect(updatedLabel(now - 86_400_000, now, "en-US")).toBe("yesterday");
    expect(updatedLabel(now - 5_000, now, "en-US")).toBe("now");
  });
});
