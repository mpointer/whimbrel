import { describe, expect, it } from "vitest";
import { contrastRatio } from "../src/themes";
import { composite, parseColor, ratio, type Rgb } from "./contrast";

const hex = (c: Rgb) => `#${c.map((v) => v.toString(16).padStart(2, "0")).join("")}`;

describe("the matrix's contrast ratio", () => {
  it("is 21 for black on white and 1 for a colour on itself", () => {
    expect(ratio([0, 0, 0], [255, 255, 255])).toBeCloseTo(21, 5);
    expect(ratio([120, 30, 200], [120, 30, 200])).toBe(1);
  });

  it("agrees with the app's own check on a spread of pairs", () => {
    const colors: Rgb[] = [[0, 0, 0], [255, 255, 255], [17, 34, 51], [200, 80, 40], [90, 160, 250], [128, 128, 128], [250, 245, 230]];
    for (const a of colors) for (const b of colors) expect(ratio(a, b)).toBeCloseTo(contrastRatio(hex(a), hex(b)), 6);
  });

  it("composites by channel and reads the browser's colour spellings", () => {
    expect(composite([255, 0, 0], 0.5, [0, 0, 255])).toEqual([128, 0, 128]);
    expect(parseColor("rgb(1, 2, 3)")).toEqual({ rgb: [1, 2, 3], alpha: 1 });
    expect(parseColor("rgba(1, 2, 3, 0.5)")).toEqual({ rgb: [1, 2, 3], alpha: 0.5 });
    expect(parseColor("color(srgb 1 0 0.5 / 0.25)")).toEqual({ rgb: [255, 0, 128], alpha: 0.25 });
    expect(parseColor("transparent")).toBeNull();
  });
});
