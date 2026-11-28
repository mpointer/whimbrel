import { describe, expect, it } from "vitest";
import type { DeleteEverythingPreview } from "../generated/types";

const { canDelete, closed, confirmArgs, confirmLines, dialogReducer, syncAccountLine } = await import("./deleteEverythingModel");

const preview = (over: Partial<DeleteEverythingPreview> = {}): DeleteEverythingPreview => ({
  dataFolder: "C:\\Users\\me\\AppData\\Local\\Whimbrel\\data",
  dataBytes: 2_500_000_000,
  snapshotsBytes: 0,
  syncOn: false,
  syncSignedIn: false,
  googleAccounts: [],
  storeOpen: false,
  ...over,
});
const open = dialogReducer(closed, { kind: "open" });

describe("Delete everything's confirmation", () => {
  it("says what goes and what stays: the data folder and its size, the slide images, never the models", () => {
    const lines = confirmLines(preview({ snapshotsFolder: "C:\\Users\\me\\Pictures\\Whimbrel Snapshots", snapshotsBytes: 3_000_000 }));
    expect(lines[0]).toMatch(/memory database, the logs and everything else in the data folder \(C:\\Users\\me\\AppData\\Local\\Whimbrel\\data, .+\), then restarts Whimbrel\./);
    expect(lines[1]).toMatch(/slide images Whimbrel saved in C:\\Users\\me\\Pictures\\Whimbrel Snapshots/);
    expect(lines.join(" ")).toMatch(/keys in Credential Manager go with it/);
    expect(lines.join(" ")).not.toMatch(/as new/);
  });

  it("names everything that stays (settings.json and the own Google client by name) and the sign-ins it can't remove; the logs go", () => {
    const stays = confirmLines(preview()).find((l) => l.startsWith("These stay:")) ?? "";
    expect(stays).toMatch(/settings\.json/);
    expect(stays).toMatch(/your own Google client ID and secret/);
    expect(stays).not.toMatch(/logs/);
    expect(confirmLines(preview()).join(" ")).toMatch(/starts like a new install, and its first-run notices show again/);
    expect(stays).toMatch(/downloaded models/);
    expect(stays).toMatch(/everything else Whimbrel keeps in Credential Manager, such as your API keys/);
    expect(stays).toMatch(/this PC's sync identity/);
    expect(stays).not.toMatch(/sync sign-in/);
    // A stored sync sign-in stays and says so: sync on with the account kept, or sync not on (waiting, stopped).
    expect(confirmLines(preview({ syncOn: true, syncSignedIn: true }), false).join(" ")).toMatch(/its sync sign-in\./);
    expect(confirmLines(preview({ syncOn: false, syncSignedIn: true }), true).join(" ")).toMatch(/its sync sign-in\./);
    // Account deleted: the sign-in goes with it, so it is not listed as staying.
    expect(confirmLines(preview({ syncOn: true, syncSignedIn: true }), true).join(" ")).not.toMatch(/sync sign-in/);
  });

  it("lists the Google accounts that stay connected, or says they need connecting again when it can't list them", () => {
    const kept = confirmLines(preview({ googleAccounts: ["a@example.com", "b@example.com"] })).join(" ");
    expect(kept).toMatch(/These Google accounts stay connected: a@example\.com, b@example\.com\. Their calendar events, and their mail where Whimbrel can read it, are downloaded again into the new library\./);
    expect(kept).not.toMatch(/disconnect|revoke/);
    expect(confirmLines(preview({ googleAccounts: null })).join(" ")).toMatch(
      /can't list your connected Google accounts .* sign-ins stay in Credential Manager, but you'll need to connect the accounts again/,
    );
    expect(confirmLines(preview()).join(" ")).not.toMatch(/Google accounts/);
  });

  it("tells an unreadable account list from running without the database", () => {
    const failed = confirmLines(preview({ googleAccounts: null, storeOpen: true })).join(" ");
    expect(failed).toMatch(/couldn't list your connected Google accounts/);
    expect(failed).not.toMatch(/without its memory database/);
    const noStore = confirmLines(preview({ googleAccounts: null, storeOpen: false })).join(" ");
    expect(noStore).toMatch(/without its memory database/);
  });

  it("says the sync key goes with a deleted account, and stays otherwise", () => {
    const deleted = confirmLines(preview({ syncOn: true, syncSignedIn: true }), true).join(" ");
    expect(deleted).toMatch(/sync key goes with the account/);
    expect(confirmLines(preview({ syncOn: true, syncSignedIn: true }), false).join(" ")).not.toMatch(/sync key goes/);
  });

  it("says activity capture is off now and back on after the restart, as the settings file keeps it", () => {
    expect(confirmLines(preview()).join(" ")).toMatch(/Activity capture is turned off now and comes back on after the restart/);
  });

  it("records the sync account choice, and says what it means", () => {
    const on = dialogReducer(open, { kind: "syncAccount", on: true });
    expect(on.alsoDeleteSyncAccount).toBe(true);
    expect(syncAccountLine(true)).toMatch(/Your other devices keep their own data/);
    // Not while it runs.
    const busy = dialogReducer(dialogReducer(on, { kind: "typed", text: "DELETE" }), { kind: "busy" });
    expect(dialogReducer(busy, { kind: "syncAccount", on: false }).alsoDeleteSyncAccount).toBe(true);
  });

  it("sends the typed word, the account choice (only with sync on) and whether the slides were said to stay", () => {
    const typed = dialogReducer(dialogReducer(open, { kind: "syncAccount", on: true }), { kind: "typed", text: "DELETE" });
    expect(confirmArgs(typed, preview({ syncOn: true }))).toEqual({ confirm: "DELETE", alsoDeleteSyncAccount: true, slidesLeftAlone: false });
    expect(confirmArgs(typed, preview({ syncOn: false })).alsoDeleteSyncAccount).toBe(false);
    expect(confirmArgs(typed, preview({ snapshotsRefused: "not safe" })).slidesLeftAlone).toBe(true);
  });

  it("turns Delete and restart on only once DELETE is typed exactly", () => {
    expect(canDelete(open, preview())).toBe(false);
    for (const typed of ["delete", "DELETE ", "DELET"]) {
      expect(canDelete(dialogReducer(open, { kind: "typed", text: typed }), preview())).toBe(false);
    }
    const typed = dialogReducer(open, { kind: "typed", text: "DELETE" });
    expect(canDelete(typed, preview())).toBe(true);
  });

  it("keeps Delete off before the preview is in; a meeting is ended, not a block", () => {
    const typed = dialogReducer(open, { kind: "typed", text: "DELETE" });
    expect(canDelete(typed, null)).toBe(false);
    const meeting = preview({ endsMeeting: "A meeting is being recorded. Delete everything stops the recording first, and the meeting is deleted with the rest." });
    expect(canDelete(typed, meeting)).toBe(true);
  });

  it("Cancel closes it and forgets what was typed and chosen, but not while the delete runs", () => {
    const typed = dialogReducer(dialogReducer(open, { kind: "typed", text: "DELETE" }), { kind: "syncAccount", on: true });
    expect(dialogReducer(typed, { kind: "cancel" })).toEqual(closed);
    const busy = dialogReducer(typed, { kind: "busy" });
    expect(dialogReducer(busy, { kind: "cancel" })).toBe(busy);
    const failed = dialogReducer(busy, { kind: "failed", error: "These slide image folders could not be removed" });
    expect(failed.busy).toBe(false);
    expect(failed.error).toContain("could not be removed");
    expect(dialogReducer(failed, { kind: "cancel" })).toEqual(closed);
    expect(dialogReducer(failed, { kind: "open" })).toEqual(open);
  });
});
