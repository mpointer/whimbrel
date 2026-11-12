import { describe, expect, it } from "vitest";
import { asBytes } from "./api";

describe("asBytes", () => {
  const png = [0x89, 0x50, 0x4e, 0x47];

  it("keeps an ArrayBuffer's bytes", () => {
    expect(Array.from(asBytes(new Uint8Array(png).buffer))).toEqual(png);
  });

  it("turns a plain array of numbers into bytes, not their text", () => {
    const bytes = asBytes(png);
    expect(Array.from(bytes)).toEqual(png);
    // The bug it fixes: a Blob of the raw array held "137,80,78,71" (12 bytes), which no image decodes.
    expect(new Blob([bytes]).size).toBe(4);
    expect(new Blob([png as unknown as BlobPart]).size).toBe("137,80,78,71".length);
  });
});
