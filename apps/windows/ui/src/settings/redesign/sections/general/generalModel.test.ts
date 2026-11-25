import { describe, expect, it } from "vitest";
import type { SkinPalette } from "../../../../skin";
import { aboutItems, actionsWaitNote, dotColor, updateRowState } from "./generalModel";

const status = { version: "1.4.0", deviceId: "dev-1", dataDir: "C:\\Users\\me\\AppData\\Roaming\\Whimbrel", storeOpen: true, embeddingModelLoaded: false };

describe("the About list", () => {
  it("has the old card's facts, with the database called your library", () => {
    const items = aboutItems(status);
    expect(items.map((i) => [i.label, i.value])).toEqual([
      ["Version", "1.4.0"],
      ["Device ID", "dev-1"],
      ["Data folder", "C:\\Users\\me\\AppData\\Roaming\\Whimbrel"],
      ["Your library", "Open"],
      ["Semantic search model", "Not downloaded yet"],
    ]);
    expect(JSON.stringify(items)).not.toMatch(/memory database/i);
  });

  it("says when there is no device id or the library is closed, and copies only real values", () => {
    const items = aboutItems({ ...status, deviceId: null, storeOpen: false, embeddingModelLoaded: true });
    expect(items[1]).toMatchObject({ value: "not available", copy: false, mono: false });
    expect(items[3].value).toBe("Not open");
    expect(items[4].value).toBe("Loaded");
    expect(aboutItems(status)[2]).toMatchObject({ copy: true, mono: true });
  });
});

describe("the update row", () => {
  it("follows the old card: a line per state, the button waits while checking, Restart only when ready", () => {
    expect(updateRowState({ state: "idle" }, false)).toEqual({ line: null, busy: false, canRestart: false });
    expect(updateRowState({ state: "checking" }, false)).toMatchObject({ line: "Checking…", busy: true, canRestart: false });
    expect(updateRowState({ state: "downloading", version: "1.5.0" }, false)).toMatchObject({ line: "Downloading v1.5.0…", busy: true });
    expect(updateRowState({ state: "upToDate", version: "1.4.0" }, false).line).toBe("Whimbrel is up to date (v1.4.0).");
    expect(updateRowState({ state: "ready", version: "1.5.0" }, false)).toEqual({ line: "v1.5.0 is ready.", busy: false, canRestart: true });
    expect(updateRowState({ state: "ready", version: "1.5.0" }, true).busy).toBe(true);
    expect(updateRowState({ state: "failed" }, false).line).toBe("Couldn't check for updates. Try again later.");
  });
});

describe("the skin card colours", () => {
  const palette = {
    id: "p",
    name: "P",
    light: { backgroundPage: "#fff", backgroundCard: "#eee", accentPrimary: "#00f", accentSecondary: "#0ff", textPrimary: "#000" },
  } as unknown as SkinPalette;

  it("uses the accent of the mode, and the variant the palette has", () => {
    expect(dotColor(palette, "light")).toBe("#00f");
    expect(dotColor(palette, "dark")).toBe("#00f");
  });

  it("says why the skin actions wait", () => {
    expect(actionsWaitNote()).toBe("Rename, export, delete and import wait until the skin chat finishes.");
  });
});
