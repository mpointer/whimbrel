import { describe, expect, it } from "vitest";
import type { TopicListItemView } from "../generated/types";
import { attendeeLine, chooseTopic, dueText, groupOf, listHint, parseDay, summaryLine, todayIso } from "./topicPageModel";

const L = "en-US";
const at = (y: number, m: number, d: number, h = 10) => new Date(y, m - 1, d, h, 0).getTime();

const entry = (id: number, next: number | null, openMine: number): TopicListItemView => ({
  topic: { topicId: id, key: `k${id}`, name: `T${id}` },
  kind: null,
  nextMeetingUnixMs: next,
  openMine,
});

describe("dates", () => {
  it("reads a local day and writes today", () => {
    expect(parseDay("2026-11-14")?.getDate()).toBe(14);
    expect(parseDay("Friday")).toBeNull();
    expect(todayIso(new Date(2026, 0, 5))).toBe("2026-01-05");
  });

  it("says overdue once the day has passed", () => {
    expect(dueText("2026-11-07", L, "2026-11-08")).toBe("overdue");
    expect(dueText("2026-11-14", L, "2026-11-08")).toBe("due Sat, Nov 14");
    expect(dueText("2026-11-08", L, "2026-11-08")).toBe("due Sun, Nov 8");
  });
});

describe("summaryLine", () => {
  it("leads with a key date or a step, then the next call", () => {
    const next = { title: "Sync", startUnixMs: at(2026, 11, 18), attendeesByOrg: [], me: 1 };
    expect(summaryLine({ lead: { kind: "date", label: "SOW due", date: "2026-11-14" }, nextMeeting: next }, L, "2026-11-08")).toBe(
      "SOW due Sat, Nov 14 · next call Wed, Nov 18, 10:00 AM",
    );
    expect(summaryLine({ lead: { kind: "step", text: "Send the SOW", dueDate: "2026-11-01" }, nextMeeting: null }, L, "2026-11-08")).toBe(
      "Send the SOW overdue",
    );
    expect(summaryLine({ lead: { kind: "step", text: "Send the SOW", dueDate: null }, nextMeeting: null }, L)).toBe("Send the SOW");
    expect(summaryLine({ lead: null, nextMeeting: null }, L)).toBeNull();
  });
});

describe("attendeeLine", () => {
  it("lists organisations, then the user", () => {
    const line = attendeeLine({ title: "x", startUnixMs: 0, attendeesByOrg: [{ org: "maximus.com", count: 3 }], me: 1 });
    expect(line).toBe("3 maximus.com · 1 you");
  });
});

describe("listHint", () => {
  const now = at(2026, 11, 8);
  it("names a call within a week and the steps owed", () => {
    expect(listHint(entry(1, at(2026, 11, 10), 2), L, now)).toBe("call Tue · 2 steps");
    expect(listHint(entry(1, at(2026, 11, 30), 0), L, now)).toBeNull();
    expect(listHint(entry(1, null, 1), L, now)).toBe("1 step");
  });
});

describe("groupOf and chooseTopic", () => {
  it("puts unknown kinds under Other", () => {
    expect([groupOf("customer"), groupOf(null), groupOf("planet")]).toEqual(["customer", "other", "other"]);
  });

  it("keeps the chosen topic while it is listed, else the first", () => {
    const list = [entry(4, null, 0), entry(9, null, 0)];
    expect(chooseTopic(list, 9)).toBe(9);
    expect(chooseTopic(list, 7)).toBe(4);
    expect(chooseTopic([], 7)).toBeNull();
  });
});
