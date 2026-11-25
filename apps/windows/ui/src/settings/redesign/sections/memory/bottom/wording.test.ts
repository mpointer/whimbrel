import { describe, expect, it } from "vitest";
import { setLanguage } from "../../../../../l10n";
import type { CardBadge, FreeUpOptions, NoteRow, RetentionSettings, SpeedCheckView, StorageView } from "../../../../../generated/types";
import { bannedIn } from "./libraryWording";
import { deleteNowCopy, lastRunLine, periodConfirmCopy } from "./retentionWording";
import { RETENTION_KEYS } from "./retentionKeys";
import { compactFrees, compactLabel, compactResult, compressResult, redesignedFreeUpRows, storageSummary } from "./storageWording";
import { fastSearchHelp, libraryBadge, libraryBody, libraryEncryptWords, libraryEncryptMessage, libraryNote, libraryNoteActionMessage, librarySpeedLines, unencryptedCopyNote } from "./encryptionWording";
import { keptAsideConfirmBody, keptAsideExplainer, keptAsideSummary } from "./foreignFilesWording";

const none = (lines: string[]) => expect(lines.flatMap(bannedIn)).toEqual([]);

describe("the banned phrases", () => {
  it("catch the old name", () => {
    expect(bannedIn("the memory database")).not.toEqual([]);
    expect(bannedIn("Your library")).toEqual([]);
  });
});

describe("Keeping data words", () => {
  const settings: RetentionSettings = { captureDays: 30, meetingDays: 365, conversationDays: 90 };
  it("name what will be deleted and never say Chats", () => {
    for (const key of RETENTION_KEYS) {
      for (const from of [null, 90]) {
        for (const ack of [true, false]) {
          const c = periodConfirmCopy(key, 14, from, ack);
          none([c.title, c.body, c.confirmLabel]);
          expect(c.title).toMatch(/older than 14 days\?$/);
          expect(c.body).toContain("older than 14 days will be deleted once a day");
          expect(c.body).toContain("This can't be undone.");
        }
      }
    }
    expect(periodConfirmCopy("conversationDays", 14, null, true).title).toBe("Delete conversations older than 14 days?");
    expect(periodConfirmCopy("captureDays", 14, 90, false).body).toContain("waiting for the notice");
  });
  it("Delete old data now lists each period, with counts when known", () => {
    const withCounts = deleteNowCopy(settings, { captures: 5, meetings: 2, conversations: 1, orphans: 3, orphanBytes: 2_000_000 });
    expect(withCounts.lines).toEqual([
      "Screen text older than 30 days: 5 items",
      "Meetings and recordings older than 365 days: 2 meetings",
      "Conversations older than 90 days: 1 conversation",
      "Recordings no meeting uses any more: 3 files (2.0 MB)",
    ]);
    none([withCounts.title, withCounts.body, withCounts.confirmLabel, ...withCounts.lines]);
    expect(deleteNowCopy(settings, null).lines).toEqual(["Screen text older than 30 days", "Meetings and recordings older than 365 days", "Conversations older than 90 days"]);
    const forever = deleteNowCopy({ captureDays: null, meetingDays: null, conversationDays: null }, null).body;
    expect(forever).toContain("Keep forever");
    // It still removes recordings no meeting uses, and still starts the daily clean-up: it must not say nothing is deleted.
    expect(forever).toContain("removes recordings that no meeting uses any more, whatever the periods say");
    expect(forever).toContain("starts the daily clean-up");
    expect(forever).not.toMatch(/nothing is deleted|no .* are deleted\.$/);
    const normal = deleteNowCopy(settings, null).body;
    expect(normal).toContain("It can't be undone.");
    expect(normal).toContain("removes recordings that no meeting uses any more");
    expect(normal).toContain("starts the daily clean-up");
  });
  it("the last run line says conversations", () => {
    const line = lastRunLine({ capturesDeleted: 1, meetingsDeleted: 2, conversationsDeleted: 3, orphansDeleted: 4, finishedUnixMs: 0 }, "3:04 PM");
    expect(line).toBe("Last run 3:04 PM: 1 screen captures, 2 meetings, 3 conversations, 4 leftover recordings deleted.");
    none([line]);
  });
});

describe("Storage words", () => {
  const view = {
    days: [30, 90],
    dbFreeBytes: 0,
    defaultDays: 90,
    footprint: { total: 5_000_000_000, database: 1_000_000_000, recordings: 3_000_000_000, models: 500_000_000, other: 500_000_000 },
    lowDisks: [{ volume: "C:", freeBytes: 4_000_000_000 }],
  } as unknown as StorageView;
  const options = { days: 90, compactBytes: 1000, compressBytes: 2000, recordingsBytes: 3000, capturesCount: 4, capturesBytes: 5000 } as unknown as FreeUpOptions;
  it("has no banned phrase and keeps the two delete questions", () => {
    const s = storageSummary(view);
    none([s.total, s.parts, ...s.lowDisks]);
    const rows = redesignedFreeUpRows(options);
    none(rows.flatMap((r) => [r.detail, r.button, r.confirm ?? ""]));
    expect(rows.find((r) => r.kind === "recordings")?.confirm).toBe("Delete the audio of meetings older than 90 days? This can't be undone.");
    expect(rows.find((r) => r.kind === "captures")?.confirm).toBe("Delete screen captures older than 90 days from this PC and your other devices? This can't be undone.");
    expect(rows.find((r) => r.kind === "compact")?.confirm).toBeNull();
    none([compactResult({ ran: false, reason: "recording" } as never), compressResult({ compressed: 1, failed: 0 } as never)]);
  });
});

describe("Library encryption words", () => {
  const badges: CardBadge[] = ["encrypted", "notEncrypted", "needsAttention"];
  const notes = [
    { kind: "keyStoreUnavailable" },
    { kind: "failedBefore" },
    { kind: "migrationSkipped", stateUnreadable: true },
    { kind: "migrationSkipped", skip: { reason: "open_elsewhere" } },
    { kind: "migrationSkipped", skip: { reason: "recording" } },
    { kind: "migrationSkipped", skip: { reason: "low_disk" } },
    { kind: "migrationFailed" },
    { kind: "encryptionAvailable" },
    { kind: "plainBackupKept" },
    { kind: "restoredCopy", movedAside: 1, warning: "stateNotMovedAside" },
    { kind: "pendingKeyNotPromoted" },
    { kind: "pendingKeyNotDeleted" },
    { kind: "recoveredKeyNotStored", failure: "noPreviousSlot" },
    { kind: "recoveredKeyNotStored", failure: "other" },
    { kind: "recoveredKeyStoreUnverified" },
    { kind: "memoryDatabaseUnavailable" },
  ] as unknown as NoteRow["note"][];
  it("says your library in the badge, body, notes and messages", () => {
    for (const b of badges) none([libraryBadge(b).label, ...libraryBody(b)]);
    expect(libraryBody("needsAttention")).toEqual(["Your library could not be opened."]);
    expect(libraryBody("encrypted")[0]).toMatch(/^Your library on this PC is encrypted/);
    for (const note of notes) {
      const lines = libraryNote({ note, action: null } as unknown as NoteRow);
      none(lines.texts);
    }
    for (const action of ["retryAfterFailedBefore", "resetEncryptionStatus", "deletePlainBackup"] as const) {
      for (const result of ["done", "failed"] as const) none([libraryNoteActionMessage(action as never, result as never)?.text ?? ""]);
    }
    none([unencryptedCopyNote(), fastSearchHelp()]);
  });
  it("the encrypt wizard, its refusals and its outcomes say your library, and none names the database", () => {
    const w = libraryEncryptWords();
    none([w.atNextLaunch, w.continueTitle, w.startButton, w.setup.intro, w.setup.sameDisk, w.setup.wordsWarning, w.setup.footer, w.setup.printNotice, w.setup.printFooter]);
    for (const line of [w.setup.printNotice, w.setup.printFooter]) expect(line).not.toMatch(/database/i);
    expect(w.setup.printFooter).toContain("your library");
    expect(w.atNextLaunch).toBe("Whimbrel will finish encrypting your library the next time it starts.");
    expect(w.continueTitle).toBe("Encrypt your library");
    for (const r of ["keychainUnavailable", "twoKeysFound", "recording", "lowDisk", "openElsewhere", "failedBefore", "other"] as const) {
      none([w.refusal(r)]);
      expect(w.refusal(r)).not.toMatch(/database/i);
    }
    expect(w.refusal("lowDisk")).toContain("your library");
    expect(w.refusal("openElsewhere")).toContain("your library");
    for (const o of ["atNextLaunch", "twoKeysFound", "failed", "notReady"] as const) {
      const m = libraryEncryptMessage(o as never);
      if (m) expect(m).not.toMatch(/database/i);
    }
    for (const lines of [libraryBody("encrypted"), libraryBody("notEncrypted"), libraryBody("needsAttention")]) expect(lines.join(" ")).not.toMatch(/database/i);
    for (const note of notes) expect(libraryNote({ note, action: null } as unknown as NoteRow).texts.join(" ")).not.toMatch(/database/i);
    expect(fastSearchHelp()).not.toMatch(/database/i);
  });
  it("the wizard's words follow the language: they are computed when asked, not kept", () => {
    const before = libraryEncryptWords().setup.printFooter;
    setLanguage("es");
    try {
      expect(libraryEncryptWords().setup.printFooter).not.toBe(before);
    } finally {
      setLanguage("en");
    }
  });
  it("the speed report says Your library", () => {
    const report = { encrypted: true, openMs: 3, tooLittleData: false, keywordP50Ms: 1, keywordP95Ms: 2, semanticP50Ms: 1, semanticP95Ms: 2, semanticUnavailable: false, cpuArch: "x64", ramGib: 16, os: "Windows" } as unknown as SpeedCheckView;
    const lines = librarySpeedLines(report, "en");
    expect(lines[0]).toBe("Your library: encrypted");
    none(lines);
    none(librarySpeedLines({ ...report, encrypted: false }, "en"));
  });
});

describe("Files kept aside words", () => {
  it("says beside your library and gives the count and size", () => {
    const v = { count: 3, bytes: 12_000_000 };
    none([keptAsideExplainer(), keptAsideSummary(v), keptAsideConfirmBody(v)]);
    expect(keptAsideExplainer()).toContain("beside your library");
    expect(keptAsideConfirmBody(v)).toContain("3 files");
    expect(keptAsideConfirmBody(v)).toContain("12.0 MB");
  });
});

describe("Storage words name the library", () => {
  const view = {
    days: [30],
    dbFreeBytes: 1_100_000_000,
    defaultDays: 30,
    footprint: { total: 5_000_000_000, database: 1_100_000_000, recordings: 3_000_000_000, models: 500_000_000, other: 400_000_000 },
    lowDisks: [],
  } as unknown as StorageView;
  it("says Your library for the file size and Compact your library for the button", () => {
    expect(storageSummary(view).parts).toBe("Your library 1.1 GB · Recordings 3.0 GB · Speech models 500.0 MB · Other 400.0 MB");
    expect(compactLabel()).toBe("Compact your library");
    expect(compactFrees(1_100_000_000)).toBe("Frees 1.1 GB inside your library file.");
    const compact = redesignedFreeUpRows({ days: 30, compactBytes: 1_100_000_000 } as unknown as FreeUpOptions).find((r) => r.kind === "compact");
    expect(compact?.detail).toBe("Frees 1.1 GB inside your library file.");
  });
});
