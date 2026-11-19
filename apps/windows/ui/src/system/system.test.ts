import { execFileSync } from "node:child_process";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";

// Guards for the design system folder: colors only from tokens, every --w-* variable defined, generated file current.

const generated = (await import("./tokens.generated.css?raw")).default;
const files = import.meta.glob("./**/*.{ts,tsx,css}", { query: "?raw", import: "default", eager: true }) as Record<string, string>;
const own = Object.entries(files).filter(([f]) => !f.endsWith("tokens.generated.css") && !/\.test\.tsx?$/.test(f));

describe("the system folder", () => {
  it("finds what it guards", () => {
    expect(own.length).toBeGreaterThan(8);
    expect(own.some(([f]) => f.endsWith("system.css"))).toBe(true);
  });

  it("uses no hex, rgb(), hsl() or color-mix() outside the generated file", () => {
    const RAW = /#(?:[0-9a-f]{8}|[0-9a-f]{6}|[0-9a-f]{4}|[0-9a-f]{3})\b|\b(?:rgba?|hsla?|hwb|lab|lch|oklab|oklch)\(|\bcolor-mix\(/i;
    const offenders: string[] = [];
    for (const [file, text] of own) text.split(/\r?\n/).forEach((l, i) => RAW.test(l) && offenders.push(`${file}:${i + 1}: ${l.trim()}`));
    expect(offenders).toEqual([]);
  });

  it("uses no named color keyword other than system colors, transparent and currentColor", () => {
    const NAMED = /:\s*(?:white|black|red|green|blue|gray|grey|orange|yellow|purple|teal|navy)\b/i;
    expect(own.flatMap(([f, text]) => text.split("\n").filter((l) => NAMED.test(l)).map((l) => `${f}: ${l.trim()}`))).toEqual([]);
  });

  it("reads only --w-* variables the generated file defines, and declares none of its own", () => {
    const defined = new Set([...generated.matchAll(/(--w-[a-z0-9-]+)\s*:/g)].map((m) => m[1]));
    const undefinedUses: string[] = [];
    const declared: string[] = [];
    for (const [file, text] of own) {
      for (const m of text.matchAll(/var\((--w-[a-z0-9-]+)/g)) if (!defined.has(m[1])) undefinedUses.push(`${file}: ${m[1]}`);
      for (const m of text.matchAll(/(--w-[a-z0-9-]+)\s*:/g)) declared.push(`${file}: ${m[1]}`);
    }
    expect(undefinedUses).toEqual([]);
    expect(declared).toEqual([]);
  });

  it("reads only --color-* variables the app defines", () => {
    const tokens = readFileSync(new URL("../tokens.css", import.meta.url), "utf8");
    const defined = new Set([...tokens.matchAll(/(--color-[a-z-]+)\s*:/g)].map((m) => m[1]));
    const bad = own.flatMap(([f, text]) => [...text.matchAll(/var\((--color-[a-z-]+)/g)].filter((m) => !defined.has(m[1])).map((m) => `${f}: ${m[1]}`));
    expect(bad).toEqual([]);
  });

  it("takes shape and fonts from the skin variables first", () => {
    const css = own.find(([f]) => f.endsWith("system.css"))![1];
    for (const v of ["--radius-control", "--radius-card", "--radius-surface", "--font-body", "--font-display", "--border-width", "--density"]) {
      expect(css, v).toContain(`var(${v},`);
    }
    // No literal px radius or font-family in a component rule.
    expect(css.replace(/\/\*[\s\S]*?\*\//g, "")).not.toMatch(/border-radius:\s*\d+px/);
    expect(css).not.toMatch(/font-family:\s*["A-Z]/);
  });

  it("has a generated tokens file that is current", () => {
    // fileURLToPath, not .pathname: on Windows the pathname is "/D:/...", which is not a folder the check can run in.
    const root = fileURLToPath(new URL("../../../../../", import.meta.url));
    expect(() => execFileSync(process.execPath, ["design/tokens/build.mjs", "--check"], { cwd: root, stdio: "pipe" })).not.toThrow();
  });

  it("keeps the gallery out of the app's entry", () => {
    const main = readFileSync(new URL("../main.tsx", import.meta.url), "utf8");
    expect(main).not.toMatch(/system|gallery/i);
  });
});
