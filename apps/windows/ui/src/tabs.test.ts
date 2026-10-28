import { describe, expect, it } from "vitest";
import { DEFAULT_TAB, TABS } from "./tabs";

describe("TABS", () => {
  it("matches the Mac's five tabs in order", () => {
    expect(TABS.map((t) => t.label)).toEqual(["Today", "Meetings", "Library", "Automations", "Settings"]);
  });

  it("marks no tab as coming: every tab now has its content (Automations arrived with M3.4)", () => {
    expect(TABS.filter((t) => t.comingIn).map((t) => t.id)).toEqual([]);
  });

  it("defaults to a real tab", () => {
    expect(TABS.some((t) => t.id === DEFAULT_TAB)).toBe(true);
  });
});
