import { describe, expect, it } from "vitest";
import { primeRedesign, redesignEnabled, redesignKnown } from "./redesign";

describe("the hidden redesign switch", () => {
  it("is off unless the shell answers a real true", async () => {
    expect(await redesignEnabled(async () => false)).toBe(false);
    expect(await redesignEnabled(async () => true)).toBe(true);
    for (const answer of [null, undefined, 1, "true", "yes", {}, []]) expect(await redesignEnabled(async () => answer), String(answer)).toBe(false);
  });

  it("is off when the shell cannot answer", async () => {
    expect(
      await redesignEnabled(async () => {
        throw new Error("unknown command");
      }),
    ).toBe(false);
  });
});

describe("the answer kept before the first render", () => {
  it("is unknown until the shell has been asked, then true only for a real true", async () => {
    expect(redesignKnown()).toBeNull();
    expect(await primeRedesign(async () => true)).toBe(true);
    expect(redesignKnown()).toBe(true);
    expect(await primeRedesign(async () => "yes")).toBe(false);
    expect(redesignKnown()).toBe(false);
  });

  it("is false when the shell cannot answer, so the old look is what draws", async () => {
    expect(
      await primeRedesign(async () => {
        throw new Error("unknown command");
      }),
    ).toBe(false);
    expect(redesignKnown()).toBe(false);
  });

  it("main.tsx waits for it, with the language, before it renders", async () => {
    const main = (await import("./main.tsx?raw")).default;
    expect(main).toMatch(/Promise\.all\(\[[\s\S]*primeRedesign\(\)[\s\S]*\]\)\.finally\(\(\) =>\s*createRoot/);
  });
});
