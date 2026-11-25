import { describe, expect, it } from "vitest";
import type { CaptureSettings, CaptureStatus, DeleteEverythingPreview } from "../../../../../generated/types";
import { memoryWords, popupMemory } from "../../../../../popup/popupModel";
import { BANNED_PHRASES, deleteIntro, excludeCandidates, memoryDetailLine, memoryRow, ocrLine, redesignDeleteLines } from "./memoryTopModel";

// The same fixtures captureSummary.test.ts uses.
const base: CaptureStatus = { enabled: true, running: true, stored: 0, unchanged: 0, excluded: 0, ocrRuns: 0 };
const now = 1_757_520_000_000;

describe("the Memory row says what the tray popup says", () => {
  const tomorrow = now + 30 * 3_600_000;
  const cases: { name: string; status: CaptureStatus; noLibrary?: boolean }[] = [
    { name: "on", status: base },
    { name: "off", status: { ...base, enabled: false } },
    { name: "off with a stale deadline", status: { ...base, enabled: false, resumeAtUnixMs: now + 600_000 } },
    { name: "timed pause today", status: { ...base, resumeAtUnixMs: now + 600_000 } },
    { name: "timed pause until tomorrow", status: { ...base, resumeAtUnixMs: tomorrow } },
    { name: "paused by the app (locked)", status: { ...base, paused: "the screen is locked" } },
    { name: "paused by the app (display off)", status: { ...base, paused: "the display is off" } },
    { name: "paused by the app (sleep)", status: { ...base, paused: "the PC is going to sleep" } },
    { name: "paused by the app (idle)", status: { ...base, paused: "no input for 5 minutes" } },
    { name: "stopped, window text", status: { ...base, running: false, lastError: "UI Automation: failed" } },
    { name: "stopped, unknown", status: { ...base, running: false } },
  ];
  for (const c of cases) {
    it(`${c.name}: the words are the popup's, and no raw shell text shows`, () => {
      const popup = popupMemory(c.status, now);
      const row = memoryRow(c.status, now, "en-US");
      expect(row.memory).toEqual(popup);
      expect(row.words).toBe(memoryWords(popup!, now, "en-US"));
      const shown = [row.words, row.reason, row.pauseReason, memoryDetailLine(c.status, now)].filter(Boolean).join(" | ");
      expect(shown).not.toMatch(/Not running|UI Automation|lastError|Paused:|capture is/i);
    });
  }

  it("the words themselves", () => {
    expect(memoryRow(base, now, "en-US").words).toBe("Memory on");
    expect(memoryRow({ ...base, enabled: false }, now, "en-US").words).toBe("Memory off");
    expect(memoryRow({ ...base, resumeAtUnixMs: now + 600_000 }, now, "en-US").words).toMatch(/^Memory paused until .+$/);
    expect(memoryRow({ ...base, resumeAtUnixMs: tomorrow }, now, "en-US").words).toMatch(/^Memory paused until tomorrow at .+$/);
    expect(memoryRow({ ...base, paused: "the screen is locked" }, now, "en-US").words).toBe("Memory paused");
    expect(memoryRow({ ...base, running: false }, now, "en-US").words).toBe("Memory stopped");
  });

  it("Pause is offered only while on; Resume now only during a timed pause", () => {
    expect(memoryRow(base, now, "en-US").pause).toBe("choices");
    expect(memoryRow({ ...base, resumeAtUnixMs: now + 1000 }, now, "en-US").pause).toBe("resume");
    for (const s of [{ ...base, enabled: false }, { ...base, paused: "the screen is locked" }, { ...base, running: false }]) {
      const row = memoryRow(s, now, "en-US");
      expect(row.pause).toBe("none");
      expect(row.pauseReason).toBeTruthy();
    }
  });

  it("a stopped Memory gives the plain mapped reason and turns the switch off, with the reason", () => {
    const window = memoryRow({ ...base, running: false, lastError: "UI Automation: failed" }, now, "en-US");
    expect(window.reason).toBe("Windows would not let Whimbrel read window text.");
    // The switch stays usable for turning Memory off; only turning it on is pointless, and the row says what it does.
    expect(window.switchDisabled).toBe(false);
    expect(window.switchHint).toBe("Turning it off means Memory will not start again until you turn it on.");
    const other = memoryRow({ ...base, running: false, lastError: "os error 5" }, now, "en-US");
    expect(other.reason).toBe("Whimbrel could not start memory.");
    expect(other.reason).not.toContain("os error");
  });

  it("with no library the switch and Pause are off, with the reason", () => {
    const row = memoryRow(base, now, "en-US", true);
    expect(row.words).toBe("Memory off");
    expect(row.switchDisabled).toBe(true);
    expect(row.pause).toBe("none");
    expect(row.reason).toMatch(/could not open your library/);
    expect(row.pauseReason).toMatch(/could not open your library/);
  });

  it("the detail line is the old status line, only while Memory is on", () => {
    expect(memoryDetailLine(base, now)).toBe("Capturing; nothing stored yet");
    expect(memoryDetailLine({ ...base, lastStoredAtMs: now - 42_000, lastApp: "msedge.exe" }, now)).toBe("Capturing · last from msedge.exe 42s ago");
    expect(memoryDetailLine({ ...base, paused: "the screen is locked" }, now)).toBeNull();
    expect(memoryDetailLine({ ...base, enabled: false }, now)).toBeNull();
    expect(memoryDetailLine({ ...base, running: false }, now)).toBeNull();
    expect(memoryDetailLine({ ...base, resumeAtUnixMs: now + 1000 }, now)).toBeNull();
  });
});

describe("the exclude chooser", () => {
  const settings: Pick<CaptureSettings, "excluded" | "runningApps"> = {
    excluded: ["Chat.EXE"],
    runningApps: [
      { appId: "chat.exe", exe: "Chat.exe", title: "Chat" },
      { appId: "code.exe", exe: "code.exe", title: "x".repeat(60) },
    ],
  };
  it("never offers an app already excluded (case ignored) and clips long titles at 50", () => {
    const options = excludeCandidates(settings);
    expect(options.map((o) => o.value)).toEqual(["code.exe"]);
    expect(options[0].label).toBe(`code.exe: ${"x".repeat(50)}…`);
  });
});

describe("text recognition", () => {
  it("lists the languages, or says what is missing and what is installed", () => {
    expect(ocrLine({ covered: true, available: ["en-US", "de-DE"], profile: ["en-US"] })).toEqual({ covered: true, text: "OCR languages on this PC: en-US, de-DE." });
    const missing = ocrLine({ covered: false, available: ["en-US"], profile: ["fr-FR"] });
    expect(missing.covered).toBe(false);
    expect(missing.text).toContain("no OCR language pack for fr-FR");
    expect(missing.text).toContain("Installed: en-US.");
    expect(ocrLine({ covered: false, available: [], profile: [] }).text).toContain("for your display language");
  });
});

describe("Delete everything wording", () => {
  const preview: DeleteEverythingPreview = {
    dataBytes: 2_500_000_000,
    dataFolder: "C:\\Users\\sample\\AppData\\Local\\Whimbrel",
    snapshotsBytes: 1_000_000,
    snapshotsFolder: "C:\\Users\\sample\\Pictures\\Slides",
    storeOpen: true,
    syncOn: true,
    syncSignedIn: true,
    googleAccounts: ["sample@example.test"],
    endsMeeting: "A meeting is being recorded.",
    snapshotsRefused: "The slide folder is not Whimbrel's.",
  };
  const variants: DeleteEverythingPreview[] = [
    preview,
    { ...preview, googleAccounts: null },
    { ...preview, googleAccounts: null, storeOpen: false },
    { ...preview, googleAccounts: [], syncSignedIn: false, snapshotsFolder: null, endsMeeting: null, snapshotsRefused: null },
  ];
  it("says library and Memory, never the banned names, in every variant", () => {
    for (const p of variants) {
      for (const alsoAccount of [false, true]) {
        const text = [...redesignDeleteLines(p, alsoAccount), deleteIntro()].join("\n");
        for (const banned of BANNED_PHRASES) expect(text.toLowerCase()).not.toContain(banned.toLowerCase());
      }
    }
    const lines = redesignDeleteLines(preview);
    expect(lines[0]).toContain("your library");
    expect(lines).toContain("Memory is turned off now and comes back on after the restart if it is on in Settings.");
  });
  it("keeps the facts: sizes, the shell's own lines, the sync-account line and the sign-in that stays", () => {
    const lines = redesignDeleteLines(preview);
    expect(lines[0]).toContain("2.5 GB");
    expect(lines).toContain("A meeting is being recorded.");
    expect(lines).toContain("The slide folder is not Whimbrel's.");
    expect(lines.at(-1)).toContain("its sync sign-in");
    const gone = redesignDeleteLines(preview, true);
    expect(gone).toContain("The sync key goes with the account.");
    expect(gone.at(-1)).not.toContain("its sync sign-in");
    expect(redesignDeleteLines({ ...preview, syncOn: false }, true)).not.toContain("The sync key goes with the account.");
  });
});
