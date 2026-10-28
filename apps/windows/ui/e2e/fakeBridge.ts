// The Tauri side of the page, faked: the built UI is served on its own and every `invoke` answers from fixtures.
// The skins are the bundled ones, read from the repo, plus the worst-case manifest from the shared corpus.
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import type { Page } from "@playwright/test";
import { PNG } from "pngjs";

const ROOT = fileURLToPath(new URL("../../../../", import.meta.url));
const readJson = (path: string) => JSON.parse(readFileSync(`${ROOT}${path}`, "utf8"));

export interface Palette {
  id: string;
  name: string;
  author: string;
  light?: Record<string, string>;
  dark?: Record<string, string>;
}
export interface Manifest {
  id: string;
  name: string;
  palettes: string[];
  today: { archetype: string; options: Record<string, string> };
  decoration: { imagery: { asset: string; placement: string; textFree: boolean }[] } & Record<string, unknown>;
  [key: string]: unknown;
}
export interface SkinDto {
  skin: Manifest;
  palettes: Palette[];
  builtIn: boolean;
  imageScrims?: Record<string, number>;
}

const GENERATED = `${ROOT}apps/windows/ui/e2e/.generated/skins.json`;

/** The shipped skins exactly as the shell's `skins_list` answers them, palettes already repaired for each layout
 *  (src-tauri/src/commands/skins.rs, test `dump_bundled_skins_for_the_matrix`), so the page is judged on what a
 *  user would see. */
export const BUILT_IN: SkinDto[] = (() => {
  try {
    return JSON.parse(readFileSync(GENERATED, "utf8")) as SkinDto[];
  } catch {
    throw new Error(`${GENERATED} is missing: run \`npm run matrix:skins\` first`);
  }
})();

export const ARCHETYPES = ["standard", "terminal", "glass", "field", "almanac", "reference"] as const;

/** Imagery the worst-case skin carries when asked: all three placements, the background not text-free. */
/** The measured scrim of the background picture (Task 10 computes it; the picture below is a mid-tone, so most of it hides). */
const SCRIM = 0.95;

const IMAGERY = [
  { asset: "header.png", placement: "header", textFree: true },
  { asset: "badge.png", placement: "badge", textFree: true },
  { asset: "background.png", placement: "background", textFree: false },
];

/** The spec's worst-case skin (skins/testdata/worst-case.json) on one Today layout, with or without images. It
 *  wears the palettes of the shipped skin that draws that layout, which the shell has already repaired for it. */
export function worstCase(archetype: string, imagery: boolean): SkinDto {
  const base: Manifest = readJson("skins/testdata/worst-case.json").expected;
  const home = BUILT_IN.find((d) => d.skin.today.archetype === archetype);
  if (!home) throw new Error(`no shipped skin draws ${archetype}`);
  const skin: Manifest = { ...base, today: { archetype, options: {} }, decoration: { ...base.decoration, imagery: imagery ? IMAGERY : [] } };
  // A scrim as the shell will measure it: the page colour covers the picture enough for text to read.
  return { skin, builtIn: false, palettes: home.palettes, ...(imagery ? { imageScrims: { "background.png": SCRIM } } : {}) };
}

/** A picture with real tonal range, so a scrim has something to hide. */
function picture(): string {
  const png = new PNG({ width: 64, height: 64 });
  for (let y = 0; y < 64; y++)
    for (let x = 0; x < 64; x++) {
      const i = (y * 64 + x) * 4;
      const v = (x + y) % 16 < 8 ? 100 : 160;
      png.data[i] = v;
      png.data[i + 1] = 260 - v;
      png.data[i + 2] = 100 + ((x * 2) % 60);
      png.data[i + 3] = 255;
    }
  return PNG.sync.write(png).toString("base64");
}

const MIN = 60_000;
const LONG_WORD = "Supercalifragilisticexpialidocious-renegotiation";

/** What the app asks for, answered with a busy day: long titles (one with no break to wrap at), many attendees,
 *  recorded and unrecorded events, earlier meetings, open loops and a recap. */
function fixtures(skins: SkinDto[], now: number) {
  const dev = { deviceId: "PC", label: "A-Very-Long-Computer-Name", thisDevice: true };
  const meeting = (id: number, hours: number, title: string) => ({ id, device: dev, hasSummary: true, status: "done", title, trigger: "calendar", tsUnixMs: now - hours * 3_600_000 });
  const long = "Quarterly planning and renegotiation of the enterprise agreement with every stakeholder involved in it";
  return {
    skins_list: skins,
    meeting_most_relevant: null,
    // The greeting's {name} is the first word: the longest the corpus allows.
    profile_get: { name: "W".repeat(30) },
    today_overview: {
      day: "2026-10-22",
      devices: [
        { device: dev, captures: 123456, lastTsUnixMs: now - 5 * MIN },
        { device: { deviceId: "MB", label: "MacBook", thisDevice: false }, captures: 0, lastTsUnixMs: null },
      ],
      openLoops: [
        { id: 1, subject: `Re: ${long}`, status: "waiting_on_me", lastMessageFrom: "ann@acme.test", lastActivityTsUnixMs: now - 3_600_000 },
        { id: 2, subject: LONG_WORD, status: "waiting_on_them", lastMessageFrom: "bob@x.test", lastActivityTsUnixMs: now - 7_200_000 },
      ],
      recap: { day: "2026-10-22", summary: `${long}. ${long}. ${LONG_WORD}.`, openLoops: [], cached: true, emptyDay: false },
      briefRecap: null,
      latestMeeting: null,
    },
    today_schedule: {
      events: [
        { eventId: "e1", title: long, startUnixMs: now + 9 * MIN, endUnixMs: now + 39 * MIN, attendees: ["ann@acme.test", "bob@acme.test", "cy@acme.test", "dee@acme.test"], recordedMeetingId: null },
        { eventId: "e2", title: LONG_WORD, startUnixMs: now + 90 * MIN, endUnixMs: now + 120 * MIN, attendees: [], recordedMeetingId: null },
        { eventId: "e3", title: "Design review", startUnixMs: now + 180 * MIN, endUnixMs: now + 240 * MIN, attendees: ["d@x.test"], recordedMeetingId: 7 },
        { eventId: "e4", title: "Board prep", startUnixMs: now + 300 * MIN, endUnixMs: now + 330 * MIN, attendees: ["ceo@x.test"], recordedMeetingId: null },
        { eventId: "e5", title: "Late review", startUnixMs: now + 400 * MIN, endUnixMs: now + 430 * MIN, attendees: [], recordedMeetingId: null },
      ],
      earlierToday: [meeting(11, 4, "Standup"), meeting(12, 2, long)],
      recording: false,
    },
    next_meeting_prep: null,
    meeting_list: [meeting(1, 26, long), meeting(2, 50, LONG_WORD), meeting(3, 75, "Customer call")],
    ai_settings: { routing: { preferredProvider: "ollama", localOnly: false, ollamaModel: "llama3", anthropicModel: "", openaiModel: "", googleModel: "" }, keys: [] },
    approvals_count: 0,
  };
}

/** Make `page` answer the app's Tauri calls, and choose the skin, palette and appearance it starts in. */
export async function installBridge(page: Page, opts: { skins: SkinDto[]; skinId: string; paletteId: string; mode: "light" | "dark" }): Promise<void> {
  const data = fixtures(opts.skins, Date.now());
  const image = picture();
  await page.addInitScript(
    ({ data, image, skinId, paletteId, mode }) => {
      try {
        localStorage.setItem("skinId", skinId);
        localStorage.setItem("paletteId", paletteId);
        localStorage.setItem("appearance", mode);
      } catch {
        /* the page still draws with its defaults */
      }
      const bytes = () => Uint8Array.from(atob(image), (c) => c.charCodeAt(0)).buffer;
      (window as unknown as { __TAURI_INTERNALS__: unknown }).__TAURI_INTERNALS__ = {
        metadata: { currentWindow: { label: "main" }, currentWebview: { label: "main", windowLabel: "main" } },
        transformCallback: () => 0,
        invoke: async (cmd: string) => (cmd === "skin_image" ? bytes() : cmd in data ? (data as Record<string, unknown>)[cmd] : null),
        convertFileSrc: (s: string) => s,
      };
    },
    { data, image, skinId: opts.skinId, paletteId: opts.paletteId, mode: opts.mode },
  );
}
