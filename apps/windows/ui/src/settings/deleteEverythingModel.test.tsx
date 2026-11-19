import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";
import type { DeleteEverythingPreview } from "../generated/types";

vi.mock("../api", () => {
  const never = () => new Promise(() => {});
  return { deleteEverythingNow: never, deleteEverythingPreview: never };
});
const { DeleteEverythingDialog, DeleteEverythingCard } = await import("./DeleteEverythingCard");
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
const noop = () => {};
const render = (state = open, p: DeleteEverythingPreview | null = preview()) =>
  renderToStaticMarkup(<DeleteEverythingDialog preview={p} state={state} onTyped={noop} onSyncAccount={noop} onCancel={noop} onConfirm={noop} />);
/** The "Delete and restart" button's tag, to read its disabled attribute. */
const deleteButton = (html: string) => html.match(/<button[^>]*>(Delete and restart|Deleting…)<\/button>/)?.[0] ?? "";
const cancelButton = (html: string) => html.match(/<button[^>]*>Cancel<\/button>/)?.[0] ?? "";

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

  it("names a slide folder it will not delete from", () => {
    const refused = "Whimbrel will not delete the slide images because their folder is not safe to delete from (a link, or a top-level folder such as Documents). Remove them by hand from: C:\\Users\\me\\Documents";
    const html = render(open, preview({ snapshotsRefused: refused }));
    expect(html).toContain("not safe to delete from");
    expect(html).not.toContain("removes the meeting slide images");
  });

  it("offers to delete the sync account only when sync is on, and says what each choice means", () => {
    expect(render(open, preview())).not.toContain("Also delete my sync account");
    const html = render(open, preview({ syncOn: true }));
    expect(html).toContain("Also delete my sync account");
    expect(html).toContain("download everything again from your other devices unless you also delete your sync account");
    const on = dialogReducer(open, { kind: "syncAccount", on: true });
    expect(on.alsoDeleteSyncAccount).toBe(true);
    expect(render(on, preview({ syncOn: true }))).toContain("every device is signed out of sync");
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
    expect(deleteButton(render())).toContain("disabled");
    for (const typed of ["delete", "DELETE ", "DELET"]) {
      expect(canDelete(dialogReducer(open, { kind: "typed", text: typed }), preview())).toBe(false);
    }
    const typed = dialogReducer(open, { kind: "typed", text: "DELETE" });
    expect(canDelete(typed, preview())).toBe(true);
    expect(deleteButton(render(typed))).not.toContain("disabled");
  });

  it("keeps the buttons off while the delete runs and before the preview is in; a meeting is ended, not a block", () => {
    const typed = dialogReducer(open, { kind: "typed", text: "DELETE" });
    const busy = dialogReducer(typed, { kind: "busy" });
    const html = render(busy);
    expect(deleteButton(html)).toContain("disabled");
    expect(deleteButton(html)).toContain("Deleting…");
    expect(cancelButton(html)).toContain("disabled");
    expect(canDelete(typed, null)).toBe(false);
    const meeting = preview({ endsMeeting: "A meeting is being recorded. Delete everything stops the recording first, and the meeting is deleted with the rest." });
    expect(canDelete(typed, meeting)).toBe(true);
    expect(render(typed, meeting)).toContain("stops the recording first");
  });

  it("Cancel closes it and forgets what was typed and chosen, but not while the delete runs", () => {
    const typed = dialogReducer(dialogReducer(open, { kind: "typed", text: "DELETE" }), { kind: "syncAccount", on: true });
    expect(dialogReducer(typed, { kind: "cancel" })).toEqual(closed);
    const busy = dialogReducer(typed, { kind: "busy" });
    expect(dialogReducer(busy, { kind: "cancel" })).toBe(busy);
    const failed = dialogReducer(busy, { kind: "failed", error: "These slide image folders could not be removed" });
    expect(failed.busy).toBe(false);
    expect(render(failed)).toContain("could not be removed");
    expect(dialogReducer(failed, { kind: "cancel" })).toEqual(closed);
    expect(dialogReducer(failed, { kind: "open" })).toEqual(open);
  });

  it("the card starts closed, with only the button, and says what stays", () => {
    const html = renderToStaticMarkup(<DeleteEverythingCard />);
    expect(html).toContain("Delete everything on this PC…");
    expect(html).toContain("Removes your meetings, memory, recordings and logs from this PC. Your settings (settings.json), downloaded models and API keys stay.");
    expect(html).not.toContain("as new");
    expect(html).not.toContain("Type DELETE");
  });
});
