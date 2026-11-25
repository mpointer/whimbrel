import { describe, expect, it } from "vitest";
import type { ActionItemView, NoteSourcesView } from "../../generated/types";
import { buildNote, clockToCs, hasWhimbrelLines, holdMargin, lineSpeaker, marginShown, nearestLineId, noteState, offerSources, offsetClock, resolveTarget, sourceName } from "./noteModel";

const src = (segmentId: number, startCs: number, speaker = "Jo", text = "words") => ({ segmentId, startCs, speaker, text });
const item = (id: number, text: string, source = "generated"): ActionItemView => ({ id, text, source, completed: false });

describe("clock labels", () => {
  it("reads minutes and seconds, and hours past an hour", () => {
    expect(offsetClock(0)).toBe("0:00");
    expect(offsetClock(110_500)).toBe("18:25");
    expect(offsetClock(372_300)).toBe("1:02:03");
    expect(offsetClock(-5)).toBe("0:00");
  });
  it("reads an alert's asked-at time back", () => {
    expect(clockToCs("4:05")).toBe(24_500);
    expect(clockToCs("1:02:03")).toBe(372_300);
    expect(clockToCs("soon")).toBeNull();
    expect(clockToCs("4:75")).toBeNull();
  });
  it("reads minutes of 100 or more, as a long meeting's alert shows them", () => {
    expect(clockToCs("125:30")).toBe(753_000);
    expect(clockToCs("100:00")).toBe(600_000);
    expect(clockToCs("125:75")).toBeNull();
    expect(clockToCs("1:75:00")).toBeNull();
    expect(clockToCs("12345:00")).toBeNull();
  });
  it("names a mark by number, speaker and time", () => {
    expect(sourceName(3, "Jo", 110_500)).toBe("Source 3, Jo, 18:25");
  });
  it("shows a guess with a question mark and an unnamed speaker as its stored label", () => {
    expect(lineSpeaker({ label: "Priya Shah", speaker: "Speaker 3", guess: true })).toBe("Priya Shah?");
    expect(lineSpeaker({ label: "Priya Shah", speaker: "Speaker 3", guess: false })).toBe("Priya Shah");
    expect(lineSpeaker({ label: "Me", speaker: "Me", guess: false })).toBe("Me");
  });
});

describe("the note's lines", () => {
  const sources: NoteSourcesView = {
    decisions: [{ text: "Renew for two years", source: src(11, 37_200) }, { text: "Hold the price" }],
    openQuestions: [{ text: "Who owns the questionnaire?", source: src(12, 110_500) }],
    actionItems: [
      { itemId: 1, text: "Send the quote", owner: "Sam", due: "Thu, Nov 19", source: src(13, 173_000, "Sam") },
      { itemId: 2, text: "Answer the items", owner: "Dana" },
    ],
  };
  const base = { decisions: ["Renew for two years", "Hold the price"], decisionsAreEdited: false, openQuestions: ["Who owns the questionnaire?"], openQuestionsAreEdited: false, items: [item(1, "Send the quote"), item(2, "Answer the items")], sources };

  it("numbers the sources in reading order and leaves a line with none unmarked", () => {
    const m = buildNote(base);
    expect(m.decisions.map((l) => l.number)).toEqual([1, null]);
    expect(m.questions.map((l) => l.number)).toEqual([2]);
    expect(m.actions.map((a) => a.number)).toEqual([3, null]);
    expect(m.entries.map((e) => [e.number, e.source.segmentId])).toEqual([[1, 11], [2, 12], [3, 13]]);
  });
  it("gives an action item its owner and due, with or without a source", () => {
    const m = buildNote(base);
    expect(m.actions[0]).toMatchObject({ owner: "Sam", due: "Thu, Nov 19" });
    expect(m.actions[1]).toMatchObject({ owner: "Dana", due: null, number: null });
  });
  it("takes AI ink from a line Whimbrel wrote, not one the person wrote or edited", () => {
    const m = buildNote({ ...base, items: [item(1, "Send the quote"), item(2, "Mine", "user")], decisionsAreEdited: true, editedItems: new Set([1]) });
    expect(m.decisions.every((l) => !l.ai && l.number === null)).toBe(true);
    expect(m.questions[0].ai).toBe(true);
    expect(m.actions.map((a) => a.ai)).toEqual([false, false]);
    expect(m.actions[0]).toMatchObject({ owner: null, number: null });
  });
  it("treats a generated item whose text no longer matches a stored line as the person's, after a reload too", () => {
    // The person changed "Send the quote" and reloaded: the stored line is still the old words, so nothing attaches.
    const m = buildNote({ ...base, items: [item(1, "Send the quote today"), item(2, "Answer the items")] });
    expect(m.actions[0]).toMatchObject({ ai: false, owner: null, due: null, number: null });
    expect(m.actions[1].ai).toBe(true);
    // The same id with other words never takes the stored line's source.
    expect(m.entries.map((e) => e.source.segmentId)).toEqual([11, 12]);
  });
  it("keeps plain AI ink on a generated item when no action line was stored to compare with", () => {
    expect(buildNote({ ...base, sources: null }).actions.every((a) => a.ai)).toBe(true);
    const none = { decisions: [], openQuestions: [], actionItems: [] };
    expect(buildNote({ ...base, sources: none }).actions.every((a) => a.ai)).toBe(true);
  });
  it("never gives a person's own item AI ink", () => {
    expect(buildNote({ ...base, items: [item(9, "Mine", "user")] }).actions[0].ai).toBe(false);
  });
  it("says whether Whimbrel wrote any line on the page", () => {
    const empty = buildNote({ decisions: [], decisionsAreEdited: false, openQuestions: [], openQuestionsAreEdited: false, items: [], sources: null });
    expect(hasWhimbrelLines(empty, "", false)).toBe(false);
    expect(hasWhimbrelLines(empty, "A summary.", false)).toBe(true);
    expect(hasWhimbrelLines(empty, "A summary.", true)).toBe(false);
    expect(hasWhimbrelLines(buildNote(base), "", true)).toBe(true);
    const mine = buildNote({ ...base, decisionsAreEdited: true, openQuestionsAreEdited: true, items: [item(5, "Mine", "user")] });
    expect(hasWhimbrelLines(mine, "", false)).toBe(false);
  });
  it("shows plain AI lines when nothing was ever sourced", () => {
    const m = buildNote({ ...base, sources: null });
    expect(m.entries).toEqual([]);
    expect(m.decisions.every((l) => l.ai && l.number === null)).toBe(true);
  });
  it("pairs repeated lines one to one", () => {
    const m = buildNote({ ...base, decisions: ["A", "A"], sources: { decisions: [{ text: "A", source: src(1, 100) }, { text: "A" }], openQuestions: [], actionItems: [] }, items: [], openQuestions: [] });
    expect(m.decisions.map((l) => l.number)).toEqual([1, null]);
  });
  it("matches an item with no stored id by its text", () => {
    const m = buildNote({ ...base, sources: { decisions: [], openQuestions: [], actionItems: [{ itemId: null, text: "Send the quote", owner: "Sam" }] }, items: [item(1, "Send the quote")] });
    expect(m.actions[0].owner).toBe("Sam");
  });
});

describe("opening the transcript", () => {
  const lines = [{ id: 10, startCs: 0 }, { id: 11, startCs: 3000 }, { id: 12, startCs: 9000 }];
  it("finds the nearest line to a time, the earlier on a tie", () => {
    expect(nearestLineId(lines, 3100)).toBe(11);
    expect(nearestLineId(lines, 6000)).toBe(11);
    expect(nearestLineId(lines, 99_999)).toBe(12);
    expect(nearestLineId([], 5)).toBeNull();
  });
  it("resolves a line by id only when it is loaded, a time to the nearest line", () => {
    expect(resolveTarget(lines, { kind: "line", id: 12 })).toBe(12);
    expect(resolveTarget(lines, { kind: "line", id: -4 })).toBeNull();
    expect(resolveTarget(lines, { kind: "time", cs: 2900 })).toBe(11);
    expect(resolveTarget(lines, null)).toBeNull();
  });
});

describe("the page's state", () => {
  const view = { hasTranscript: true, decisions: [], actionItems: [] };
  it("is processing while a stage runs, whatever else is there", () => {
    expect(noteState({ ...view, processingStage: "Writing the summary", summary: "x" })).toBe("processing");
  });
  it("is failed when the summary failed and nothing was written", () => {
    expect(noteState({ ...view, summaryFailure: "timed out" })).toBe("failed");
    expect(noteState({ ...view, summaryFailure: "timed out", summary: "Older text" })).toBe("ready");
  });
  it("has no summary for a transcript alone", () => {
    expect(noteState(view)).toBe("noSummary");
    expect(noteState({ ...view, summary: "  " })).toBe("noSummary");
    expect(noteState({ ...view, decisions: ["a"] })).toBe("ready");
  });
  it("offers to add sources only where this PC can regenerate and no line has one", () => {
    const none = buildNote({ decisions: ["a"], decisionsAreEdited: false, openQuestions: [], openQuestionsAreEdited: false, items: [], sources: null });
    expect(offerSources(none, true, true)).toBe(true);
    expect(offerSources(none, false, true)).toBe(false);
    expect(offerSources(none, true, false)).toBe(false);
  });
});

describe("the right margin", () => {
  const none = buildNote({ decisions: [], decisionsAreEdited: false, openQuestions: [], openQuestionsAreEdited: false, items: [], sources: null });
  const sourced = buildNote({
    decisions: ["A"],
    decisionsAreEdited: false,
    openQuestions: [],
    openQuestionsAreEdited: false,
    items: [],
    sources: { decisions: [{ text: "A", source: src(1, 100) }], openQuestions: [], actionItems: [] },
  });
  it("folds away when there is nothing to put in it: no sources and no Whimbrel line (a meeting with no transcript and no summary)", () => {
    expect(marginShown(none, false)).toBe(false);
  });
  it("stays for a note with sources", () => {
    expect(marginShown(sourced, true)).toBe(true);
  });
  it("stays where it holds its line and the Regenerate offer: Whimbrel wrote lines and none has a source", () => {
    const plain = buildNote({ decisions: ["a"], decisionsAreEdited: false, openQuestions: [], openQuestionsAreEdited: false, items: [], sources: null });
    expect(offerSources(plain, true, true)).toBe(true);
    expect(marginShown(plain, true)).toBe(true);
  });
});


describe("the margin while editing", () => {
  it("follows the note when nobody is editing", () => {
    expect(holdMargin(null, false, true)).toEqual({ shown: true, held: null });
    expect(holdMargin(true, false, false)).toEqual({ shown: false, held: null });
  });
  it("takes the answer when Edit begins and keeps it, whatever typing does to the note", () => {
    const begin = holdMargin(null, true, true);
    expect(begin).toEqual({ shown: true, held: true });
    // The person types over every Whimbrel line: the live answer becomes false, the column stays.
    expect(holdMargin(begin.held, true, false)).toEqual({ shown: true, held: true });
    // And a folded margin stays folded when typing would have made it show.
    const folded = holdMargin(null, true, false);
    expect(holdMargin(folded.held, true, true)).toEqual({ shown: false, held: false });
  });
  it("is released when Edit ends", () => {
    expect(holdMargin(true, false, false).shown).toBe(false);
  });
});
