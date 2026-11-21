import { describe, expect, it } from "vitest";
import type { TranscriptLineView } from "../../generated/types";
import { buildLanes, CAP_CS, growRange, LAST_LINE_CS, neighborSpan, otherLaneSpan, percent, rangeAround, rangeHolding, ROW_CHUNK, spanEnd } from "./lanesModel";

const line = (id: number, startCs: number, speaker: string, endCs: number | null = null): TranscriptLineView => ({ id, startCs, endCs, speaker, label: speaker, guess: false, speakerId: null, personKey: null, text: "x" });

describe("a span's end", () => {
  it("is the stored end when it is after the start", () => {
    expect(spanEnd(100, 450, 900)).toBe(450);
  });
  it("is the next line's start otherwise, at most 30 seconds on", () => {
    expect(spanEnd(100, null, 900)).toBe(900);
    expect(spanEnd(100, 100, 900)).toBe(900);
    expect(spanEnd(100, null, 100_000)).toBe(100 + CAP_CS);
  });
  it("draws the last line 5 seconds and never ends before the start", () => {
    expect(spanEnd(100, null, null)).toBe(100 + LAST_LINE_CS);
    expect(spanEnd(500, null, 400)).toBe(500);
  });
});

describe("the lanes", () => {
  const lines = [line(1, 0, "Me"), line(2, 1000, "Jo"), line(3, 2500, "Me", 2800), line(4, 6000, "Jo")];
  const model = buildLanes(lines, [{ id: 9, meetingId: 1, atCs: 7000, label: "ask", createdUnixMs: 0 }], (m) => m.label ?? "");
  it("has one lane per speaker, in the order they first speak", () => {
    expect(model.lanes.map((l) => l.speaker)).toEqual(["Me", "Jo"]);
    expect(model.lanes[0].spans.map((s) => [s.id, s.startCs, s.endCs])).toEqual([[1, 0, 1000], [3, 2500, 2800]]);
    expect(model.lanes[1].spans.map((s) => s.endCs)).toEqual([2500, 6500]);
  });
  it("runs to the last span or mark and puts the marks on as ticks", () => {
    expect(model.endCs).toBe(7000);
    expect(model.ticks).toEqual([{ id: 9, atCs: 7000, label: "ask" }]);
  });
  it("sorts lines that arrive out of order", () => {
    const m = buildLanes([line(2, 1000, "Jo"), line(1, 0, "Me")], [], () => "");
    expect(m.lanes.map((l) => l.speaker)).toEqual(["Me", "Jo"]);
  });
  it("has a length of at least one for an empty meeting", () => {
    expect(buildLanes([], [], () => "").endCs).toBe(1);
    expect(percent(5, 0)).toBe(100);
    expect(percent(50, 200)).toBe(25);
  });
  it("moves along in time with left and right and between lanes with up and down", () => {
    expect(neighborSpan(model, null, 1)).toBe(1);
    expect(neighborSpan(model, null, -1)).toBe(4);
    expect(neighborSpan(model, 2, 1)).toBe(3);
    expect(neighborSpan(model, 4, 1)).toBe(4);
    expect(neighborSpan(model, 1, -1)).toBe(1);
    expect(otherLaneSpan(model, 1, 1)).toBe(2);
    expect(otherLaneSpan(model, 3, 1)).toBe(2);
    expect(otherLaneSpan(model, 3, -1)).toBe(3);
    expect(otherLaneSpan(model, null, 1)).toBe(1);
  });
  it("answers null for no spans", () => {
    expect(neighborSpan({ lanes: [], endCs: 1, ticks: [] }, null, 1)).toBeNull();
  });
});

describe("the rows drawn", () => {
  it("starts at the top, or around a line", () => {
    expect(rangeAround(null, 500)).toEqual({ from: 0, to: ROW_CHUNK });
    expect(rangeAround(null, 10)).toEqual({ from: 0, to: 10 });
    expect(rangeAround(300, 500)).toEqual({ from: 300 - ROW_CHUNK / 2, to: 300 + ROW_CHUNK });
    expect(rangeAround(499, 500).to).toBe(500);
  });
  it("only grows, up or down, and stops at the ends", () => {
    expect(growRange({ from: 100, to: 180 }, 500, "down")).toEqual({ from: 100, to: 260 });
    expect(growRange({ from: 100, to: 180 }, 500, "up")).toEqual({ from: 20, to: 180 });
    expect(growRange({ from: 10, to: 490 }, 500, "up").from).toBe(0);
    expect(growRange({ from: 10, to: 490 }, 500, "down").to).toBe(500);
  });
  it("keeps the window for a line already in it and moves it for one outside", () => {
    const r = { from: 100, to: 180 };
    expect(rangeHolding(r, 120, 500)).toBe(r);
    expect(rangeHolding(r, 400, 500)).toEqual(rangeAround(400, 500));
  });
});
