import { describe, expect, it } from "vitest";
import { retentionDecision, shownDays, RETENTION_DEFAULT_DAYS } from "./retentionDecision";

describe("the safe retention rules", () => {
  it("ticking Keep forever saves null at once, and writes nothing when it is already forever", () => {
    expect(retentionDecision(30, 30, { kind: "tickForever" })).toEqual({ kind: "save", days: null });
    expect(retentionDecision(null, 30, { kind: "tickForever" })).toEqual({ kind: "none" });
  });

  it("unticking saves nothing and shows the last number", () => {
    expect(retentionDecision(null, 45, { kind: "untick" })).toEqual({ kind: "none", show: 45 });
  });

  it("a shorter period asks first and says it was shorter", () => {
    expect(retentionDecision(90, 90, { kind: "commit", value: 30 })).toEqual({ kind: "confirm", days: 30, why: "shorter", from: 90 });
  });

  it("a longer period saves at once", () => {
    expect(retentionDecision(30, 30, { kind: "commit", value: 90 })).toEqual({ kind: "save", days: 90 });
  });

  it("the same number writes nothing", () => {
    expect(retentionDecision(30, 30, { kind: "commit", value: 30 })).toEqual({ kind: "none" });
  });

  it("an empty, invalid or out of range number saves nothing and says which problem", () => {
    expect(retentionDecision(30, 30, { kind: "commitText", text: "" })).toEqual({ kind: "reject", problem: "empty" });
    expect(retentionDecision(30, 30, { kind: "commitText", text: "soon" })).toEqual({ kind: "reject", problem: "invalid" });
    expect(retentionDecision(30, 30, { kind: "commitText", text: "0" })).toEqual({ kind: "reject", problem: "range" });
    expect(retentionDecision(30, 30, { kind: "commitText", text: "3651" })).toEqual({ kind: "reject", problem: "range" });
    expect(retentionDecision(30, 30, { kind: "commitText", text: "2.5" })).toEqual({ kind: "reject", problem: "fraction" });
    expect(retentionDecision(null, 30, { kind: "commit", value: -4 })).toEqual({ kind: "reject", problem: "range" });
  });

  it("any period after forever asks first, even a long one", () => {
    expect(retentionDecision(null, 30, { kind: "commit", value: 3650 })).toEqual({ kind: "confirm", days: 3650, why: "fromForever", from: null });
    expect(retentionDecision(null, 30, { kind: "commitText", text: "30" })).toEqual({ kind: "confirm", days: 30, why: "fromForever", from: null });
  });

  it("an unset window is treated as forever", () => {
    expect(retentionDecision(undefined, 30, { kind: "commit", value: 10 }).kind).toBe("confirm");
  });

  it("the number shown is the stored one, else the last one, else 30", () => {
    expect(shownDays(12, 40)).toBe(12);
    expect(shownDays(null, 40)).toBe(40);
    expect(shownDays(null, undefined)).toBe(RETENTION_DEFAULT_DAYS);
  });
});
