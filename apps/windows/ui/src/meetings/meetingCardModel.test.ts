import { describe, expect, it } from "vitest";
import type { MeetingRow, NextMeetingView, ScheduleEvent, TopicRefView } from "../generated/types";
import { timeLabel } from "../library/rows";
import {
  chipLookupKey,
  chipTarget,
  dayHeading,
  forEvent,
  forRecorded,
  groupByDay,
  latestOnly,
  nowLabel,
  nowMarkerIndex,
  peopleText,
  subtitle,
  timeRange,
  timeline,
  type HeldChips,
} from "./meetingCardModel";

const MIN = 60_000;
const START = new Date(2026, 10, 11, 14, 0).getTime();
const NOW = START - 60 * MIN;
const L = "en-US";
const topic: TopicRefView = { topicId: 7, key: "acme", name: "Acme" };

const event = (over: Partial<ScheduleEvent> = {}): ScheduleEvent => ({
  eventId: "EVT-1",
  title: "Planning",
  startUnixMs: START,
  endUnixMs: START + 45 * MIN,
  attendees: ["Ana", "Ben"],
  recordedMeetingId: null,
  prepKey: null,
  hasSummary: false,
  actionItemCount: 0,
  ...over,
});

const meeting = (over: Partial<MeetingRow> = {}): MeetingRow => ({
  id: 42,
  tsUnixMs: START,
  endUnixMs: START + 30 * MIN,
  title: "Standup",
  status: "completed",
  trigger: "manual",
  hasSummary: false,
  attendeeCount: 1,
  actionItemCount: 0,
  device: { deviceId: "d", label: "This PC", thisDevice: true },
  ...over,
});

describe("an upcoming event's card", () => {
  it("opens its brief when one exists, with Prep then Topic chips", () => {
    const m = forEvent(event({ prepKey: "EVT-1@1" }), topic, NOW, L);
    expect(m.target).toEqual({ kind: "brief", key: "EVT-1@1" });
    expect(m.chips).toEqual([
      { kind: "prep", key: "EVT-1@1" },
      { kind: "topic", id: 7, name: "Acme" },
    ]);
    expect(m.isPast).toBe(false);
  });

  it("opens its topic page when there is no brief", () => {
    const m = forEvent(event(), topic, NOW, L);
    expect(m.target).toEqual({ kind: "topic", id: 7 });
    expect(m.chips).toEqual([{ kind: "topic", id: 7, name: "Acme" }]);
  });

  it("is not clickable with neither", () => {
    const m = forEvent(event(), null, NOW, L);
    expect(m.target).toEqual({ kind: "none" });
    expect(m.chips).toEqual([]);
  });

  it("opens the meeting once it is recorded, with its summary chips", () => {
    const m = forEvent(event({ recordedMeetingId: 9, prepKey: "k", hasSummary: true, actionItemCount: 2 }), topic, NOW, L);
    expect(m.target).toEqual({ kind: "meeting", id: 9 });
    expect(m.chips.map((c) => c.kind)).toEqual(["prep", "summary", "actionItems", "topic"]);
  });

  it("counts you among the people", () => {
    expect(forEvent(event(), null, NOW, L).peopleCount).toBe(3);
    expect(forEvent(event({ attendees: [] }), null, NOW, L).peopleCount).toBe(1);
    expect(peopleText(1)).toBe("1 person");
    expect(peopleText(3)).toBe("3 people");
  });

  it("is running between start and end, and past from its end", () => {
    const during = forEvent(event(), null, START + 10 * MIN, L);
    expect([during.isRunning, during.isPast]).toEqual([true, false]);
    const before = forEvent(event(), null, NOW, L);
    expect([before.isRunning, before.isPast]).toEqual([false, false]);
    const after = forEvent(event(), null, START + 45 * MIN, L);
    expect([after.isRunning, after.isPast]).toEqual([false, true]);
  });

  it("writes the time range and duration", () => {
    const m = forEvent(event(), null, NOW, L);
    expect(m.timeRange).toBe(`${timeLabel(START, L)} to ${timeLabel(START + 45 * MIN, L)}`);
    expect(m.durationMinutes).toBe(45);
    expect(subtitle(m)).toBe(`${m.timeRange} · 3 people`);
  });
});

describe("a recorded meeting's card", () => {
  it("always opens the meeting, chips in the order Prep, Summary, action items, Topic", () => {
    const m = forRecorded(meeting({ hasSummary: true, actionItemCount: 1 }), "k@1", topic, L);
    expect(m.target).toEqual({ kind: "meeting", id: 42 });
    expect(m.chips).toEqual([{ kind: "prep", key: "k@1" }, { kind: "summary" }, { kind: "actionItems", count: 1 }, { kind: "topic", id: 7, name: "Acme" }]);
    expect(m.durationMinutes).toBe(30);
  });

  it("shows no chips without a summary, action items, brief or topic", () => {
    const m = forRecorded(meeting(), null, null, L);
    expect(m.chips).toEqual([]);
    expect(m.target).toEqual({ kind: "meeting", id: 42 });
  });

  it("shows no people count with no attendees", () => {
    expect(forRecorded(meeting({ attendeeCount: 0 }), null, null, L).peopleCount).toBeNull();
    expect(forRecorded(meeting({ attendeeCount: 2 }), null, null, L).peopleCount).toBe(3);
  });

  it("shows only the start when the end is missing, not after the start, or equal to it", () => {
    const startOnly = timeLabel(START, L);
    expect(forRecorded(meeting({ endUnixMs: null }), null, null, L).timeRange).toBe(startOnly);
    expect(forRecorded(meeting({ endUnixMs: null }), null, null, L).durationMinutes).toBeNull();
    const backwards = forRecorded(meeting({ endUnixMs: START - 15 * MIN }), null, null, L);
    expect([backwards.timeRange, backwards.durationMinutes]).toEqual([startOnly, null]);
    expect(timeRange(START, START, L)).toBe(startOnly);
  });

  it("keeps Windows' status line: recording now, or no summary yet", () => {
    expect(forRecorded(meeting({ status: "recording" }), null, null, L).statusLine).toBe("Recording");
    expect(forRecorded(meeting(), null, null, L).statusLine).toBe("No summary yet");
    expect(forRecorded(meeting({ hasSummary: true }), null, null, L).statusLine).toBeNull();
  });

  it("is not running", () => {
    expect(forRecorded(meeting({ status: "recording", endUnixMs: null }), null, null, L).isRunning).toBe(false);
  });
});

describe("chips open their own item", () => {
  it("Prep opens the brief, Topic the page, Summary and action items the meeting", () => {
    const card = { kind: "meeting", id: 42 } as const;
    expect(chipTarget({ kind: "prep", key: "k" }, card)).toEqual({ kind: "brief", key: "k" });
    expect(chipTarget({ kind: "topic", id: 7, name: "Acme" }, card)).toEqual({ kind: "topic", id: 7 });
    expect(chipTarget({ kind: "summary" }, card)).toEqual(card);
    expect(chipTarget({ kind: "actionItems", count: 2 }, card)).toEqual(card);
    expect(chipTarget({ kind: "summary" }, { kind: "brief", key: "k" })).toEqual({ kind: "none" });
  });
});

describe("Today's timeline", () => {
  const none = { eventTopics: [], held: new Map<number, HeldChips>() };

  it("puts held meetings and events in start order, with the held meeting's chips", () => {
    const held = meeting({ id: 5, tsUnixMs: START - 120 * MIN, endUnixMs: START - 90 * MIN });
    const chips = { eventTopics: [], held: new Map([[5, { prepKey: "p@1", topic }]]) };
    const entries = timeline([event()], [held], null, chips, NOW, L);
    expect(entries.map((e) => e.key)).toEqual(["held-5", `event-EVT-1@${START}`]);
    expect(entries[0].model.chips.map((c) => c.kind)).toEqual(["prep", "topic"]);
    expect(entries[0].model.isPast).toBe(true);
  });

  it("gives the hint only to the next upcoming event, even while another is running", () => {
    const running = event({ eventId: "run", startUnixMs: NOW - 10 * MIN, endUnixMs: NOW + 20 * MIN });
    const soon = event({ eventId: "soon", startUnixMs: NOW + 30 * MIN, endUnixMs: NOW + 60 * MIN });
    const later = event({ eventId: "later", startUnixMs: NOW + 90 * MIN, endUnixMs: NOW + 120 * MIN });
    const entries = timeline([running, soon, later], [], null, none, NOW, L);
    expect(entries.map((e) => e.model.hint)).toEqual([null, "In 30m", null]);
    expect(entries[0].model.isRunning).toBe(true);
  });

  it("marks the next meeting's card for its brief toggle", () => {
    const next: NextMeetingView = { eventId: "EVT-1", title: "Planning", startUnixMs: START, brief: null };
    expect(timeline([event()], [], next, none, NOW, L)[0].isNext).toBe(true);
    expect(timeline([event({ recordedMeetingId: 3 })], [], next, none, NOW, L)[0].isNext).toBe(false);
  });

  it("counts a held meeting with no end as ended at its start", () => {
    const entries = timeline([], [meeting({ endUnixMs: null })], null, none, NOW, L);
    expect(entries[0].endMs).toBe(START);
  });
});

describe("the Now marker", () => {
  const ago = (m: number) => NOW - m * MIN;
  it("sits before the first meeting that hasn't ended", () => {
    expect(nowMarkerIndex([], NOW)).toBe(0);
    expect(nowMarkerIndex([ago(60), ago(30)], NOW)).toBe(2);
    expect(nowMarkerIndex([ago(-60), ago(-120)], NOW)).toBe(0);
    expect(nowMarkerIndex([NOW, ago(-60)], NOW)).toBe(1);
    expect(nowMarkerIndex([ago(60), ago(-60), ago(30)], NOW)).toBe(1);
  });

  it("says the time", () => {
    expect(nowLabel(NOW, L)).toBe(`Now · ${timeLabel(NOW, L)}`);
  });
});

describe("day headings", () => {
  const now = new Date(2026, 9, 6, 12).getTime(); // Tuesday, Oct 6
  const at = (day: number) => new Date(2026, 9, day, 9).getTime();

  it("are Today, Yesterday, the weekday within the week, then the date", () => {
    expect(dayHeading(at(6), now, L)).toBe("Today");
    expect(dayHeading(at(5), now, L)).toBe("Yesterday");
    expect(dayHeading(at(2), now, L)).toBe("Friday");
    expect(dayHeading(at(1), now, L)).toBe("Thursday");
    expect(dayHeading(new Date(2026, 8, 29, 9).getTime(), now, L)).toBe("Tue, Sep 29"); // a week ago: the date
    expect(dayHeading(new Date(2026, 8, 20, 9).getTime(), now, L)).toBe("Sun, Sep 20");
  });

  it("are written in the language", () => {
    expect(dayHeading(at(6), now, "de")).toBe("Heute");
    expect(dayHeading(at(5), now, "de")).toBe("Gestern");
  });

  it("group rows by local day, keeping their order", () => {
    const rows = [{ id: 1, tsUnixMs: at(6) }, { id: 2, tsUnixMs: at(6) - 60 * MIN }, { id: 3, tsUnixMs: at(5) }];
    expect(groupByDay(rows, now, L).map((g) => [g.title, g.rows.map((r) => r.id)])).toEqual([
      ["Today", [1, 2]],
      ["Yesterday", [3]],
    ]);
  });
});

describe("the chips lookup", () => {
  it("reruns only when the set of meetings, or which have a summary, changes", () => {
    const a = chipLookupKey([{ id: 1, hasSummary: false }, { id: 2, hasSummary: true }]);
    expect(chipLookupKey([{ id: 2, hasSummary: true }, { id: 1, hasSummary: false }])).toBe(a);
    expect(chipLookupKey([{ id: 1, hasSummary: true }, { id: 2, hasSummary: true }])).not.toBe(a);
    expect(chipLookupKey([{ id: 1, hasSummary: false }])).not.toBe(a);
  });

  it("drops a stale answer that arrives after a newer lookup began", () => {
    const latest = latestOnly();
    const first = latest.begin();
    const second = latest.begin();
    expect(latest.isLatest(second)).toBe(true);
    expect(latest.isLatest(first)).toBe(false);
  });
});
