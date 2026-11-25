import { describe, expect, it } from "vitest";
import { filterInput, filterKey, keptChips } from "./findModel";

const local = (y: number, m: number, d: number, h = 0, min = 0) => new Date(y, m - 1, d, h, min).getTime();
const now = local(2026, 11, 21, 12);

describe("the filters a reopened Find conversation kept", () => {
  it("shows whole days as the same days and asks under exactly the kept bounds", () => {
    const kept = { start_unix_ms: local(2026, 11, 9), end_unix_ms: local(2026, 11, 16), app_bundle_id: "excel.exe", people_left_out: false };
    const chips = keptChips(kept);
    expect(chips).toEqual({ person: null, app: "excel.exe", dates: { kind: "custom", from: "2026-11-09", to: "2026-11-15" } });
    const input = filterInput(chips, now);
    expect([input.startUnixMs, input.endUnixMs, input.app, input.person]).toEqual([kept.start_unix_ms, kept.end_unix_ms, "excel.exe", null]);
  });

  it("keeps a span that is not whole days as that span", () => {
    const kept = { start_unix_ms: local(2026, 11, 20, 10), end_unix_ms: local(2026, 11, 20, 11, 30), app_bundle_id: null, people_left_out: false };
    const chips = keptChips(kept);
    expect(chips.dates?.kind).toBe("span");
    const input = filterInput(chips, now);
    expect([input.startUnixMs, input.endUnixMs]).toEqual([kept.start_unix_ms, kept.end_unix_ms]);
  });

  it("is never a filter change from the filter the question was asked under, for each preset and with an app", () => {
    for (const dates of [{ kind: "today" as const }, { kind: "yesterday" as const }, { kind: "last7" as const }, { kind: "custom" as const, from: "2026-11-01", to: "2026-11-03" }, null]) {
      const asked = filterInput({ person: "Sam Okafor", app: "excel.exe", dates }, now);
      // What core keeps: the bounds and the app, and only that a person was on.
      const kept = { start_unix_ms: asked.startUnixMs ?? null, end_unix_ms: asked.endUnixMs ?? null, app_bundle_id: asked.app ?? null, people_left_out: true };
      const reopened = filterInput(keptChips(kept), now);
      expect(filterKey(reopened)).toBe(filterKey({ ...asked, person: null }));
      expect(reopened.person).toBeNull();
    }
  });

  it("has no chips for nothing kept, or for a range with a side missing", () => {
    expect(keptChips(null)).toEqual({ person: null, app: null, dates: null });
    expect(keptChips({ start_unix_ms: 5, end_unix_ms: null, app_bundle_id: null, people_left_out: true }).dates).toBeNull();
  });
});
