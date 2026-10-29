import { describe, expect, it } from "vitest";
import type { ActionItemView, SnapshotsView, SnapshotView, WorkspaceView } from "../generated/types";
import {
  displayOrder,
  movedItem,
  overrideOf,
  phaseTitle,
  prunedThumbnails,
  slidesLabel,
  snapshotKey,
  thumbnailsToFetch,
  unsavedEditsWouldBeLost,
  withoutSnapshot,
} from "./workspaceModel";

const item = (id: number, over: Partial<ActionItemView> = {}): ActionItemView => ({
  id,
  text: `item ${id}`,
  completed: false,
  source: "generated",
  ...over,
});

const view = (over: Partial<WorkspaceView> = {}): WorkspaceView =>
  ({
    phase: "after",
    row: { id: 1, tsUnixMs: 0, endUnixMs: null, title: "Standup", status: "completed", hasSummary: true, device: { label: "This PC", isThisDevice: true } },
    attendees: [],
    trigger: "manual",
    conferenceLink: null,
    elapsedSeconds: null,
    processingStage: null,
    summary: "generated summary",
    summaryIsEdited: false,
    decisions: ["ship it"],
    decisionsAreEdited: false,
    openQuestions: [],
    openQuestionsAreEdited: false,
    personalNotes: "",
    actionItems: [],
    hasTranscript: true,
    readOnly: false,
    ...over,
  }) as WorkspaceView;

describe("unsavedEditsWouldBeLost", () => {
  it("is false while the fields still match what was generated", () => {
    expect(unsavedEditsWouldBeLost(view(), { summary: "generated summary" })).toBe(false);
  });

  it("catches an edit the autosave hasn't stored yet", () => {
    expect(unsavedEditsWouldBeLost(view(), { summary: "my own words" })).toBe(true);
    expect(unsavedEditsWouldBeLost(view(), { summary: "generated summary", decisions: ["ship it", "and tell support"] })).toBe(true);
  });
});

describe("overrideOf", () => {
  it("clears the override when the text is back to the generated one", () => {
    expect(overrideOf("  same  ", "same")).toBe(null);
    expect(overrideOf("different", "same")).toBe("different");
    expect(overrideOf("typed", null)).toBe("typed");
  });
});

describe("movedItem", () => {
  const items = [item(1), item(2), item(3)];

  it("swaps with the neighbour in that direction", () => {
    expect(movedItem(items, 2, -1).map((i) => i.id)).toEqual([2, 1, 3]);
    expect(movedItem(items, 2, 1).map((i) => i.id)).toEqual([1, 3, 2]);
  });

  it("leaves the ends and unknown ids alone", () => {
    expect(movedItem(items, 1, -1)).toBe(items);
    expect(movedItem(items, 3, 1)).toBe(items);
    expect(movedItem(items, 99, 1)).toBe(items);
  });
});

describe("displayOrder", () => {
  it("sinks completed items without reordering the rest", () => {
    const items = [item(1, { completed: true }), item(2), item(3, { completed: true }), item(4)];
    expect(displayOrder(items).map((i) => i.id)).toEqual([2, 4, 1, 3]);
  });
});

describe("phaseTitle", () => {
  it("names each phase", () => {
    expect(phaseTitle(view({ phase: "before" }))).toBe("Before the meeting");
    expect(phaseTitle(view({ phase: "during" }))).toBe("Recording");
    expect(phaseTitle(view({ phase: "after" }))).toBe("After the meeting");
    expect(phaseTitle(view({ phase: "cancelled" }))).toBe("Cancelled");
  });
});

const snap = (id: number, over: Partial<SnapshotView> = {}): SnapshotView => ({
  id,
  capturedUnixMs: id * 1_000,
  fileName: `10.00.0${id}.png`,
  width: 1920,
  height: 1080,
  modifiedUnixMs: 5_000,
  text: "",
  ...over,
});

describe("slide thumbnails", () => {
  it("fetches only the slides not already known", () => {
    const list = [snap(1), snap(2), snap(3)];
    const known = new Set([snapshotKey(snap(1)), snapshotKey(snap(3))]);
    expect(thumbnailsToFetch(list, known).map((s) => s.id)).toEqual([2]);
  });

  it("fetches again when a build replaces a slide, even under the same name", () => {
    const before = snap(1);
    const replaced = snap(1, { modifiedUnixMs: 9_000 });
    expect(thumbnailsToFetch([replaced], new Set([snapshotKey(before)]))).toEqual([replaced]);
  });

  it("drops cached thumbnails of slides that are gone or replaced", () => {
    const cache = new Map([
      [snapshotKey(snap(1)), "one"],
      [snapshotKey(snap(2)), "two"],
    ]);
    const pruned = prunedThumbnails(cache, [snap(1, { modifiedUnixMs: 9_000 }), snap(2)]);
    expect([...pruned.values()]).toEqual(["two"]);
  });
});

describe("the slides strip", () => {
  const strip: SnapshotsView = { snapshots: [snap(1), snap(2), snap(3)], keep: true };

  it("counts the slides in its heading", () => {
    expect(slidesLabel(strip)).toBe("Slides (3)");
  });

  it("removes a deleted slide and keeps the rest in order", () => {
    const after = withoutSnapshot(strip, 2);
    expect(after.snapshots.map((s) => s.id)).toEqual([1, 3]);
    expect(after.keep).toBe(true);
    expect(withoutSnapshot(strip, 99).snapshots).toHaveLength(3);
  });
});
