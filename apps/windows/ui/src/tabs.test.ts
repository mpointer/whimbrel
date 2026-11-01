import { describe, expect, it } from "vitest";
import { DEFAULT_TAB, tabs } from "./tabs";

describe("tabs", () => {
  it("matches the Mac's five tabs in order", () => {
    expect(tabs().map((tab) => tab.label)).toEqual(["Today", "Meetings", "Library", "Automations", "Settings"]);
  });

  it("marks no tab as coming: every tab now has its content (Automations arrived with M3.4)", () => {
    expect(tabs().filter((tab) => tab.comingIn).map((tab) => tab.id)).toEqual([]);
  });

  it("defaults to a real tab", () => {
    expect(tabs().some((tab) => tab.id === DEFAULT_TAB)).toBe(true);
  });
});
