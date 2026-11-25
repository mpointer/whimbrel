import { describe, expect, it } from "vitest";
import { isShortViewport, SHORT_VIEWPORT_PX } from "./shortViewport";

describe("a short main window", () => {
  it("is shorter than 480 CSS px", () => {
    expect(SHORT_VIEWPORT_PX).toBe(480);
    expect(isShortViewport(360)).toBe(true);
    expect(isShortViewport(479)).toBe(true);
    expect(isShortViewport(480)).toBe(false);
    expect(isShortViewport(720)).toBe(false);
  });
});
