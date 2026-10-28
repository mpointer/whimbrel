import { describe, expect, it } from "vitest";
import type { DeviceCount, MeetingRow, NextMeetingView, ScheduleEvent, ThreadRow } from "../generated/types";
import { timeLabel } from "../library/rows";
import { WHIMBREL } from "../skin";
import {
  ARCHETYPE_IDS,
  FIELD,
  HELD_SEPARATOR,
  PLACED,
  REFERENCE_CONTENTS,
  REFERENCE_FLOAT_THRESHOLD,
  SECTIONS,
  archetypeOf,
  devicesLine,
  earlierMeetings,
  earlierRows,
  fieldDay,
  fieldSlots,
  greetingFor,
  imageryOf,
  infoboxPlacement,
  isDrawn,
  leftovers,
  markerLabel,
  nonEmptySections,
  recapModel,
  schedule,
  scheduleItems,
  thisDeviceLabel,
  upNext,
  voiceText,
  type TodayModel,
} from "./layoutModel";

const MIN = 60_000;
const NOW = new Date(2026, 9, 22, 9, 0, 0).getTime();

const event = (id: string, minutesFromNow: number, over: Partial<ScheduleEvent> = {}): ScheduleEvent => ({
  eventId: id,
  title: `Meeting ${id}`,
  startUnixMs: NOW + minutesFromNow * MIN,
  endUnixMs: NOW + (minutesFromNow + 30) * MIN,
  attendees: ["ann@acme.test"],
  recordedMeetingId: null,
  ...over,
});

const pc: DeviceCount = { device: { deviceId: "PC", label: "This PC", thisDevice: true }, captures: 3, lastTsUnixMs: NOW };
const laptop: DeviceCount = { device: { deviceId: "LT", label: "Laptop", thisDevice: false }, captures: 0, lastTsUnixMs: null };

const meeting = (id: number, minutesFromNow: number, title = `Held ${id}`): MeetingRow => ({
  device: pc.device,
  hasSummary: false,
  id,
  status: "done",
  title,
  trigger: "calendar",
  tsUnixMs: NOW + minutesFromNow * MIN,
});

function model(over: Partial<TodayModel> = {}): TodayModel {
  return {
    now: NOW,
    name: "Mike Pointer",
    schedule: { events: [], earlierToday: [], recording: false },
    next: null,
    earlier: [],
    openLoops: [],
    recap: null,
    devices: [pc],
    ...over,
  };
}

const withSchedule = (events: ScheduleEvent[], earlierToday: MeetingRow[] = [], recording = false) => model({ schedule: { events, earlierToday, recording } });

describe("which layout", () => {
  it("knows the six archetypes and shows Standard for one it does not know", () => {
    expect(ARCHETYPE_IDS).toEqual(["standard", "terminal", "glass", "field", "almanac", "reference"]);
    expect(archetypeOf("field")).toBe("field");
    expect(archetypeOf("hologram")).toBe("standard");
    expect(archetypeOf(undefined)).toBe("standard");
  });
});

describe("the sections each layout places", () => {
  const macFiles = import.meta.glob("../../../../macos/Whimbrel/UI/Today/Archetypes/*TodayLayout.swift", { query: "?raw", import: "default", eager: true }) as Record<string, string>;
  const macSource = (name: string): string => {
    const hit = Object.entries(macFiles).find(([path]) => path.endsWith(`/${name}TodayLayout.swift`));
    if (!hit) throw new Error(`the Mac's ${name}TodayLayout.swift is not where the test looks`);
    return hit[1];
  };
  const sectionNames = ["greeting", "composer", "upNext", "schedule", "earlier", "threads", "recap", "devices"];

  /** The order the Mac file draws its sections in: greeting first, then each `sections.contains(.x)` as it
   *  appears, then any placed section the file draws without asking (Terminal's device footer). */
  function macOrder(name: string): string[] {
    const source = macSource(name);
    const body = source.slice(source.indexOf("var body"), source.indexOf("// MARK: pieces"));
    const asked = [...body.matchAll(/sections\.contains\(\.(\w+)\)/g)].map((m) => m[1]);
    const ordered = ["greeting", ...asked.filter((s, i) => asked.indexOf(s) === i)];
    const line = /static var placedSections: Set<TodaySection> \{ (.*) \}/.exec(source)?.[1] ?? "";
    const placed = line.includes("allCases") ? sectionNames : [...line.matchAll(/\.(\w+)/g)].map((m) => m[1]);
    return [...ordered, ...placed.filter((s) => !ordered.includes(s))];
  }

  it("section order per archetype: Terminal, Glass, Field and Almanac draw what the Mac's files draw, in that order", () => {
    for (const [id, file] of [["terminal", "Terminal"], ["glass", "Glass"], ["field", "Field"], ["almanac", "Almanac"]] as const) {
      expect(macOrder(file).length, file).toBeGreaterThan(3);
      expect([...PLACED[id]], id).toEqual(macOrder(file));
    }
  });

  it("Reference places every section and lists them in the Mac's Contents order", () => {
    const source = macSource("Reference");
    const contents = [...source.matchAll(/Entry\(section: \.(\w+)/g)].map((m) => m[1]);
    expect(contents).toEqual([...REFERENCE_CONTENTS]);
    expect(new Set(PLACED.reference)).toEqual(new Set(SECTIONS));
    expect(source).toContain("Set(TodaySection.allCases)");
  });

  it("Glass and Field leave the rest to the plain list, as the Mac's chrome does", () => {
    const placedLine = (name: string) => /static var placedSections: Set<TodaySection> \{ (.*) \}/.exec(macSource(name))?.[1];
    expect(placedLine("Glass")).toBe("[.greeting, .composer, .upNext, .schedule, .recap]");
    expect(placedLine("Field")).toBe("[.greeting, .composer, .upNext, .schedule, .earlier]");
  });

  it("every layout places the greeting and the composer first (Reference puts the infobox between)", () => {
    for (const id of ARCHETYPE_IDS) {
      expect(PLACED[id][0], id).toBe("greeting");
      expect(PLACED[id], id).toContain("composer");
    }
    expect(PLACED.reference.slice(0, 3)).toEqual(["greeting", "upNext", "composer"]);
  });

  it("Standard is Windows' own Today: no separate up next, no Earlier list", () => {
    expect([...PLACED.standard]).toEqual(["greeting", "composer", "schedule", "recap", "threads", "devices"]);
  });

  it("places each section at most once and uses only known sections", () => {
    for (const id of ARCHETYPE_IDS) {
      const placed = [...PLACED[id]];
      expect(new Set(placed).size, id).toBe(placed.length);
      for (const s of placed) expect(SECTIONS, `${id} ${s}`).toContain(s);
    }
  });
});

describe("voice", () => {
  it("substitutes {name} wherever it appears", () => {
    expect(voiceText({ greetingMorning: "Hi {name}, yes {name}" }, "greetingMorning", "Ann")).toBe("Hi Ann, yes Ann");
  });

  it("voice strings substitute {name}", () => {
    expect(voiceText(WHIMBREL.skin.voice, "greetingMorning", "Mike")).toBe("Good morning, Mike");
    expect(voiceText(WHIMBREL.skin.voice, "upNextHeader")).toBe("Up next");
  });

  it("reads a key the skin leaves out as Whimbrel's, and never prints a placeholder", () => {
    expect(voiceText({}, "recapHeader")).toBe("Daily recap");
    expect(voiceText({ greetingEvening: "Evening, {name}" }, "greetingEvening")).toBe("Evening, ");
  });

  it("words the greeting in the skin's voice and keeps the date line", () => {
    const voice = { ...WHIMBREL.skin.voice, greetingMorning: "Rise, {name}!", greetingAfternoon: "Afternoon {name}", greetingEvening: "Night {name}" };
    expect(greetingFor("Mike Pointer", new Date(2026, 9, 22, 8), voice)).toEqual({ kicker: "Thursday, October 22", title: "Rise, Mike!" });
    expect(greetingFor("Mike", new Date(2026, 9, 22, 12), voice).title).toBe("Afternoon Mike");
    expect(greetingFor("Mike", new Date(2026, 9, 22, 17), voice).title).toBe("Night Mike");
    expect(greetingFor("Mike", new Date(2026, 9, 22, 8), WHIMBREL.skin.voice).title).toBe("Good morning, Mike");
  });

  it("makes the date the title when there is no usable name, whatever the voice", () => {
    expect(greetingFor("", new Date(2026, 9, 22, 8), WHIMBREL.skin.voice)).toEqual({ kicker: null, title: "Thursday, October 22" });
    expect(greetingFor("me@acme.test", new Date(2026, 9, 22, 8), WHIMBREL.skin.voice).kicker).toBeNull();
  });
});

describe("up next and the schedule", () => {
  const next = (eventId: string, brief: boolean): NextMeetingView =>
    ({ eventId, title: "x", startUnixMs: NOW, brief: brief ? ({ summary: "s" } as NextMeetingView["brief"]) : null }) as NextMeetingView;

  it("up next is the first event that has no recording", () => {
    const m = withSchedule([event("a", -10, { recordedMeetingId: 5 }), event("b", 10), event("c", 40)]);
    expect(upNext(m)?.id).toBe("b");
    expect(upNext(withSchedule([]))).toBeNull();
    expect(upNext(model({ schedule: null }))).toBeNull();
  });

  it("the up-next row has prep state: ready only when the next meeting is this event and has a brief", () => {
    const events = [event("b", 10), event("c", 40)];
    expect(upNext({ ...withSchedule(events), next: next("b", true) })?.prep).toBe("ready");
    expect(upNext({ ...withSchedule(events), next: next("b", false) })?.prep).toBe("none");
    expect(upNext({ ...withSchedule(events), next: next("c", true) })?.prep).toBe("none");
    expect(upNext(withSchedule(events))?.prep).toBe("none");
  });

  it("no other row has prep state", () => {
    const m = { ...withSchedule([event("b", 10), event("c", 40)]), next: next("c", true) };
    expect(schedule(m, false).upcoming.map((r) => r.prep)).toEqual([null, null]);
  });

  it("leaves the up-next event out of the list when the layout places it", () => {
    const m = withSchedule([event("b", 10), event("c", 40), event("d", 20)]);
    expect(schedule(m, true).upcoming.map((r) => r.id)).toEqual(["d", "c"]);
    expect(schedule(m, false).upcoming.map((r) => r.id)).toEqual(["b", "d", "c"]);
  });

  it("lists each group in ascending time order", () => {
    const m = withSchedule([event("z", 50), event("y", 20)], [meeting(2, -30), meeting(1, -90), meeting(3, -60)]);
    expect(schedule(m, false).upcoming.map((r) => r.id)).toEqual(["y", "z"]);
    expect(schedule(m, false).held.map((r) => r.id)).toEqual([1, 3, 2]);
  });

  it("puts the separator between the groups only when both have rows", () => {
    const both = scheduleItems(schedule(withSchedule([event("a", 10)], [meeting(1, -60)]), false));
    expect(both.map((i) => i.kind)).toEqual(["event", "separator", "held"]);
    expect(scheduleItems(schedule(withSchedule([event("a", 10)]), false)).map((i) => i.kind)).toEqual(["event"]);
    expect(scheduleItems(schedule(withSchedule([], [meeting(1, -60)]), false)).map((i) => i.kind)).toEqual(["held"]);
    expect(HELD_SEPARATOR).toBe("Earlier today");
  });

  it("offers Start for an event to come, Open once recorded, and no Start while recording", () => {
    const rows = schedule(withSchedule([event("a", 10), event("b", 40, { recordedMeetingId: 9 })]), false).upcoming;
    expect(rows.map((r) => r.action)).toEqual(["start", "open"]);
    expect(rows[1].recordedMeetingId).toBe(9);
    const recording = schedule(withSchedule([event("a", 10), event("b", 40, { recordedMeetingId: 9 })], [], true), false).upcoming;
    expect(recording.map((r) => r.action)).toEqual([null, "open"]);
  });

  it("words the time, countdown and invitees like the schedule card", () => {
    const [row] = schedule(withSchedule([event("a", 9, { attendees: ["a@x", "b@x", "c@x"] })]), false).upcoming;
    expect(row.timeText).toBe(timeLabel(NOW + 9 * MIN));
    expect(row.relative).toBe("In 9m");
    expect(row.attendees).toBe("a@x, b@x +1");
    expect(schedule(withSchedule([event("a", 5, { attendees: [] })]), false).upcoming[0].attendees).toBeNull();
  });

  it("earlier days list newest first with a date and time", () => {
    const m = model({ earlier: [meeting(8, -26 * 60), meeting(7, -50 * 60)] });
    const rows = earlierRows(m);
    expect(rows.map((r) => r.id)).toEqual([8, 7]);
    expect(rows[0].timeText).toContain("·");
  });
});

describe("what has something to show", () => {
  it("always has the greeting, the composer and the devices footer", () => {
    expect(nonEmptySections(model())).toEqual(["greeting", "composer", "devices"]);
  });

  it("adds each section when it has something", () => {
    const m = {
      ...withSchedule([event("a", 10), event("b", 20)], [meeting(1, -60)]),
      earlier: [meeting(2, -26 * 60)],
      openLoops: [{ id: 1 } as ThreadRow],
      recap: { kind: "digest", text: "x" } as const,
    };
    expect(nonEmptySections(m)).toEqual([...SECTIONS]);
  });

  it("has a schedule from held meetings alone, and an up next only from an unrecorded event", () => {
    expect(nonEmptySections(withSchedule([], [meeting(1, -60)]))).toContain("schedule");
    expect(nonEmptySections(withSchedule([], [meeting(1, -60)]))).not.toContain("upNext");
    expect(nonEmptySections(withSchedule([event("a", -5, { recordedMeetingId: 3 })]))).not.toContain("upNext");
  });

  it("leaves to the plain list what a layout does not place, in the standard order", () => {
    const m = {
      ...withSchedule([event("a", 10), event("b", 20)]),
      earlier: [meeting(2, -26 * 60)],
      openLoops: [{ id: 1 } as ThreadRow],
      recap: { kind: "digest", text: "x" } as const,
    };
    expect(leftovers("glass", m)).toEqual(["earlier", "threads", "devices"]);
    expect(leftovers("field", m)).toEqual(["threads", "recap", "devices"]);
    expect(leftovers("terminal", m)).toEqual([]);
    expect(leftovers("almanac", m)).toEqual([]);
    expect(leftovers("reference", m)).toEqual([]);
  });

  it("never repeats the up-next event in the leftover schedule", () => {
    // Nothing is placed by an imaginary layout that skips schedule; a lone event is up next, so it has no schedule leftover.
    const lone = withSchedule([event("a", 10)]);
    expect(nonEmptySections(lone)).toContain("schedule");
    const rest = schedule(lone, true);
    expect(rest.upcoming.length + rest.held.length).toBe(0);
  });
});

describe("devices", () => {
  it("says this PC's activity and how many others reported", () => {
    expect(devicesLine([pc, laptop])).toMatch(/^3 captures · last .+ · 1 other device$/);
    expect(devicesLine([pc, laptop, { ...laptop, device: { ...laptop.device, deviceId: "T2" } }])).toMatch(/· 2 other devices$/);
    expect(devicesLine([{ ...pc, captures: 0, lastTsUnixMs: null }])).toBe("Nothing yet today");
    expect(devicesLine([])).toBe("Nothing yet today");
  });

  it("labels the chip with this PC", () => {
    expect(thisDeviceLabel([laptop, pc])).toBe("This PC");
    expect(thisDeviceLabel([laptop])).toBe("Laptop");
    expect(thisDeviceLabel([])).toBeNull();
  });
});

describe("the Field pitch", () => {
  const at = (...minutes: number[]) => minutes.map((m) => NOW + m * MIN);

  it("places no markers for no meetings", () => {
    expect(fieldSlots([], 600)).toEqual([]);
  });

  it("puts one marker at the left, with the rest of the pitch for its label", () => {
    const [slot] = fieldSlots(at(10), 600);
    expect(slot.centerX).toBe(12);
    expect(slot.labelX).toBe(0);
    expect(slot.labelWidth).toBe(600);
    expect(slot.row).toBe(0);
  });

  it("spaces markers by time, first at the left and last at the room kept for its label", () => {
    const slots = fieldSlots(at(0, 30, 120), 700);
    expect(slots[0].centerX).toBe(12);
    expect(slots[2].centerX).toBeCloseTo(700 - FIELD.minLabel);
    expect(slots[1].centerX).toBeCloseTo(12 + (700 - FIELD.minLabel - 12) * 0.25);
  });

  it("spreads markers evenly when their times are the same", () => {
    const slots = fieldSlots(at(10, 10, 10), 700);
    const gaps = [slots[1].centerX - slots[0].centerX, slots[2].centerX - slots[1].centerX];
    expect(gaps[0]).toBeCloseTo(gaps[1]);
    expect(gaps[0]).toBeGreaterThanOrEqual(FIELD.minGap - 1e-9);
  });

  it("alternates the labels above and below the line", () => {
    expect(fieldSlots(at(0, 10, 20, 30, 40), 800).map((s) => s.row)).toEqual([0, 1, 0, 1, 0]);
  });

  it("never lets two markers closer than the gap (or evenly squeezed on a narrow pitch), in order", () => {
    for (const width of [120, 240, 400, 900]) {
      const slots = fieldSlots(at(0, 1, 2, 3, 4, 5, 6, 7), width);
      const hi = Math.max(12, width - FIELD.minLabel);
      const gap = Math.min(FIELD.minGap, (hi - 12) / 7);
      for (let i = 1; i < slots.length; i++) expect(slots[i].centerX - slots[i - 1].centerX, `${width} ${i}`).toBeGreaterThanOrEqual(gap - 1e-9);
      for (const s of slots) expect(s.centerX).toBeGreaterThanOrEqual(12 - 1e-9);
    }
  });

  it("never overlaps two labels in the same row, whatever the times and the width", () => {
    let seed = 12345;
    const random = () => ((seed = (seed * 1103515245 + 12345) & 0x7fffffff) / 0x7fffffff);
    for (let trial = 0; trial < 300; trial++) {
      const n = 1 + Math.floor(random() * 8);
      const starts = Array.from({ length: n }, () => Math.floor(random() * 600)).sort((a, b) => a - b).map((m) => NOW + m * MIN);
      const width = 150 + Math.floor(random() * 800);
      const slots = fieldSlots(starts, width);
      for (let i = 0; i + 2 < n; i++) {
        expect(slots[i].labelX + slots[i].labelWidth, `${trial} ${i}`).toBeLessThanOrEqual(slots[i + 2].labelX - FIELD.labelGap + 1e-9);
      }
      for (const s of slots) {
        expect(s.labelWidth).toBeGreaterThanOrEqual(0);
        expect(s.labelX + s.labelWidth).toBeLessThanOrEqual(width + 1e-9);
      }
    }
  });

  it("puts up to eight upcoming meetings on the pitch, up next highlighted, and the rest in a list", () => {
    const events = Array.from({ length: 11 }, (_, i) => event(`e${i}`, 10 + i * 10));
    const day = fieldDay(withSchedule(events));
    expect(day.markers).toHaveLength(8);
    expect(day.markers.map((m) => m.isNext)).toEqual([true, false, false, false, false, false, false, false]);
    expect(day.overflow.upcoming.map((r) => r.id)).toEqual(["e8", "e9", "e10"]);
    expect(day.moreLabel).toBe("+3 more");
  });

  it("fills the slots left with the latest held meetings, in time order", () => {
    const events = [event("a", 30), event("b", 60)];
    const held = [meeting(1, -300), meeting(2, -240), meeting(3, -180), meeting(4, -120), meeting(5, -60), meeting(6, -30), meeting(7, -10)];
    const day = fieldDay(withSchedule(events, held));
    expect(day.markers.map((m) => m.id)).toEqual(["held-2", "held-3", "held-4", "held-5", "held-6", "held-7", "a", "b"]);
    expect(day.markers.filter((m) => m.isHeld)).toHaveLength(6);
    expect(day.overflow.held.map((r) => r.id)).toEqual([1]);
    expect(day.moreLabel).toBe("+1 more");
  });

  it("has no overflow when everything fits, and no markers for an empty day", () => {
    expect(fieldDay(withSchedule([event("a", 10)])).moreLabel).toBeNull();
    expect(fieldDay(withSchedule([])).markers).toEqual([]);
  });

  it("gives a marker the action its row would have", () => {
    const day = fieldDay(withSchedule([event("a", 10), event("b", 30, { recordedMeetingId: 4 })], [meeting(9, -60)]));
    const byId = Object.fromEntries(day.markers.map((m) => [m.id, m.action]));
    expect(byId.a).toEqual({ kind: "start", eventId: "a" });
    expect(byId.b).toEqual({ kind: "open", meetingId: 4 });
    expect(byId["held-9"]).toEqual({ kind: "open", meetingId: 9 });
    const recording = fieldDay(withSchedule([event("a", 10)], [], true));
    expect(recording.markers[0].action).toBeNull();
  });

  it("says the verb, the meeting, the time and whether it is next", () => {
    const [m] = fieldDay(withSchedule([event("a", 10)])).markers;
    expect(markerLabel(m)).toBe(`Start Meeting a, ${timeLabel(NOW + 10 * MIN)}, up next`);
    expect(markerLabel({ ...m, action: null, isNext: false })).toBe(`Meeting a, ${timeLabel(NOW + 10 * MIN)}`);
    expect(markerLabel({ ...m, action: { kind: "open", meetingId: 1 }, isNext: false })).toMatch(/^Open Meeting a/);
  });
});

describe("Reference", () => {
  it("floats the infobox from 700 and stacks it below", () => {
    expect(REFERENCE_FLOAT_THRESHOLD).toBe(700);
    expect(infoboxPlacement(699)).toBe("stacked");
    expect(infoboxPlacement(700)).toBe("floated");
    expect(infoboxPlacement(1400)).toBe("floated");
  });
});

describe("imagery", () => {
  const manifest = (imagery: { asset: string; placement: string; textFree: boolean }[]) => ({ decoration: { imagery } });

  it("textFree image is never placed on background", () => {
    expect(isDrawn({ placement: "background", textFree: true })).toBe(false);
    expect(isDrawn({ placement: "background", textFree: false })).toBe(true);
    expect(isDrawn({ placement: "header", textFree: true })).toBe(true);
    expect(isDrawn({ placement: "badge", textFree: true })).toBe(true);
    const kept = imageryOf(manifest([{ asset: "a.png", placement: "background", textFree: true }, { asset: "b.png", placement: "header", textFree: true }]));
    expect(kept.map((i) => i.asset)).toEqual(["b.png"]);
  });

  it("covers an image fully until the shell has measured its scrim", () => {
    const [unmeasured] = imageryOf(manifest([{ asset: "a.png", placement: "background", textFree: false }]));
    expect(unmeasured.scrim).toBe(1);
    const [measured] = imageryOf(manifest([{ asset: "a.png", placement: "background", textFree: false }]), { "a.png": 0.42 });
    expect(measured.scrim).toBe(0.42);
  });

  it("drops a placement it does not know and a skin with no images has none", () => {
    expect(imageryOf(manifest([{ asset: "a.png", placement: "wallpaper", textFree: false }]))).toEqual([]);
    expect(imageryOf(WHIMBREL.skin)).toEqual([]);
  });
});

describe("the recap and earlier days", () => {
  const overview = (briefRecap: { summary: string; from?: { label: string } | null } | null) => ({ briefRecap }) as Parameters<typeof recapModel>[0];
  const digest = { day: "d", summary: "digest text", openLoops: [], cached: true, emptyDay: false };

  it("prefers the Daily Brief, then the digest, then the error, then writing", () => {
    expect(recapModel(overview({ summary: "brief", from: { label: "Mac" } }), digest, false, null)).toEqual({ kind: "brief", text: "brief", origin: "Mac" });
    expect(recapModel(overview({ summary: "brief" }), null, false, null)).toEqual({ kind: "brief", text: "brief", origin: null });
    expect(recapModel(overview(null), digest, false, "boom")).toEqual({ kind: "digest", text: "digest text" });
    expect(recapModel(overview(null), null, false, "boom")).toEqual({ kind: "failed", message: "boom", retryEnabled: true });
    expect(recapModel(overview(null), null, true, "boom")).toEqual({ kind: "failed", message: "boom", retryEnabled: false });
    expect(recapModel(overview(null), null, true, null)).toEqual({ kind: "writing" });
    expect(recapModel(overview(null), null, false, null)).toBeNull();
  });

  it("lists up to five meetings from before today, newest first", () => {
    const day = 24 * 60 * MIN;
    const rows = [meeting(1, 5), meeting(2, -2 * day / MIN), meeting(3, -3 * day / MIN), meeting(4, -1 * day / MIN), meeting(5, -4 * day / MIN), meeting(6, -5 * day / MIN), meeting(7, -6 * day / MIN), meeting(8, -7 * day / MIN)];
    const got = earlierMeetings(rows, NOW).map((m) => m.id);
    expect(got).toEqual([4, 2, 3, 5, 6]);
    expect(earlierMeetings([meeting(9, -30)], NOW)).toEqual([]); // 8:30 today is today
  });
});
