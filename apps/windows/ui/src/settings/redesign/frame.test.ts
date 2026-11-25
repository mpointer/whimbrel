import { describe, expect, it } from "vitest";
import frameSource from "./RedesignedSettings.tsx?raw";
import { SETTINGS_SEARCH_ENTRIES } from "../settingsSearch";
import { SETTINGS_SECTIONS } from "../settingsSections";
import { redesignSectionTitle } from "./sections";
import { REDESIGN_SEARCH_ENTRIES, searchRedesign } from "./search";

describe("redesigned Settings frame", () => {
  it("keeps the section ids and order, and renames only Capture & Privacy", () => {
    expect(SETTINGS_SECTIONS.map(redesignSectionTitle)).toEqual(["Status", "General", "Profile", "Plan & billing", "Memory & Privacy", "Meetings", "AI", "Topics", "Sync & Devices", "Connectors"]);
  });

  it("searches the same registry: same ids, same sections", () => {
    expect(REDESIGN_SEARCH_ENTRIES.map((e) => [e.id, e.section])).toEqual(SETTINGS_SEARCH_ENTRIES.map((e) => [e.id, e.section]));
  });

  it("finds the Memory & Privacy section under both 'capture' and 'memory'", () => {
    for (const q of ["capture", "memory"]) {
      const sections = new Set(searchRedesign(q).map((r) => r.section));
      expect(sections.has("capture")).toBe(true);
    }
    expect(searchRedesign("capture").filter((r) => r.section === "capture").length).toBe(SETTINGS_SEARCH_ENTRIES.filter((e) => e.section === "capture").length);
  });

  it("finds a row by its new title and by its old one", () => {
    expect(searchRedesign("library encryption")[0].id).toBe("capture.encryption");
    expect(searchRedesign("database encryption").map((r) => r.id)).toContain("capture.encryption");
    expect(searchRedesign("keep forever")[0].id).toBe("capture.retention");
  });

  it("matches nothing for a blank query and never reads it as a pattern", () => {
    expect(searchRedesign("   ")).toEqual([]);
    expect(searchRedesign("[a-z]+")).toEqual([]);
  });

  it("draws the MCP token host in the frame itself, above the section switch, so a section change cannot drop an open token", () => {
    const host = frameSource.indexOf("<McpTokenHost />");
    expect(host).toBeGreaterThan(-1);
    expect(host).toBeLessThan(frameSource.indexOf("<SectionBody"));
  });
});
