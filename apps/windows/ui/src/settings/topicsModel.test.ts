import { describe, expect, it } from "vitest";
import type { TopicSortEstimate, TopicSortView } from "../generated/types";
import { formatUsd, itemCount, parseAliases, seedDetail, sortPrompt } from "./topicsModel";

const ready: TopicSortView = { key: "••••••••3456", enabled: true, started: false, localOnly: false, keyRejected: false, overBudget: false };
const estimate = (e: Partial<TopicSortEstimate>): TopicSortEstimate => ({ sources: 120, tokens: 50_000, usd: 0.0021, indexing: false, ...e });

describe("Topics settings", () => {
  it("splits the editor's other names on commas and drops blanks", () => {
    expect(parseAliases(" Acme, ACME Corp ,, ")).toEqual(["Acme", "ACME Corp"]);
    expect(parseAliases("")).toEqual([]);
  });

  it("shows a topic's kind, then its other names", () => {
    expect(seedDetail({ kind: "customer", aliases: ["Acme", "ACME Corp"] })).toBe("Customer · Acme, ACME Corp");
    expect(seedDetail({ kind: null, aliases: ["Acme"] })).toBe("Acme");
    expect(seedDetail({ kind: "project", aliases: [] })).toBe("Project");
  });

  it("counts items with one plural key", () => {
    expect(itemCount(1)).toBe("1 item");
    expect(itemCount(1487)).toBe("1,487 items");
  });

  it("prices in dollars", () => {
    expect(formatUsd(0.0021)).toBe("$0.00");
    expect(formatUsd(3.5)).toBe("$3.50");
  });
});

describe("the sorting row", () => {
  it("asks for nothing unless the switch is on, Start wasn't pressed, there is a key and Local-only is off", () => {
    for (const view of [
      { ...ready, enabled: false },
      { ...ready, started: true },
      { ...ready, key: null },
      { ...ready, localOnly: true },
    ]) {
      expect(sortPrompt(view, estimate({}), false)).toEqual({ kind: "none" });
    }
  });

  it("waits while sorting, then offers the price and Start", () => {
    expect(sortPrompt(ready, null, false)).toEqual({ kind: "loading" });
    expect(sortPrompt(ready, estimate({ indexing: true }), false)).toEqual({ kind: "sorting" });
    expect(sortPrompt(ready, estimate({ sources: 0 }), false)).toEqual({ kind: "sorting" });
    expect(sortPrompt(ready, estimate({}), false)).toEqual({ kind: "estimate", text: "About 120 items, about $0.00. Start?", tokens: 50_000 });
    expect(sortPrompt(ready, estimate({ sources: 1, usd: 1.25 }), false)).toMatchObject({ text: "About 1 item, about $1.25. Start?" });
  });

  it("offers Try again when the estimate failed", () => {
    expect(sortPrompt(ready, estimate({}), true)).toEqual({ kind: "failed" });
  });
});
