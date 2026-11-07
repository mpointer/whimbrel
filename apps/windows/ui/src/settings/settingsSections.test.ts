import { describe, expect, it } from "vitest";
import { DEFAULT_SECTION, sectionTitle, SETTINGS_SECTIONS, toSection } from "./settingsSections";

describe("Settings sections", () => {
  it("opens with Status, then the Mac's section names in its order", () => {
    expect(SETTINGS_SECTIONS.map(sectionTitle)).toEqual(["Status", "General", "Profile", "Capture & Privacy", "Meetings", "AI", "Sync & Devices", "Connectors"]);
    expect(DEFAULT_SECTION).toBe("status");
  });

  it("falls back to Status for anything that isn't a section", () => {
    expect(toSection("ai")).toBe("ai");
    expect(toSection("automations")).toBe(DEFAULT_SECTION);
    expect(toSection(null)).toBe(DEFAULT_SECTION);
    expect(toSection(undefined)).toBe(DEFAULT_SECTION);
  });
});
