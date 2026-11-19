import { describe, expect, it } from "vitest";
import type { DayBlock } from "../../api";
import type { ScheduleEvent, ScheduleView } from "../../generated/types";
import { arrangeTimeline, earlierGroups, peopleFrom, appDot, appName, blockItems, buildTimeline, captureLine, dateRange, dayIsEmpty, heroOf, meetingIntervals, nextEvent, searchArgs, soonLabel } from "./homeModel";

const MIN = 60_000;
const at = (h: number, m = 0, day = 18) => new Date(2026, 10, day, h, m, 0, 0).getTime();
const NOW = at(10, 22);

const event = (id: string, title: string, start: number, mins = 30, extra: Partial<ScheduleEvent> = {}): ScheduleEvent => ({
  actionItemCount: 0,
  attendees: ["Sam Okafor"],
  endUnixMs: start + mins * MIN,
  eventId: id,
  hasSummary: false,
  startUnixMs: start,
  title,
  ...extra,
});

const device = { deviceId: "PC1", label: "Alex's laptop", thisDevice: true };
const held = (id: number, title: string, start: number) => ({ actionItemCount: 0, attendeeCount: 2, device, endUnixMs: start + 30 * MIN, hasSummary: true, id, status: "done", title, trigger: "calendar", tsUnixMs: start });

const schedule = (events: ScheduleEvent[], earlierToday: ReturnType<typeof held>[] = [], recording = false): ScheduleView => ({ earlierToday, events, recording });

const block = (start: number, end: number, apps: DayBlock["apps"] = [{ app_bundle_id: "excel.exe", items: ["Pricing v3.xlsx"], more_items: 0 }], more_apps = 0): DayBlock => ({ start_unix_ms: start, end_unix_ms: end, apps, more_apps });

describe("soonLabel", () => {
  it("counts minutes, then hours and minutes", () => {
    expect(soonLabel(NOW + 9 * MIN, NOW)).toBe("in 9 min");
    expect(soonLabel(NOW + 128 * MIN, NOW)).toBe("in 2 hr 8 min");
    expect(soonLabel(NOW + 120 * MIN, NOW)).toBe("in 2 hr");
    expect(soonLabel(NOW - MIN, NOW)).toBe("now");
  });
});

describe("heroOf", () => {
  const base = { now: NOW, relevantMeeting: null, calendarConnected: true };
  it("is the recording meeting while one records", () => {
    const s = schedule([event("e1", "Pricing review", at(10, 0), 60, { recordedMeetingId: 7 })], [], true);
    expect(heroOf({ ...base, schedule: s, relevantMeeting: 7 })).toMatchObject({ kind: "live", meetingId: 7, title: "Pricing review" });
  });
  it("is the first event still ahead with no recording", () => {
    const s = schedule([event("e0", "Done", at(9, 0)), event("e1", "Hiring debrief", at(15, 0)), event("e2", "Later", at(16, 0))]);
    expect(nextEvent(s, NOW)?.eventId).toBe("e1");
    expect(heroOf({ ...base, schedule: s })).toMatchObject({ kind: "next", event: { eventId: "e1" } });
  });
  it("says no calendar only when nothing is scheduled and none is connected", () => {
    expect(heroOf({ ...base, schedule: schedule([]), calendarConnected: false }).kind).toBe("noCalendar");
    expect(heroOf({ ...base, schedule: schedule([]), calendarConnected: true }).kind).toBe("noMeetings");
    expect(heroOf({ ...base, schedule: schedule([]), calendarConnected: null }).kind).toBe("noMeetings");
  });
  it("is none when the day had meetings and none is left", () => {
    expect(heroOf({ ...base, schedule: schedule([event("e0", "Done", at(9, 0))], [held(1, "Standup", at(8, 0))]) }).kind).toBe("none");
  });
});

describe("buildTimeline", () => {
  const s = schedule(
    [event("e0", "Roadmap check-in", at(9, 0)), event("e1", "Pricing review", at(10, 30)), event("e2", "Design sync", at(13, 30), 45, { attendees: ["Dana Whitfield", "Priya Shah", "Sam Okafor", "Alex"] })],
    [held(1, "Standup", at(8, 0))],
  );
  const blocks = [block(at(8, 40), at(8, 55)), block(at(9, 40), at(10, 5))];
  const stops = buildTimeline({ now: NOW, locale: "en-US", schedule: s, heroEventId: "e1", heroMeetingId: null, blocks });

  it("runs oldest first with the now marker where the time falls", () => {
    expect(stops.map((x) => x.key.split("-")[0] + ":" + x.at)).toEqual([
      `meeting:${at(8, 0)}`,
      `block:${at(8, 40)}`,
      `event:${at(9, 0)}`,
      `block:${at(9, 40)}`,
      `now:${NOW}`,
      `event:${at(13, 30)}`,
    ]);
  });
  it("does not repeat the meeting the hero shows", () => {
    expect(stops.some((x) => x.kind === "meeting" && x.eventId === "e1")).toBe(false);
  });
  it("marks past, upcoming and what each can do", () => {
    const m = (id: string) => stops.find((x) => x.kind === "meeting" && x.eventId === id) as Extract<(typeof stops)[number], { kind: "meeting" }>;
    expect(m("e0")).toMatchObject({ node: "past", action: null, soon: null });
    expect(m("e2")).toMatchObject({ node: "upcoming", action: "start", soon: "in 3 hr 8 min", attendees: ["Dana Whitfield", "Priya Shah", "Sam Okafor"] });
    expect(m("e2").name).toBe("1:30 PM, Design sync, in 3 hr 8 min");
  });
  it("lists a held meeting with Open, and offers no Start while a meeting records", () => {
    const standup = stops.find((x) => x.kind === "meeting" && x.meetingId === 1);
    expect(standup).toMatchObject({ node: "past", action: "open" });
    const rec = buildTimeline({ now: NOW, locale: "en-US", schedule: { ...s, recording: true }, heroEventId: null, heroMeetingId: null, blocks: [] });
    expect(rec.filter((x) => x.kind === "meeting" && x.node === "upcoming").every((x) => x.kind === "meeting" && x.action === null)).toBe(true);
  });
  it("does not list a held meeting twice when its calendar event is there", () => {
    const linked = schedule([event("e0", "Standup", at(8, 0), 30, { recordedMeetingId: 1 })], [held(1, "Standup", at(8, 0))]);
    const out = buildTimeline({ now: NOW, locale: "en-US", schedule: linked, heroEventId: null, heroMeetingId: null, blocks: [] });
    expect(out.filter((x) => x.kind === "meeting")).toHaveLength(1);
  });
  it("is empty of content on a quiet day", () => {
    const out = buildTimeline({ now: NOW, locale: "en-US", schedule: schedule([]), heroEventId: null, heroMeetingId: null, blocks: [] });
    expect(dayIsEmpty(out)).toBe(true);
    expect(dayIsEmpty(stops)).toBe(false);
  });
  it("gives the memory read every meeting's span", () => {
    expect(meetingIntervals(s)).toEqual([[at(9, 0), at(9, 30)], [at(10, 30), at(11, 0)], [at(13, 30), at(14, 15)], [at(8, 0), at(8, 30)]]);
  });
});

describe("memory blocks", () => {
  it("reads the app name from the stored identity", () => {
    expect(appName("excel.exe")).toBe("Excel");
    expect(appName("MSEdge.profile2.Default")).toBe("Microsoft Edge");
    expect(appName("figma.exe")).toBe("Figma");
  });
  it("gives the same app the same color", () => expect(appDot("slack.exe")).toBe(appDot("Slack")));
  it("lists titles, then what was left out", () => {
    const items = blockItems(block(at(9, 0), at(9, 30), [{ app_bundle_id: "excel.exe", items: ["Pricing v3.xlsx", "Budget.xlsx"], more_items: 2 }, { app_bundle_id: "slack.exe", items: [], more_items: 0 }], 3));
    expect(items.map((i) => i.name)).toEqual(["Pricing v3.xlsx in Excel", "Budget.xlsx in Excel", "2 more in Excel", "Slack", "3 more apps"]);
  });
  it("never repeats a row name", () => {
    const items = blockItems(block(at(9, 0), at(9, 30), [{ app_bundle_id: "a.exe", items: ["x", "x"], more_items: 0 }]));
    expect(new Set(items.map((i) => i.name)).size).toBe(2);
  });
});

describe("captureLine", () => {
  it("says nothing while memory is on, and says why a stretch is empty otherwise", () => {
    expect(captureLine({ kind: "on" }, NOW, "en-US")).toBeNull();
    expect(captureLine(null, NOW, "en-US")).toBeNull();
    expect(captureLine({ kind: "off" }, NOW, "en-US")).toBe("Memory is off.");
    expect(captureLine({ kind: "paused", resumeAt: at(15, 10) }, NOW, "en-US")?.replace(/\s/g, " ")).toBe("Memory is paused until 3:10 PM.");
  });
});

describe("search filters", () => {
  it("turns each date choice into a local range", () => {
    expect(dateRange({ kind: "today" }, NOW)).toEqual({ start: at(0), end: at(0, 0, 19) });
    expect(dateRange({ kind: "yesterday" }, NOW)).toEqual({ start: at(0, 0, 17), end: at(0) });
    expect(dateRange({ kind: "last7" }, NOW)).toEqual({ start: at(0, 0, 12), end: at(0, 0, 19) });
    expect(dateRange({ kind: "custom", from: "2026-11-10", to: "2026-11-12" }, NOW)).toEqual({ start: at(0, 0, 10), end: at(0, 0, 13) });
  });
  it("refuses a custom range that is unreadable or backwards", () => {
    expect(dateRange({ kind: "custom", from: "soon", to: "2026-11-12" }, NOW)).toBeNull();
    expect(dateRange({ kind: "custom", from: "2026-11-12", to: "2026-11-10" }, NOW)).toBeNull();
  });
  it("sends only what is chosen", () => {
    expect(searchArgs({ person: null, app: null, dates: null }, NOW)).toEqual({ person: null, app: null, startUnixMs: null, endUnixMs: null });
    expect(searchArgs({ person: "Dana Whitfield", app: "excel.exe", dates: { kind: "today" } }, NOW)).toEqual({ person: "Dana Whitfield", app: "excel.exe", startUnixMs: at(0), endUnixMs: at(0, 0, 19) });
  });
});

describe("people and the line's order", () => {
  it("lists attendees most often seen first, without the user or duplicates", () => {
    expect(peopleFrom([["Sam Okafor", "Me", "Dana Whitfield"], ["sam okafor", "You", " "], ["Priya Shah", "Dana Whitfield"]])).toEqual(["Dana Whitfield", "Sam Okafor", "Priya Shah"]);
  });
  it("puts what is ahead furthest first and what is behind latest first", () => {
    const s = schedule([event("e0", "Roadmap", at(9, 0)), event("e1", "Pricing", at(10, 30)), event("e2", "Design", at(13, 30))], [held(1, "Standup", at(8, 0))]);
    const stops = buildTimeline({ now: NOW, locale: "en-US", schedule: s, heroEventId: "e1", heroMeetingId: null, blocks: [block(at(9, 40), at(10, 5))] });
    const { ahead, behind } = arrangeTimeline(stops);
    expect(ahead.map((x) => x.at)).toEqual([at(13, 30)]);
    expect(behind.map((x) => x.at)).toEqual([at(9, 40), at(9, 0), at(8, 0)]);
  });
  it("groups yesterday and older meetings, and leaves out an empty group", () => {
    const m = (id: number, d: number, h: number) => held(id, `M${id}`, at(h, 0, d));
    expect(earlierGroups([m(1, 17, 15), m(2, 17, 9), m(3, 12, 10)], NOW, "en-US").map((g) => [g.id, g.meetings.length])).toEqual([["yesterday", 2], ["lastWeek", 1]]);
    expect(earlierGroups([m(3, 12, 10)], NOW, "en-US").map((g) => g.id)).toEqual(["lastWeek"]);
    expect(earlierGroups([], NOW, "en-US")).toEqual([]);
  });
});
