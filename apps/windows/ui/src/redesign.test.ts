import { describe, expect, it } from "vitest";
import { primeRedesign, recheckRedesign, redesignEnabled, redesignKnown, useRedesign } from "./redesign";

describe("the redesign switch", () => {
  it("is on whatever the shell would answer, because nothing asks it", async () => {
    let asked = 0;
    const load = async () => {
      asked += 1;
      return false;
    };
    expect(await redesignEnabled(load)).toBe(true);
    expect(await primeRedesign(load)).toBe(true);
    expect(await recheckRedesign(load)).toBe(true);
    expect(asked).toBe(0);
  });

  it("is on when the shell cannot answer", async () => {
    const broken = async () => {
      throw new Error("unknown command");
    };
    expect(await redesignEnabled(broken)).toBe(true);
    expect(await primeRedesign(broken)).toBe(true);
    expect(await recheckRedesign(broken)).toBe(true);
  });

  it("is known before anything primes it, and the hook answers on at once", () => {
    expect(redesignKnown()).toBe(true);
    expect(useRedesign()).toBe(true);
  });

  it("main.tsx still waits for it, with the language, before it renders", async () => {
    const main = (await import("./main.tsx?raw")).default;
    expect(main).toMatch(/Promise\.all\(\[[\s\S]*primeRedesign\(\)[\s\S]*\]\)\.finally\(\(\) =>\s*createRoot/);
  });
});
