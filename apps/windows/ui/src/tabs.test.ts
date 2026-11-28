import { describe, expect, it } from "vitest";
import { DEFAULT_TAB, shownTab, tabs } from "./tabs";

describe("tabs", () => {
  it("lists the sections in order, with Conversations after Topics", () => {
    expect(tabs().map((tab) => tab.id)).toEqual(["today", "meetings", "library", "topics", "conversations", "automations", "settings"]);
    expect(tabs().map((tab) => tab.label)).toEqual(["Today", "Meetings", "Library", "Topics", "Conversations", "Automations", "Settings"]);
  });

  it("shows the default tab when the active one is not in the list", () => {
    expect(shownTab([{ id: "today", label: "Today" }], "conversations").id).toBe("today");
    expect(shownTab(tabs(), "conversations").id).toBe("conversations");
    expect(shownTab(tabs(), "settings").id).toBe("settings");
  });

  it("marks no tab as coming: every tab now has its content (Automations arrived with M3.4)", () => {
    expect(tabs().filter((tab) => tab.comingIn).map((tab) => tab.id)).toEqual([]);
  });

  it("defaults to a real tab", () => {
    expect(tabs().some((tab) => tab.id === DEFAULT_TAB)).toBe(true);
  });
});
