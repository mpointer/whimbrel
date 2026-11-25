import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import type { FoundCard } from "../../generated/types";
import { DayLine } from "./FindResultsView";

const card = (id: number, ts: number): FoundCard => ({ kind: "capture", id, found: true, title: `Window ${id}`, text: `A long text for card ${id}`, tsUnixMs: ts });

describe("the time line's days", () => {
  it("names each day's list item by its day alone, not by everything in it", () => {
    const day = (d: number, h: number) => new Date(2026, 10, d, h).getTime();
    const html = renderToStaticMarkup(<DayLine label="Results by day" cards={[card(1, day(13, 9)), card(2, day(13, 11)), card(3, day(10, 9))]} order="newest" render={(c) => <li key={c.id}>{c.title}</li>} />);
    const items = [...html.matchAll(/<li class="w-spine-stop[^"]*"[^>]*aria-labelledby="([^"]+)"/g)].map((m) => m[1]);
    expect(items).toHaveLength(2);
    for (const id of items) {
      const heading = new RegExp(`<h3[^>]*id="${id}"[^>]*>([^<]+)</h3>`).exec(html);
      expect(heading?.[1]).toBeTruthy();
      expect(heading?.[1]).not.toContain("Window");
    }
    expect(new Set(items).size).toBe(2);
  });
});
