import { describe, expect, it } from "vitest";
import { DEFAULT_TAB, shownTab, tabs } from "./tabs";

describe("tabs", () => {
  it("matches the Mac's tabs in order", () => {
    expect(tabs().map((tab) => tab.label)).toEqual(["Today", "Meetings", "Library", "Topics", "Automations", "Settings"]);
  });

  it("adds Conversations after Topics with the redesign switch on, and only then", () => {
    expect(tabs(true).map((tab) => tab.id)).toEqual(["today", "meetings", "library", "topics", "conversations", "automations", "settings"]);
    expect(tabs(false).map((tab) => tab.id)).toEqual(tabs().map((tab) => tab.id));
    expect(tabs(false).some((tab) => tab.id === "conversations")).toBe(false);
  });

  it("shows the default tab when the active one is not in the list (the switch went off with Conversations open)", () => {
    expect(shownTab(tabs(false), "conversations").id).toBe("today");
    expect(shownTab(tabs(true), "conversations").id).toBe("conversations");
    expect(shownTab(tabs(false), "settings").id).toBe("settings");
  });

  it("marks no tab as coming: every tab now has its content (Automations arrived with M3.4)", () => {
    expect(tabs().filter((tab) => tab.comingIn).map((tab) => tab.id)).toEqual([]);
  });

  it("defaults to a real tab", () => {
    expect(tabs().some((tab) => tab.id === DEFAULT_TAB)).toBe(true);
  });
});
