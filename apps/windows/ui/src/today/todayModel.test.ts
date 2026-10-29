import { describe, expect, it } from "vitest";
import type {
  BriefRecap,
  DeviceCount,
  NextMeetingView,
  PrepBriefView,
  ProviderKey,
  RoutingSettings,
  ScheduleEvent,
  TodayOverview,
  TodayRecap,
} from "../generated/types";
import { timeLabel } from "../library/rows";
import {
  askAvailable,
  askModelLabel,
  attendeesLine,
  briefRecapNote,
  citedSources,
  composerPrompt,
  deviceLine,
  greeting,
  prepLine,
  prepStatus,
  recapFor,
  recapNote,
  relativeStart,
  rowAction,
  shouldGenerate,
  threadLine,
} from "./todayModel";

const pc: DeviceCount = { device: { deviceId: "PC", label: "This PC", thisDevice: true }, captures: 0, lastTsUnixMs: null };
const overview = (over: Partial<TodayOverview> = {}): TodayOverview => ({
  day: "2026-09-11",
  recap: null,
  openLoops: [],
  latestMeeting: null,
  devices: [pc],
  ...over,
});
const recap = (over: Partial<TodayRecap> = {}): TodayRecap => ({
  day: "2026-09-11",
  summary: "Wrote the plan.",
  openLoops: [],
  cached: false,
  emptyDay: false,
  ...over,
});

describe("shouldGenerate", () => {
  it("generates once on the first look of the day when nothing is cached", () => {
    expect(shouldGenerate(overview(), null, null)).toBe(true);
    expect(shouldGenerate(overview(), "2026-09-10", null)).toBe(true);
    expect(shouldGenerate(overview({ recap: recap({ cached: true }) }), null, null)).toBe(false);
  });

  it("does not generate a digest when today's Daily Brief is there, even on the first look", () => {
    const brief: BriefRecap = { summary: "Three meetings.", ranAtUnixMs: 1, from: null };
    expect(shouldGenerate(overview({ briefRecap: brief }), null, null)).toBe(false);
    expect(shouldGenerate(overview({ briefRecap: null }), null, null)).toBe(true);
  });

  it("does not repeat after an attempt, unless it found an empty day and captures arrived since", () => {
    expect(shouldGenerate(overview(), "2026-09-11", recap())).toBe(false);
    expect(shouldGenerate(overview(), "2026-09-11", null)).toBe(false);
    const empty = recap({ emptyDay: true, summary: "No activity captured today." });
    expect(shouldGenerate(overview(), "2026-09-11", empty)).toBe(false);
    expect(shouldGenerate(overview({ devices: [{ ...pc, captures: 3, lastTsUnixMs: 1 }] }), "2026-09-11", empty)).toBe(true);
  });
});

describe("briefRecapNote", () => {
  it("names the other device, or gives this PC's own run its time", () => {
    const mac = { deviceId: "MAC", label: "MacBook Pro", thisDevice: false };
    expect(briefRecapNote({ summary: "s", ranAtUnixMs: 1, from: mac })).toBe("From MacBook Pro");
    expect(briefRecapNote({ summary: "s", ranAtUnixMs: Date.UTC(2026, 9, 17, 12, 30), from: null })).toBe(
      `Daily Brief, written at ${timeLabel(Date.UTC(2026, 9, 17, 12, 30))}.`,
    );
  });
});

describe("recapFor", () => {
  it("prefers the cached recap and drops one from another day", () => {
    const cached = recap({ cached: true });
    expect(recapFor(overview({ recap: cached }), recap({ summary: "older" }))).toBe(cached);
    const inHand = recap({ emptyDay: true });
    expect(recapFor(overview(), inHand)).toBe(inHand);
    expect(recapFor(overview(), recap({ day: "2026-09-10" }))).toBeNull();
  });
});

describe("lines", () => {
  it("describes each device's day", () => {
    expect(deviceLine(pc)).toBe("Nothing yet today");
    const mac: DeviceCount = { device: { deviceId: "M", label: "MacBook Pro", thisDevice: false }, captures: 1204, lastTsUnixMs: 1_757_613_900_000 };
    expect(deviceLine(mac)).toBe(`1,204 captures · last ${timeLabel(1_757_613_900_000)}`);
    expect(deviceLine({ ...mac, captures: 1 })).toMatch(/^1 capture · last /);
  });

  it("says who a thread is waiting on, and how a recap was made", () => {
    const thread = { id: 1, subject: "Budget", lastActivityTsUnixMs: 1, lastMessageFrom: "Dana", status: "waiting_on_me" };
    expect(threadLine(thread)).toBe("Waiting on you · last from Dana");
    expect(threadLine({ ...thread, status: "waiting_on_them", lastMessageFrom: "" })).toBe("Waiting on them");
    expect(recapNote(recap({ cached: true }))).toBe("Written earlier today.");
    expect(recapNote(recap())).toBe("Just written.");
    expect(recapNote(recap({ emptyDay: true }))).toMatch(/^Nothing captured yet today/);
  });
});

describe("the next-meeting card", () => {
  const brief: PrepBriefView = {
    generatedAtUnixMs: Date.UTC(2026, 9, 15, 17, 0),
    summary: "Pricing is open.",
    summarySources: ["M1"],
    attendees: [],
    openItems: [],
    openQuestions: [],
    sources: [
      { id: "M1", title: "Acme kickoff", meetingId: 7 },
      { id: "E1", title: "Re: quote", meetingId: null },
      { id: "M2", title: "Peer's meeting" },
    ],
  };

  it("says when the brief was made, or why there is none", () => {
    expect(prepStatus(brief)).toBe(`Prepared at ${timeLabel(brief.generatedAtUnixMs)}.`);
    expect(prepStatus(null)).toMatch(/No brief yet/);
    expect(prepStatus(undefined)).toMatch(/turned on in Automations/);
  });

  it("lists the sources a claim cites, in order, and skips ids the brief does not list", () => {
    expect(citedSources(brief, ["E1", "M1"]).map((s) => s.title)).toEqual(["Re: quote", "Acme kickoff"]);
    expect(citedSources(brief, ["M9", "M1"]).map((s) => s.title)).toEqual(["Acme kickoff"]);
    expect(citedSources(brief, [])).toEqual([]);
  });

  it("keeps which cited sources open a meeting here: only those the shell vouched for", () => {
    const opens = citedSources(brief, ["M1", "E1", "M2"]).map((s) => s.meetingId ?? null);
    expect(opens).toEqual([7, null, null]);
  });
});

describe("greeting", () => {
  // Local times: the greeting follows the clock on this PC.
  const at = (h: number, m = 0) => new Date(2026, 9, 6, h, m); // Tuesday 6 October 2026

  it("greets by the first word of the Profile name, by time of day, with the date above", () => {
    expect(greeting("Mike Pointer", at(9))).toEqual({ kicker: "Tuesday, October 6", title: "Good morning, Mike" });
    expect(greeting("Mike Pointer", at(13)).title).toBe("Good afternoon, Mike");
    expect(greeting("Mike Pointer", at(19)).title).toBe("Good evening, Mike");
  });

  it("switches at noon and at 5 pm", () => {
    expect(greeting("Mike", at(11, 59)).title).toBe("Good morning, Mike");
    expect(greeting("Mike", at(12, 0)).title).toBe("Good afternoon, Mike");
    expect(greeting("Mike", at(16, 59)).title).toBe("Good afternoon, Mike");
    expect(greeting("Mike", at(17, 0)).title).toBe("Good evening, Mike");
    expect(greeting("Mike", at(0, 5)).title).toBe("Good morning, Mike");
  });

  it("uses a single-word name, and trims leading space", () => {
    expect(greeting("Mike", at(9)).title).toBe("Good morning, Mike");
    expect(greeting("   Mike   Pointer", at(9)).title).toBe("Good morning, Mike");
  });

  it("falls back to the weekday and date, with no extra line, for an empty, blank or email name", () => {
    for (const name of ["", "   ", "a@b.com", "Mike a@b.com"]) {
      expect(greeting(name, at(9))).toEqual({ kicker: null, title: "Tuesday, October 6" });
    }
  });
});

describe("the Ask composer", () => {
  const routing = (over: Partial<RoutingSettings> = {}): RoutingSettings => ({
    anthropicModel: "claude-haiku",
    googleModel: "gemini-flash",
    localOnly: false,
    ollamaModel: "gemma-local",
    openaiModel: "gpt-mini",
    openrouterModel: "",
    preferredProvider: "ollama",
    cheapFirst: true,
    useLocalModel: true,
    ...over,
  });
  const keys = (...saved: ProviderKey["provider"][]): ProviderKey[] =>
    (["anthropic", "openai", "google"] as const).map((provider) => ({ provider, masked: saved.includes(provider) ? "••••1234" : null }));

  it("is available with a local model, whatever keys or local-only say", () => {
    expect(askAvailable(routing(), keys())).toBe(true);
    expect(askAvailable(routing({ localOnly: true }), keys())).toBe(true);
  });

  it("needs a saved key for the preferred hosted provider", () => {
    expect(askAvailable(routing({ preferredProvider: "anthropic" }), keys())).toBe(false);
    expect(askAvailable(routing({ preferredProvider: "anthropic" }), keys("anthropic"))).toBe(true);
    expect(askAvailable(routing({ preferredProvider: "openai" }), keys("anthropic", "google"))).toBe(false);
    expect(askAvailable(routing({ preferredProvider: "google" }), keys("google"))).toBe(true);
  });

  it("is off for a hosted provider while local-only mode is on, even with its key", () => {
    expect(askAvailable(routing({ preferredProvider: "anthropic", localOnly: true }), keys("anthropic"))).toBe(false);
  });

  it("labels the preferred provider's model", () => {
    expect(askModelLabel(routing())).toBe("gemma-local");
    expect(askModelLabel(routing({ preferredProvider: "anthropic" }))).toBe("claude-haiku");
    expect(askModelLabel(routing({ preferredProvider: "openai" }))).toBe("gpt-mini");
    expect(askModelLabel(routing({ preferredProvider: "google" }))).toBe("gemini-flash");
  });

  it("sends the trimmed text and nothing for a blank field", () => {
    expect(composerPrompt("  what did we decide?  ")).toBe("what did we decide?");
    expect(composerPrompt("")).toBeNull();
    expect(composerPrompt("   \n\t ")).toBeNull();
  });
});

describe("the schedule card", () => {
  const MIN = 60_000;
  const HOUR = 60 * MIN;
  const NOW = 1_000_000_000_000;
  const event = (over: Partial<ScheduleEvent> = {}): ScheduleEvent => ({
    eventId: "e1",
    title: "Acme sync",
    startUnixMs: NOW + 30 * MIN,
    endUnixMs: NOW + 60 * MIN,
    attendees: [],
    recordedMeetingId: null,
    ...over,
  });

  describe("relativeStart", () => {
    const rel = (startOffset: number, length = 30 * MIN) => relativeStart(NOW + startOffset, NOW + startOffset + length, NOW);

    it("counts minutes up and hours down", () => {
      expect(rel(9 * MIN)).toBe("In 9m");
      expect(rel(2 * HOUR + 5 * MIN)).toBe("In 2h");
      expect(rel(59_000)).toBe("In 1m");
      expect(rel(HOUR)).toBe("In 1h");
      expect(rel(59 * MIN + 30_000)).toBe("In 1h");
    });

    it("says Now from the start until the end, and nothing after", () => {
      expect(rel(0)).toBe("Now");
      expect(rel(-10 * MIN)).toBe("Now");
      expect(relativeStart(NOW - 30 * MIN, NOW, NOW)).toBeNull();
      expect(relativeStart(NOW - 60 * MIN, NOW - 30 * MIN, NOW)).toBeNull();
    });

    it("says nothing for an event more than 12 hours away", () => {
      expect(rel(12 * HOUR)).toBe("In 12h");
      expect(rel(12 * HOUR + MIN)).toBeNull();
    });
  });

  it("lists up to two invitees and counts the rest", () => {
    expect(attendeesLine([])).toBeNull();
    expect(attendeesLine(["a@x.test"])).toBe("a@x.test");
    expect(attendeesLine(["a@x.test", "b@x.test"])).toBe("a@x.test, b@x.test");
    expect(attendeesLine(["a@x.test", "b@x.test", "c@x.test"])).toBe("a@x.test, b@x.test +1");
    expect(attendeesLine(["a@x.test", "b@x.test", "c@x.test", "d@x.test"])).toBe("a@x.test, b@x.test +2");
  });

  it("offers Open for a recorded event, Start otherwise, and no Start while recording", () => {
    expect(rowAction(event({ recordedMeetingId: 4 }), false)).toBe("open");
    expect(rowAction(event({ recordedMeetingId: 4 }), true)).toBe("open");
    expect(rowAction(event(), false)).toBe("start");
    expect(rowAction(event(), true)).toBeNull();
  });

  describe("prepLine", () => {
    const next = (brief: NextMeetingView["brief"]): NextMeetingView => ({ eventId: "e1", title: "Acme sync", startUnixMs: NOW, brief });
    const brief = { generatedAtUnixMs: NOW } as NonNullable<NextMeetingView["brief"]>;

    it("is Prep ready or No prep yet on the next meeting's row only", () => {
      expect(prepLine(event(), next(brief))).toBe("ready");
      expect(prepLine(event(), next(null))).toBe("none");
      expect(prepLine(event({ eventId: "other" }), next(brief))).toBeNull();
      expect(prepLine(event(), null)).toBeNull();
    });

    it("is absent once the event is recorded", () => {
      expect(prepLine(event({ recordedMeetingId: 4 }), next(brief))).toBeNull();
    });
  });
});
