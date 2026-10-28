import { describe, expect, it } from "vitest";
import { APPEARANCES, cssVariable, parseAppearance, resolveMode } from "./theme";
import { TOKENS } from "./themes";

const tokensCss = (await import("./tokens.css?raw")).default;

describe("appearance", () => {
  it("accepts light and dark and treats anything else as System", () => {
    expect(parseAppearance("light")).toBe("light");
    expect(parseAppearance("dark")).toBe("dark");
    for (const v of ["system", "Dark", "", null, undefined, 3, {}]) expect(parseAppearance(v)).toBe("system");
    expect(APPEARANCES).toEqual(["system", "light", "dark"]);
  });

  it("resolves System from the OS and the others to themselves", () => {
    expect(resolveMode("system", true)).toBe("dark");
    expect(resolveMode("system", false)).toBe("light");
    expect(resolveMode("light", true)).toBe("light");
    expect(resolveMode("dark", false)).toBe("dark");
  });
});

describe("custom property names", () => {
  it("turns a token name into its tokens.css property", () => {
    expect(cssVariable("backgroundPage")).toBe("--color-background-page");
    expect(cssVariable("textOnAccent")).toBe("--color-text-on-accent");
    expect(cssVariable("link")).toBe("--color-link");
  });

  it("names a property tokens.css defines, for every token, so a theme can restyle all of it", () => {
    for (const token of TOKENS) expect(tokensCss, token).toContain(`${cssVariable(token)}:`);
  });
});
