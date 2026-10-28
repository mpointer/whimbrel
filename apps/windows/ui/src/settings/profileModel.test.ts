import { describe, expect, it } from "vitest";
import type { ProfileView, SuggestionView } from "../generated/types";
import {
  SUGGESTIONS_EMPTY,
  SUGGEST_PRIVACY,
  addFactBlocker,
  canAddFact,
  counterLine,
  factsLine,
  fieldsDirty,
  isOverCap,
  previewText,
  sourceLabel,
  suggestStatus,
  toDraft,
} from "./profileModel";

const view = (over: Partial<ProfileView> = {}): ProfileView => ({
  name: "",
  role: "",
  organization: "",
  about: "",
  facts: [],
  enabled: true,
  suggestEnabled: true,
  suggestionsActive: false,
  preview: "",
  composedChars: 0,
  maxChars: 2000,
  maxFacts: 30,
  maxFactChars: 200,
  ...over,
});

const fact = (id: string) => ({ id, text: `fact ${id}`, sourceMeetingId: null });

describe("counterLine", () => {
  it("says how much of the cap is used", () => {
    expect(counterLine(view({ composedChars: 1203 }))).toBe("1,203 of 2,000 characters");
    expect(counterLine(view({ composedChars: 0 }))).toBe("0 of 2,000 characters");
    expect(counterLine(view({ composedChars: 2000 }))).toBe("2,000 of 2,000 characters");
  });

  it("says how much is not sent once it is over", () => {
    const v = view({ composedChars: 2431 });
    expect(counterLine(v)).toBe("2,431 of 2,000 characters. The last 431 are not sent. Shorten it to send all of it.");
    expect(isOverCap(v)).toBe(true);
    expect(isOverCap(view({ composedChars: 2000 }))).toBe(false);
  });
});

describe("facts", () => {
  it("counts the list against its limit", () => {
    expect(factsLine(view({ facts: [fact("a"), fact("b"), fact("c")] }))).toBe("3 of 30 facts");
  });

  it("blocks a fact that is too long, counting a whitespace run as one space and a character as one", () => {
    const v = view();
    expect(addFactBlocker(v, "x".repeat(200))).toBeNull();
    expect(addFactBlocker(v, "x".repeat(201))).toBe("A fact can be at most 200 characters (this one is 201).");
    expect(addFactBlocker(v, `${"x".repeat(100)}\n\n   \t${"y".repeat(99)}`)).toBeNull();
    expect(addFactBlocker(v, "😀".repeat(200))).toBeNull();
    expect(addFactBlocker(v, "😀".repeat(201))).toContain("201");
  });

  it("blocks a fact when the list is full, and says what to do", () => {
    const full = view({ facts: Array.from({ length: 30 }, (_, i) => fact(String(i))) });
    expect(addFactBlocker(full, "one more")).toBe("You can keep at most 30 facts. Remove one first.");
    expect(canAddFact(full, "one more")).toBe(false);
  });

  it("stays quiet about empty text but does not allow adding it", () => {
    expect(addFactBlocker(view(), "")).toBeNull();
    expect(addFactBlocker(view(), "  \n ")).toBeNull();
    expect(canAddFact(view(), "   ")).toBe(false);
    expect(canAddFact(view(), "Works in UTC-5")).toBe(true);
  });
});

describe("the fields draft", () => {
  it("starts as what is saved and is only dirty once it differs", () => {
    const v = view({ name: "Mike", role: "Founder", organization: "Whimbrel", about: "Builds things." });
    const draft = toDraft(v);
    expect(draft).toEqual({ name: "Mike", role: "Founder", organization: "Whimbrel", about: "Builds things." });
    expect(fieldsDirty(v, draft)).toBe(false);
    for (const key of ["name", "role", "organization", "about"] as const) {
      expect(fieldsDirty(v, { ...draft, [key]: `${draft[key]} more` }), key).toBe(true);
    }
  });

  it("treats a trailing space as a change, so what is saved is what was typed", () => {
    const v = view({ name: "Mike" });
    expect(fieldsDirty(v, { ...toDraft(v), name: "Mike " })).toBe(true);
  });
});

describe("the suggestion switch", () => {
  it("says it will not look at meetings when it is off", () => {
    expect(suggestStatus(view({ suggestEnabled: false, suggestionsActive: false }))).toMatch(/^Off\./);
    expect(suggestStatus(view({ suggestEnabled: false, suggestionsActive: false }))).toMatch(/will not look at your meetings/);
  });

  it("says it is waiting while the Profile is empty, so the default-on switch never looks active when it is not", () => {
    const line = suggestStatus(view({ suggestEnabled: true, suggestionsActive: false }));
    expect(line).toMatch(/^On, but waiting/);
    expect(line).toMatch(/field or a fact/);
  });

  it("says what it does once armed", () => {
    const line = suggestStatus(view({ suggestEnabled: true, suggestionsActive: true }));
    expect(line).toMatch(/Once a day/);
    expect(line).toMatch(/summaries/);
  });

  it("states what is sent and where, next to the switch", () => {
    expect(SUGGEST_PRIVACY).toMatch(/meeting summaries/);
    expect(SUGGEST_PRIVACY).toMatch(/AI routing you chose/);
    expect(SUGGEST_PRIVACY).toMatch(/hosted model sees them unless Local-only/);
    expect(SUGGEST_PRIVACY).toMatch(/Nothing is added unless you press Add/);
  });
});

describe("what Whimbrel sees", () => {
  it("is the exact block when something is sent", () => {
    expect(previewText(view({ preview: "Role: CTO\nFacts:\n- Works in UTC-5" }))).toBe("Role: CTO\nFacts:\n- Works in UTC-5");
  });

  it("says nothing is sent when the switch is off, even with a profile written", () => {
    expect(previewText(view({ enabled: false, preview: "" }))).toBe("Off. Nothing from your Profile is sent with a prompt.");
  });

  it("says nothing is sent when nothing is written", () => {
    expect(previewText(view({ enabled: true, preview: "" }))).toBe("Nothing written yet. Nothing is sent.");
  });
});

describe("suggestions", () => {
  const suggestion = (over: Partial<SuggestionView>): SuggestionView => ({
    id: 1,
    text: "x",
    sourceMeetingId: null,
    sourceMeetingTitle: null,
    createdAtUnixMs: 0,
    ...over,
  });

  it("names the meeting a suggestion came from", () => {
    expect(sourceLabel(suggestion({ sourceMeetingId: 7, sourceMeetingTitle: "Pricing review" }))).toBe("From “Pricing review”");
  });

  it("says so when that meeting is gone, and says nothing when there never was one", () => {
    expect(sourceLabel(suggestion({ sourceMeetingId: 7, sourceMeetingTitle: null }))).toBe("From a meeting that is no longer here");
    expect(sourceLabel(suggestion({ sourceMeetingId: null }))).toBeNull();
  });

  it("explains when suggestions appear, and that nothing is added without Add", () => {
    expect(SUGGESTIONS_EMPTY).toMatch(/Once a day/);
    expect(SUGGESTIONS_EMPTY).toMatch(/Nothing is added unless you press Add/);
  });
});
