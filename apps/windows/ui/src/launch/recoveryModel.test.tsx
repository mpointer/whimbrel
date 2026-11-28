import { describe, expect, it, vi } from "vitest";
import type { ChosenFile, UseKeyResult } from "../generated/types";
import { RecoveryModel, recoveryExplanation, unlockMessage, type RecoveryActions } from "./recoveryModel";

function actions(over: Partial<RecoveryActions> = {}): RecoveryActions {
  return {
    chooseFile: vi.fn(async (): Promise<ChosenFile> => ({ kind: "chosen", name: "Whimbrel recovery key.txt" })),
    unlockFile: vi.fn(async (): Promise<UseKeyResult> => "opened"),
    unlockWords: vi.fn(async (): Promise<UseKeyResult> => "opened"),
    ...over,
  };
}

describe("the recovery model", () => {
  it("only restores a key: its actions are choose, unlock with file, unlock with words", () => {
    const methods = Object.getOwnPropertyNames(RecoveryModel.prototype).filter((n) => n !== "constructor");
    expect(methods.sort()).toEqual(
      ["canUnlockFile", "canUnlockWords", "chooseFile", "clear", "set", "setMethod", "setPassphrase", "setWords", "unlock", "unlockFile", "unlockWords"].sort(),
    );
    expect(Object.keys(actions()).sort()).toEqual(["chooseFile", "unlockFile", "unlockWords"]);
  });

  it("the file path needs a file and a passphrase", async () => {
    const a = actions();
    const m = new RecoveryModel(a);
    expect(m.canUnlockFile).toBe(false);
    await m.chooseFile();
    expect(m.getState().fileName).toBe("Whimbrel recovery key.txt");
    expect(m.canUnlockFile).toBe(false);
    m.setPassphrase("my passphrase");
    expect(m.canUnlockFile).toBe(true);
    await m.unlockFile();
    expect(a.unlockFile).toHaveBeenCalledWith("my passphrase");
    expect(m.getState().restored).toBe(true);
    expect(m.getState().passphrase).toBe("");
  });

  it("the words path needs some words, and hands them over as typed", async () => {
    const a = actions();
    const m = new RecoveryModel(a);
    m.setMethod("words");
    m.setWords("   \n ");
    expect(m.canUnlockWords).toBe(false);
    m.setWords("1. Abandon, ability\nable");
    await m.unlockWords();
    expect(a.unlockWords).toHaveBeenCalledWith("1. Abandon, ability\nable");
    expect(m.getState().words).toBe("");
  });

  it("maps every outcome to its copy", () => {
    expect(unlockMessage("opened")).toBeNull();
    expect(unlockMessage("wrongPassphrase")).toContain("That passphrase does not open this recovery file");
    expect(unlockMessage("notARecoveryFile")).toContain("This is not a Whimbrel recovery file.");
    expect(unlockMessage("badWords")).toContain("Those words are not a valid recovery key.");
    expect(unlockMessage("keyRejected")).toContain("That recovery key does not open this memory database.");
    expect(unlockMessage("failed")).toBe("The recovery key could not be used. Your data has not been changed. Try again.");
  });

  it("a rejected key keeps the screen with keyForAnotherDatabase, and keeps what was typed", async () => {
    const m = new RecoveryModel(actions({ unlockWords: async () => "keyRejected" }));
    m.setMethod("words");
    m.setWords("some words");
    await m.unlockWords();
    expect(m.getState().restored).toBe(false);
    expect(m.getState().working).toBe(false);
    expect(m.getState().error).toContain("It may belong to another PC.");
    expect(m.getState().words).toBe("some words");
  });

  it("an unreadable file says so; switching the way in clears the error", async () => {
    const m = new RecoveryModel(actions({ chooseFile: async () => ({ kind: "unreadable" }) }));
    await m.chooseFile();
    expect(m.getState().error).toBe("That file could not be read. Choose it again.");
    m.setMethod("words");
    expect(m.getState().error).toBeNull();
  });

  it("the reason picks the explanation", () => {
    expect(recoveryExplanation("needsKey")).toContain("This PC has no key for the encrypted memory database.");
    expect(recoveryExplanation("keyRejected")).toBe("The key in Windows Credential Manager does not open this memory database.");
  });
});
