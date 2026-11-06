import { describe, expect, it } from "vitest";
import { DEFAULT_SECTION, rememberSection, rememberedSection, sectionTitle, SETTINGS_SECTIONS, toSection } from "./settingsSections";

function memory() {
  const items = new Map<string, string>();
  return { getItem: (k: string) => items.get(k) ?? null, setItem: (k: string, v: string) => void items.set(k, v) };
}

describe("Settings sections", () => {
  it("uses the Mac's section names, in its order", () => {
    expect(SETTINGS_SECTIONS.map(sectionTitle)).toEqual(["General", "Profile", "Capture & Privacy", "Meetings", "AI", "Sync & Devices", "Connectors"]);
  });

  it("falls back to General for anything that isn't a section", () => {
    expect(toSection("ai")).toBe("ai");
    expect(toSection("automations")).toBe(DEFAULT_SECTION);
    expect(toSection(null)).toBe(DEFAULT_SECTION);
    expect(toSection(undefined)).toBe(DEFAULT_SECTION);
  });

  it("reopens at the section shown last", () => {
    const storage = memory();
    expect(rememberedSection(storage)).toBe("general");
    rememberSection("connectors", storage);
    expect(rememberedSection(storage)).toBe("connectors");
  });

  it("opens at General when storage is missing or throws", () => {
    expect(rememberedSection(null)).toBe("general");
    const broken = {
      getItem: () => {
        throw new Error("blocked");
      },
      setItem: () => {
        throw new Error("blocked");
      },
    };
    expect(rememberedSection(broken)).toBe("general");
    expect(() => rememberSection("ai", broken)).not.toThrow();
  });
});
