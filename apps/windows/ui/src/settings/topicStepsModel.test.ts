import { describe, expect, it } from "vitest";
import type { TopicStepsView } from "../generated/types";
import { nameChips, needsLocalModel, stepsPrompt } from "./topicStepsModel";

const view = (over: Partial<TopicStepsView> = {}): TopicStepsView => ({
  enabled: true,
  started: false,
  hasKey: true,
  localOnly: false,
  keyRejected: false,
  overBudget: false,
  noModel: false,
  ...over,
});

describe("nameChips", () => {
  it("offers suggestions and the profile name not already listed, at most four", () => {
    expect(nameChips(["Mike"], ["mike", "Michael", " "], "Mike Pointer")).toEqual(["Michael", "Mike Pointer"]);
    expect(nameChips([], ["a", "b", "c", "d", "e"], "f")).toEqual(["a", "b", "c", "d"]);
    expect(nameChips(["Mike"], [], "")).toEqual([]);
  });
});

describe("stepsPrompt", () => {
  const estimate = { sources: 12, tokens: 3000, usd: 0.0002, indexing: false };
  it("waits for Start only when TypeSafe could run", () => {
    expect(stepsPrompt(view({ enabled: false }), estimate, false).kind).toBe("none");
    expect(stepsPrompt(view({ started: true }), estimate, false).kind).toBe("none");
    expect(stepsPrompt(view({ hasKey: false }), estimate, false).kind).toBe("none");
    expect(stepsPrompt(view({ localOnly: true }), estimate, false).kind).toBe("none");
  });
  it("shows loading, failed, still sorting, then the estimate", () => {
    expect(stepsPrompt(view(), null, false).kind).toBe("loading");
    expect(stepsPrompt(view(), null, true).kind).toBe("failed");
    expect(stepsPrompt(view(), { ...estimate, indexing: true }, false).kind).toBe("sorting");
    const p = stepsPrompt(view(), estimate, false);
    expect(p).toMatchObject({ kind: "estimate", tokens: 3000 });
  });
});

describe("needsLocalModel", () => {
  it("shows only when the local model should be running", () => {
    expect(needsLocalModel(view({ enabled: false, noModel: true }))).toBe(true);
    expect(needsLocalModel(view({ enabled: true, noModel: true }))).toBe(false);
    expect(needsLocalModel(view({ enabled: false, noModel: false }))).toBe(false);
  });
});
