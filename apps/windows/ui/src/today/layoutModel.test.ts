import { describe, expect, it } from "vitest";
import type { MeetingRow } from "../generated/types";
import { WHIMBREL } from "../skin";
import { ARCHETYPE_IDS, archetypeOf, earlierMeetings, greetingFor, hasScrim, imageryOf, isDrawn, localizedVoice, recapModel, voiceText } from "./layoutModel";

const MIN = 60_000;
const NOW = new Date(2026, 9, 22, 9, 0, 0).getTime();

const pc = { deviceId: "PC", label: "This PC", thisDevice: true };

const meeting = (id: number, minutesFromNow: number, title = `Held ${id}`): MeetingRow => ({
  device: pc,
  hasSummary: false,
  attendeeCount: 0,
  actionItemCount: 0,
  id,
  status: "done",
  title,
  trigger: "calendar",
  tsUnixMs: NOW + minutesFromNow * MIN,
});

/** The locale the models are given here, so times read the same on every machine. */
const LOCALE = "en-US";

describe("which layout", () => {
  it("knows the six archetypes and shows Standard for one it does not know", () => {
    expect(ARCHETYPE_IDS).toEqual(["standard", "terminal", "glass", "field", "almanac", "reference"]);
    expect(archetypeOf("field")).toBe("field");
    expect(archetypeOf("hologram")).toBe("standard");
    expect(archetypeOf(undefined)).toBe("standard");
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
    expect(greetingFor("Mike Pointer", new Date(2026, 9, 22, 8), voice, LOCALE)).toEqual({ kicker: "Thursday, October 22", title: "Rise, Mike!" });
    expect(greetingFor("Mike", new Date(2026, 9, 22, 12), voice, LOCALE).title).toBe("Afternoon Mike");
    expect(greetingFor("Mike", new Date(2026, 9, 22, 17), voice, LOCALE).title).toBe("Night Mike");
    expect(greetingFor("Mike", new Date(2026, 9, 22, 8), WHIMBREL.skin.voice, LOCALE).title).toBe("Good morning, Mike");
  });

  it("makes the date the title when there is no usable name, whatever the voice", () => {
    expect(greetingFor("", new Date(2026, 9, 22, 8), WHIMBREL.skin.voice, LOCALE)).toEqual({ kicker: null, title: "Thursday, October 22" });
    expect(greetingFor("me@acme.test", new Date(2026, 9, 22, 8), WHIMBREL.skin.voice, LOCALE).kicker).toBeNull();
  });
});

describe("the voice in the user's language", () => {
  const skin = (voice: Record<string, string>, voiceLocalized?: Record<string, Record<string, string>>) => ({
    ...WHIMBREL.skin,
    voice,
    voiceLocalized,
  });

  it("uses the skin's own wording for the exact language, then the base language, then its English", () => {
    const own = skin({ upNextHeader: "Coming up" }, { "fr-CA": { upNextHeader: "À venir (CA)" }, fr: { upNextHeader: "À venir" } });
    expect(localizedVoice(own, "fr-CA").upNextHeader).toBe("À venir (CA)");
    expect(localizedVoice(own, "fr-FR").upNextHeader).toBe("À venir");
    expect(localizedVoice(own, "de").upNextHeader).toBe("Coming up");
  });

  it("reads Whimbrel's own translation where a skin kept Whimbrel's English", () => {
    const whimbrelEs = WHIMBREL.skin.voiceLocalized?.es?.greetingMorning;
    expect(whimbrelEs).toBeTruthy();
    expect(localizedVoice(skin({}), "es-US").greetingMorning).toBe(whimbrelEs);
    // A skin's own English wording is its voice: it doesn't become Whimbrel's Spanish.
    expect(localizedVoice(skin({ greetingMorning: "Morning, {name}" }), "es").greetingMorning).toBe("Morning, {name}");
  });

  it("is Whimbrel's English in English", () => {
    expect(localizedVoice(WHIMBREL.skin, "en-US")).toEqual({ ...WHIMBREL.skin.voice });
  });
});

describe("imagery", () => {
  const manifest = (imagery: { asset: string; placement: string; textFree: boolean }[]) => ({ decoration: { imagery } });

  it("drops the background placement: an old file loads, and nothing is drawn for it", () => {
    expect(isDrawn({ placement: "background" })).toBe(false);
    expect(isDrawn({ placement: "header" })).toBe(true);
    expect(isDrawn({ placement: "badge" })).toBe(true);
    const kept = imageryOf(manifest([{ asset: "a.png", placement: "background", textFree: false }, { asset: "b.png", placement: "header", textFree: true }, { asset: "c.png", placement: "badge", textFree: false }]));
    expect(kept.map((i) => i.asset)).toEqual(["b.png", "c.png"]);
  });

  it("puts a scrim on a header image unless it is text-free, and none on a badge", () => {
    expect(hasScrim({ placement: "header", textFree: false })).toBe(true);
    expect(hasScrim({ placement: "header", textFree: true })).toBe(false);
    expect(hasScrim({ placement: "badge", textFree: false })).toBe(false);
  });

  it("covers a header image fully until the shell has measured its scrim", () => {
    const [unmeasured] = imageryOf(manifest([{ asset: "a.png", placement: "header", textFree: false }]));
    expect(unmeasured.scrim).toBe(1);
    const [measured] = imageryOf(manifest([{ asset: "a.png", placement: "header", textFree: false }]), { "a.png": 0.42 });
    expect(measured.scrim).toBe(0.42);
  });

  it("draws a text-free header and a badge as they are, whatever was measured", () => {
    const [free, badge] = imageryOf(manifest([{ asset: "a.png", placement: "header", textFree: true }, { asset: "b.png", placement: "badge", textFree: false }]), { "a.png": 0.9, "b.png": 0.9 });
    expect(free.scrim).toBe(0);
    expect(badge.scrim).toBe(0);
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
