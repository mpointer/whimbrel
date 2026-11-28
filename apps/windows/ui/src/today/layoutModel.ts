// The skin's voice, the Home heading, the recap and the skin's images, decided once (the Mac's `TodayContent` and
// `SkinVoice`). Pure: nothing here touches the DOM.

import type { MeetingRow, TodayOverview, TodayRecap } from "../generated/types";
import { WHIMBREL, type SkinManifest } from "../skin";
import { greeting } from "./todayModel";

export type ArchetypeId = "standard" | "terminal" | "glass" | "field" | "almanac" | "reference";

export const ARCHETYPE_IDS: readonly ArchetypeId[] = ["standard", "terminal", "glass", "field", "almanac", "reference"];

/** An archetype id from a manifest, or Standard for one this build does not know. */
export const archetypeOf = (id: string | undefined): ArchetypeId => ARCHETYPE_IDS.find((a) => a === id) ?? "standard";

// --- voice ------------------------------------------------------------------------------------

export type VoiceKey =
  | "greetingMorning"
  | "greetingAfternoon"
  | "greetingEvening"
  | "upNextHeader"
  | "upcomingHeader"
  | "earlierHeader"
  | "recapHeader"
  | "emptyDay"
  | "emptyEarlier"
  | "askPlaceholder"
  | "tagline";

const VOICE_KEYS: readonly VoiceKey[] = [
  "greetingMorning",
  "greetingAfternoon",
  "greetingEvening",
  "upNextHeader",
  "upcomingHeader",
  "earlierHeader",
  "recapHeader",
  "emptyDay",
  "emptyEarlier",
  "askPlaceholder",
  "tagline",
];

/**
 * The skin's voice in `lang` (the Mac's `SkinVoice.template`): the skin's own wording for the exact
 * language, then its base language, then its English. English that is just Whimbrel's own default
 * reads as Whimbrel's wording in `lang`, so a skin that never changed a line follows the language too.
 */
export function localizedVoice(skin: SkinManifest, lang: string): Record<string, string> {
  const base = lang.split("-")[0];
  const pick = (s: SkinManifest, key: VoiceKey) => s.voiceLocalized?.[lang]?.[key] ?? s.voiceLocalized?.[base]?.[key];
  const out: Record<string, string> = {};
  for (const key of VOICE_KEYS) {
    const own = pick(skin, key);
    const english = skin.voice[key] ?? WHIMBREL.skin.voice[key];
    const text = own ?? (english === WHIMBREL.skin.voice[key] ? (pick(WHIMBREL.skin, key) ?? english) : english);
    if (text !== undefined) out[key] = text;
  }
  return out;
}

/** A voice string with `{name}` filled in. A key the skin leaves out reads as Whimbrel's. */
export function voiceText(voice: Record<string, string>, key: VoiceKey, name = ""): string {
  const text = voice[key] ?? WHIMBREL.skin.voice[key] ?? "";
  return text.split("{name}").join(name);
}

/** Today's heading in the skin's voice: the date over "Good morning, <first name>" worded by the skin. With
 *  no usable name the date is the title, as in `greeting`. The date is written for `locale`. */
export function greetingFor(name: string, now: Date, voice: Record<string, string>, locale: string): { kicker: string | null; title: string } {
  const plain = greeting(name, now, locale);
  if (plain.kicker === null) return plain;
  const hour = now.getHours();
  const key = hour < 12 ? "greetingMorning" : hour < 17 ? "greetingAfternoon" : "greetingEvening";
  return { kicker: plain.kicker, title: voiceText(voice, key, name.trim().split(/\s+/)[0]) };
}

// --- what Today knows -------------------------------------------------------------------------

/** The Daily Brief, the digest, or why neither is there. */
export type RecapModel =
  | { kind: "brief"; text: string; origin: string | null }
  | { kind: "digest"; text: string }
  | { kind: "failed"; message: string; retryEnabled: boolean }
  | { kind: "writing" };

/** The recap as Home shows it: the Daily Brief, else the digest, else why there is none. */
export function recapModel(overview: Pick<TodayOverview, "briefRecap">, recap: TodayRecap | null, writing: boolean, error: string | null): RecapModel | null {
  if (overview.briefRecap) return { kind: "brief", text: overview.briefRecap.summary, origin: overview.briefRecap.from?.label ?? null };
  if (recap) return { kind: "digest", text: recap.summary };
  if (error) return { kind: "failed", message: error, retryEnabled: !writing };
  return writing ? { kind: "writing" } : null;
}

/** How many meetings from earlier days Today lists. */
export const EARLIER_LIMIT = 5;

/** The meetings from before today (the local day), newest first, as many as Today lists. */
export function earlierMeetings(rows: MeetingRow[], now: number): MeetingRow[] {
  const dayStart = new Date(now).setHours(0, 0, 0, 0);
  return rows
    .filter((m) => m.tsUnixMs < dayStart)
    .sort((a, b) => b.tsUnixMs - a.tsUnixMs)
    .slice(0, EARLIER_LIMIT);
}

// --- imagery ----------------------------------------------------------------------------------

export type ImagePlacement = "header" | "background" | "badge";

export interface Imagery {
  asset: string;
  placement: ImagePlacement;
  textFree: boolean;
  /** How strongly the page colour covers a background so every text colour reads at 4.5:1. 1 hides it. */
  scrim: number;
}

/** The background placement is dropped in Thread: no part of Home is safe to put an image under. A skin that still
 *  has one loads as before and shows nothing there. */
export const isDrawn = (image: Pick<Imagery, "placement">): boolean => image.placement !== "background";

/** The placements that are drawn. `background` stays in the type because old skin files can name it. */
const PLACEMENTS: readonly string[] = ["header", "badge"];

/** Whether an image carries a scrim. A header image sits in the band against the title row, so it has one unless the skin
 *  marks it text-free; a badge sits beside the text and never under it. */
export const hasScrim = (image: Pick<Imagery, "placement" | "textFree">): boolean => image.placement === "header" && !image.textFree;

/** The images of a skin that would be drawn. Until the shell has measured a header image (a skin you made), its
 *  scrim is 1, which covers it: an image is never shown under text below 4.5:1. */
export function imageryOf(
  manifest: { decoration: { imagery: { asset: string; placement: string; textFree: boolean }[] } },
  scrims: Record<string, number> = {},
): Imagery[] {
  return manifest.decoration.imagery
    .filter((i) => PLACEMENTS.includes(i.placement))
    .map((i) => {
      const entry = { asset: i.asset, placement: i.placement as ImagePlacement, textFree: i.textFree };
      // Unmeasured is 1 (fully covered); an image with no scrim has 0 so it draws as it is.
      return { ...entry, scrim: hasScrim(entry) ? (scrims[i.asset] ?? 1) : 0 };
    })
    .filter(isDrawn);
}
