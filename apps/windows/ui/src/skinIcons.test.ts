import { describe, expect, it } from "vitest";
import { ICON_FONT, ICON_ROLES, ICON_TABLE, iconEntry, iconGlyph } from "./skinIcons";
import { WHIMBREL } from "./skin";

const skinWith = (icons: Record<string, string>) => ({ icons: { ...WHIMBREL.skin.icons, ...icons } });

describe("icons", () => {
  it("draws each role's own choice from the table", () => {
    const skin = skinWith({ today: "terminal", settings: "house" });
    expect(iconEntry(skin, "today").symbol).toBe("terminal");
    expect(iconEntry(skin, "settings").symbol).toBe("house");
    expect(iconGlyph(skin, "today")).toBe(String.fromCodePoint(59222));
  });

  it("renders_mac_made_skin_with_unknown_icon: a name outside the table is the role's Whimbrel icon", () => {
    const skin = skinWith({ today: "airplane.circle.fill", meetings: "", ask: "not-a-symbol" });
    expect(iconEntry(skin, "today").symbol).toBe("sun.max");
    expect(iconEntry(skin, "meetings").symbol).toBe("person.2");
    expect(iconEntry(skin, "ask").symbol).toBe("arrow.up.circle.fill");
    expect(iconGlyph(skin, "today")).toBe(iconGlyph(WHIMBREL.skin, "today"));
  });

  it("draws a role the skin does not mention", () => {
    for (const role of ICON_ROLES) expect(iconGlyph({ icons: {} }, role), role).not.toBe("");
  });

  it("has a Whimbrel icon in the table for every role", () => {
    for (const role of ICON_ROLES) expect(ICON_TABLE.some((e) => e.symbol === WHIMBREL.skin.icons[role]), role).toBe(true);
  });

  it("uses the same code point in Fluent and MDL2 for every symbol, so one glyph serves both fonts", () => {
    for (const e of ICON_TABLE) expect(e.fluent, e.symbol).toBe(e.mdl2);
    expect(ICON_FONT).toContain("Segoe Fluent Icons");
    expect(ICON_FONT).toContain("Segoe MDL2 Assets");
  });

  it("offers every symbol exactly once", () => {
    expect(new Set(ICON_TABLE.map((e) => e.symbol)).size).toBe(ICON_TABLE.length);
  });
});
