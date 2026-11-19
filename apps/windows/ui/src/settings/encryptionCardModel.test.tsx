import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";
import type { EncryptionCardView, NoteRow, SpeedCheckResult, SpeedCheckView } from "../generated/types";

// The card's sections call the shell on mount only through effects, which a static render does not
// run; the API module is replaced so nothing reaches Tauri.
vi.mock("../api", () => {
  const never = () => new Promise(() => {});
  return Object.fromEntries(
    [
      "encryptionCard", "encryptionEncrypt", "encryptionNoteAction", "encryptionPrecheckForSetup", "restartApp", "setFastSearch", "setupBegin",
      "setupContinue", "setupReusesKey", "setupSavedTo", "setupSaveFile", "setupStartOver", "setupVerify", "speedCheckCancel", "speedCheckRun", "speedCheckSave",
    ].map((name) => [name, never]),
  );
});
const { EncryptionCardBody, NotesList } = await import("./EncryptionCard");
const { badgeLabel, cardBody, EncryptFlow, noteActionMessage, noteLines, refusalMessage, SpeedCheck, speedLines } = await import("./encryptionCardModel");

const card = (over: Partial<EncryptionCardView> = {}): EncryptionCardView => ({
  badge: "notEncrypted",
  plainBackupPresent: false,
  pendingNextLaunch: false,
  notes: [],
  cleanupRunning: false,
  fastSearch: { available: false, enabled: false },
  showsStoreTools: true,
  ...over,
});
const noop = () => {};

describe("the badge and body", () => {
  it("come from how the launch opened the database", () => {
    expect(badgeLabel("encrypted")).toBe("Encrypted");
    expect(badgeLabel("notEncrypted")).toBe("Not encrypted");
    expect(badgeLabel("needsAttention")).toBe("Needs attention");
    expect(cardBody("encrypted")[0]).toContain("is encrypted with a key kept in Windows Credential Manager");
    expect(cardBody("notEncrypted")[0]).toContain("is an ordinary file on this PC");
    expect(cardBody("needsAttention")).toEqual(["The memory database could not be opened."]);
  });

  it("in no-store mode nothing else is offered: no Encrypt, no Faster search, no speed check", () => {
    const html = renderToStaticMarkup(
      <EncryptionCardBody
        view={card({ badge: "needsAttention", showsStoreTools: false, fastSearch: { available: true, enabled: false }, notes: [{ note: { kind: "memoryDatabaseUnavailable" } }] })}
        reload={noop}
      />,
    );
    expect(html).toContain("Whimbrel is running without the memory database");
    expect(html).not.toContain("<button");
    expect(html).not.toContain("Faster search");
  });

  it("Encrypt only while plain; Restart only while the run waits for the next launch", () => {
    expect(renderToStaticMarkup(<EncryptionCardBody view={card()} reload={noop} />)).toContain("Encrypt the database…");
    expect(renderToStaticMarkup(<EncryptionCardBody view={card({ badge: "encrypted" })} reload={noop} />)).not.toContain("Encrypt the database…");
    const pending = renderToStaticMarkup(<EncryptionCardBody view={card({ pendingNextLaunch: true })} reload={noop} />);
    expect(pending).not.toContain("Encrypt the database…");
    expect(pending).toContain("Restart Whimbrel");
    expect(pending).toContain("the next time it starts");
    expect(renderToStaticMarkup(<EncryptionCardBody view={card()} reload={noop} />)).not.toContain("Restart Whimbrel");
  });

  it("the plain-backup notice follows the shell's flag (file present, note not showing)", () => {
    const notice = "keeps an unencrypted copy from before encryption until it has started normally three times";
    expect(renderToStaticMarkup(<EncryptionCardBody view={card({ badge: "encrypted", plainBackupPresent: true })} reload={noop} />)).toContain(notice);
    expect(renderToStaticMarkup(<EncryptionCardBody view={card({ badge: "encrypted", plainBackupPresent: false })} reload={noop} />)).not.toContain(notice);
  });

  it("Faster search only when available", () => {
    expect(renderToStaticMarkup(<EncryptionCardBody view={card({ fastSearch: { available: true, enabled: true } })} reload={noop} />)).toContain(
      "Faster search (uses more memory)",
    );
    expect(renderToStaticMarkup(<EncryptionCardBody view={card()} reload={noop} />)).not.toContain("Faster search");
  });
});

describe("the precheck and the encrypt step", () => {
  const flow = (over: Partial<ConstructorParameters<typeof EncryptFlow>[0]> = {}) => {
    const a = { precheck: vi.fn(async () => null), continueSetup: vi.fn(async () => true), encrypt: vi.fn(async () => "atNextLaunch" as const), ...over };
    return { f: new EncryptFlow(a), a };
  };

  it("a refusal stops before any setup call, with its message", async () => {
    for (const [refusal, text] of [
      ["keychainUnavailable", "can't read Credential Manager right now"],
      ["twoKeysFound", "two different encryption keys"],
      ["recording", "A meeting is being recorded."],
      ["lowDisk", "not enough free disk space"],
      ["openElsewhere", "Another program has the memory database open."],
      ["failedBefore", "Encryption could not be completed."],
      ["other", "Encryption did not start."],
    ] as const) {
      const { f, a } = flow({ precheck: vi.fn(async () => refusal) });
      await f.start();
      expect(f.getState()).toEqual({ kind: "idle", message: refusalMessage(refusal) });
      expect(refusalMessage(refusal)).toContain(text);
      expect(a.continueSetup).not.toHaveBeenCalled();
      expect(a.encrypt).not.toHaveBeenCalled();
    }
  });

  it("a pass opens the setup; Continue encrypts and waits for the next launch", async () => {
    const { f, a } = flow();
    await f.start();
    expect(f.getState().kind).toBe("settingUp");
    await f.finish();
    expect(a.continueSetup).toHaveBeenCalledOnce();
    expect(a.encrypt).toHaveBeenCalledOnce();
    expect(f.getState().kind).toBe("atNextLaunch");
  });

  it("encrypt never calls a migration command: its only shell calls are precheck, continue and encrypt", () => {
    const { a } = flow();
    expect(Object.keys(a).sort()).toEqual(["continueSetup", "encrypt", "precheck"]);
  });

  it("two keys refused at the encrypt step too", async () => {
    const { f } = flow({ encrypt: vi.fn(async () => "twoKeysFound" as const) });
    await f.start();
    await f.finish();
    expect(f.getState()).toEqual({ kind: "failed", message: refusalMessage("twoKeysFound") });
    f.dismiss();
    expect(f.getState()).toEqual({ kind: "idle", message: null });
  });

  it("an untested setup does not encrypt", async () => {
    const { f, a } = flow({ continueSetup: vi.fn(async () => false) });
    await f.start();
    await f.finish();
    expect(a.encrypt).not.toHaveBeenCalled();
    expect(f.getState().kind).toBe("failed");
  });

  it("a precheck answer that arrives after Cancel is ignored", async () => {
    let answer!: (r: null) => void;
    const { f } = flow({ precheck: () => new Promise((r) => (answer = r)) });
    const running = f.start();
    expect(f.getState().kind).toBe("checking");
    f.cancel();
    answer(null);
    await running;
    expect(f.getState()).toEqual({ kind: "idle", message: null });
  });
});

describe("the notes", () => {
  const rows: NoteRow[] = [
    { note: { kind: "keyStoreUnavailable" } },
    { note: { kind: "failedBefore", attempts: 3 }, action: "retryAfterFailedBefore" },
    { note: { kind: "migrationSkipped", stateUnreadable: true }, action: "resetEncryptionStatus" },
    { note: { kind: "migrationSkipped", stateUnreadable: false, skip: { reason: "recording" } } },
    { note: { kind: "migrationSkipped", stateUnreadable: false, skip: { reason: "low_disk", need_bytes: 2, free_bytes: 1 } } },
    { note: { kind: "migrationSkipped", stateUnreadable: false, skip: { reason: "free_space_unknown", message: "x" } } },
    { note: { kind: "migrationSkipped", stateUnreadable: false, skip: { reason: "open_elsewhere" } } },
    { note: { kind: "migrationSkipped", stateUnreadable: false, skip: { reason: "open_in_this_process" } } },
    { note: { kind: "migrationFailed" } },
    { note: { kind: "encryptionAvailable" } },
    { note: { kind: "plainBackupKept" }, action: "deletePlainBackup" },
    { note: { kind: "restoredCopy", movedAside: 2, warning: "stateNotMovedAside" } },
    { note: { kind: "pendingKeyNotPromoted", failure: "keychainUnavailable" } },
    { note: { kind: "pendingKeyNotDeleted", failure: "keychainUnavailable" } },
    { note: { kind: "recoveredKeyNotStored", failure: "noPreviousSlot" } },
    { note: { kind: "recoveredKeyNotStored", failure: "keychainUnavailable" } },
    { note: { kind: "recoveredKeyStoreUnverified" } },
    { note: { kind: "memoryDatabaseUnavailable" } },
  ];
  const expected = [
    "could not read Credential Manager when it started",
    "Encryption could not be completed. Your data is unchanged.",
    "cannot read its record of an earlier encryption attempt",
    "because a meeting was being recorded",
    "not enough free disk space",
    "not enough free disk space",
    "another program had the memory database open",
    "Encryption did not start at this launch.",
    "could not be completed at this launch",
    "already has an encryption key from an earlier attempt",
    "An unencrypted copy of the memory database from before encryption is still on this PC.",
    "put back the unencrypted copy",
    "could not finish saving its encryption key",
    "kept a spare encryption key",
    "as many old Whimbrel keys as Whimbrel keeps",
    "could not save it in Credential Manager",
    "could not confirm it was saved",
    "running without the memory database",
  ];

  it("each has its copy and only its own button", () => {
    rows.forEach((row, i) => {
      const lines = noteLines(row);
      expect(lines.texts.join(" ")).toContain(expected[i]);
      expect(lines.action?.action ?? null).toBe(row.action ?? null);
    });
    expect(noteLines(rows[1]).action?.label).toBe("Try again");
    expect(noteLines(rows[2]).action?.label).toBe("Reset encryption status");
    expect(noteLines(rows[10]).action?.label).toBe("Delete the unencrypted copy");
    expect(noteLines(rows[10]).texts[1]).toBe("Deleting a file does not guarantee its contents are gone from the disk.");
    expect(noteLines(rows[11]).texts).toHaveLength(3);
    expect(noteLines({ note: { kind: "restoredCopy", movedAside: 0 } }).texts).toHaveLength(1);
  });

  it("every button is disabled while the cleanup runs", () => {
    const withButtons = rows.filter((r) => r.action);
    const running = renderToStaticMarkup(<NotesList view={card({ notes: withButtons, cleanupRunning: true })} reload={noop} />);
    expect(running.match(/<button[^>]*disabled=""/g)?.length).toBe(withButtons.length);
    const idle = renderToStaticMarkup(<NotesList view={card({ notes: withButtons })} reload={noop} />);
    expect(idle.match(/<button/g)?.length).toBe(withButtons.length);
    expect(idle).not.toContain('disabled=""');
  });

  it("what a note's button did", () => {
    expect(noteActionMessage("retryAfterFailedBefore", "done")).toEqual({ text: "Whimbrel will try again the next time it starts.", error: false });
    expect(noteActionMessage("retryAfterFailedBefore", "failed")?.error).toBe(true);
    expect(noteActionMessage("resetEncryptionStatus", "done")).toBeNull();
    expect(noteActionMessage("resetEncryptionStatus", "partlyDone")?.text).toBe("The encryption status could not be reset. Try again.");
    expect(noteActionMessage("deletePlainBackup", "refused")?.text).toBe("The unencrypted copy could not be deleted. Try again.");
  });
});

describe("the speed check", () => {
  const report: SpeedCheckView = {
    encrypted: true,
    openMs: 42,
    tooLittleData: false,
    keywordP50Ms: 12.34,
    keywordP95Ms: 56.78,
    semanticUnavailable: false,
    semanticP50Ms: 100,
    semanticP95Ms: null,
    cpuArch: "aarch64",
    ramGib: 16,
    os: "Windows 11",
  };

  it("its lines: one decimal, missing values not measured", () => {
    expect(speedLines(report)).toEqual([
      "Memory database: encrypted",
      "Opening: 42.0 ms",
      "Word search: 12.3 ms typical, 56.8 ms at the slow end",
      "Search by meaning: 100.0 ms typical, not measured ms at the slow end",
      "This PC: aarch64, 16 GB of memory, Windows 11",
    ]);
    expect(speedLines({ ...report, tooLittleData: true, encrypted: false, ramGib: null })).toEqual([
      "Memory database: not encrypted",
      "Opening: 42.0 ms",
      "Not enough data to time searches yet.",
      "This PC: aarch64, not measured GB of memory, Windows 11",
    ]);
    expect(speedLines({ ...report, semanticUnavailable: true })[3]).toBe("Search by meaning: not timed, the search model is not available");
  });

  it("a meeting recording refuses it", async () => {
    const c = new SpeedCheck({ run: async () => ({ kind: "recording" }), cancel: async () => {}, save: async () => null });
    await c.start();
    expect(c.getState()).toEqual({ kind: "idle", message: "A meeting is being recorded. Check the speed when it ends." });
  });

  it("Cancel returns at once and drops the result; no new check until the dropped run returned", async () => {
    let finish!: (r: SpeedCheckResult) => void;
    const cancel = vi.fn(async () => {});
    const run = vi.fn(() => new Promise<SpeedCheckResult>((r) => (finish = r)));
    const c = new SpeedCheck({ run, cancel, save: async () => null });
    const running = c.start();
    c.cancel();
    expect(cancel).toHaveBeenCalledOnce();
    expect(c.getState()).toEqual({ kind: "idle", message: null });
    expect(c.canStart).toBe(false);
    await c.start();
    expect(run).toHaveBeenCalledOnce();
    finish({ kind: "done", report });
    await running;
    expect(c.getState().kind).toBe("idle");
    expect(c.canStart).toBe(true);
  });

  it("a finished check shows the report, and Save says where it went", async () => {
    const c = new SpeedCheck({ run: async () => ({ kind: "done", report }), cancel: async () => {}, save: async () => "D:\\Whimbrel encryption speed.txt" });
    await c.start();
    await c.save();
    expect(c.getState()).toEqual({ kind: "done", report, savedTo: "D:\\Whimbrel encryption speed.txt" });
  });
});

describe("the Settings setup, from the shell", () => {
  it("takes reusesKey from the shell after the precheck, not from the notes", async () => {
    const reused = new EncryptFlow({ precheck: vi.fn(async () => null), continueSetup: vi.fn(async () => true), encrypt: vi.fn(async () => "atNextLaunch" as const), reusesKey: vi.fn(async () => true) });
    await reused.start();
    expect(reused.getState().kind).toBe("settingUp");
    expect(reused.reusesKey).toBe(true);
    const fresh = new EncryptFlow({ precheck: vi.fn(async () => null), continueSetup: vi.fn(async () => true), encrypt: vi.fn(async () => "atNextLaunch" as const), reusesKey: vi.fn(async () => false) });
    await fresh.start();
    expect(fresh.reusesKey).toBe(false);
  });

  it("Cancel has the shell drop the words and recovery text", async () => {
    const discard = vi.fn(async () => true);
    const f = new EncryptFlow({ precheck: vi.fn(async () => null), continueSetup: vi.fn(async () => true), encrypt: vi.fn(async () => "atNextLaunch" as const), discard });
    await f.start();
    f.cancel();
    expect(discard).toHaveBeenCalledTimes(1);
    expect(f.getState()).toEqual({ kind: "idle", message: null });
  });

  it("leaving Settings cancels: mid-setup the words go; idle or encrypting nothing changes", async () => {
    // The card cancels the flow on unmount; Cancel acts only while checking or setting up.
    const discard = vi.fn(async () => true);
    const idle = new EncryptFlow({ precheck: vi.fn(async () => null), continueSetup: vi.fn(async () => true), encrypt: vi.fn(async () => "atNextLaunch" as const), discard });
    idle.cancel();
    expect(discard).not.toHaveBeenCalled();
    const encrypting = new EncryptFlow({
      precheck: vi.fn(async () => null),
      continueSetup: vi.fn(async () => true),
      encrypt: vi.fn(() => new Promise<"atNextLaunch">(() => {})),
      discard,
    });
    await encrypting.start();
    void encrypting.finish();
    await Promise.resolve();
    encrypting.cancel();
    expect(encrypting.getState().kind).not.toBe("idle");
    expect(discard).not.toHaveBeenCalled();
    const midSetup = new EncryptFlow({ precheck: vi.fn(async () => null), continueSetup: vi.fn(async () => true), encrypt: vi.fn(async () => "atNextLaunch" as const), discard });
    await midSetup.start();
    midSetup.cancel();
    expect(discard).toHaveBeenCalledOnce();
  });
});
