import { describe, expect, it } from "vitest";
import css from "./system.css?raw";
import { MINIMUM_CONTRAST, contrastRatio, type ThemeToken } from "../themes";

// The infobox's "Next, in 9 min" label sits in .w-next-head, on the selection colour. The e2e matrix
// caught Classic missing 4.5:1 there with accentPrimary (4.19 on Default light). This reads the colour the
// rule names and checks it on every built-in palette, both variants.
const palettes = import.meta.glob(["../../../../../themes/*.json", "../../../../../skins/palettes/*.json"], {
  query: "?raw",
  import: "default",
  eager: true,
}) as Record<string, string>;

function headToken(): ThemeToken {
  const rule = /\.w-next-head\s*\{([^}]*)\}/.exec(css);
  if (!rule) throw new Error(".w-next-head rule is missing");
  const color = /(?:^|[;\s])color:\s*var\(--color-([a-z-]+)\)/.exec(rule[1]);
  if (!color) throw new Error(".w-next-head sets no token colour");
  return color[1].replace(/-([a-z])/g, (_, c: string) => c.toUpperCase()) as ThemeToken;
}

describe("the infobox head label", () => {
  it("is read from the built-in palettes", () => {
    expect(Object.keys(palettes).length).toBeGreaterThan(12);
  });

  it("reaches 4.5:1 on selection in every built-in palette, light and dark", () => {
    const token = headToken();
    const failures: string[] = [];
    for (const [path, text] of Object.entries(palettes)) {
      const file = JSON.parse(text) as Record<string, Record<string, string> | undefined>;
      for (const mode of ["light", "dark"]) {
        const v = file[mode];
        if (!v?.[token] || !v.selection) continue;
        const r = contrastRatio(v[token], v.selection);
        if (r < MINIMUM_CONTRAST) failures.push(`${path.split("/").pop()} ${mode} ${token} on selection ${r.toFixed(2)}`);
      }
    }
    expect(failures).toEqual([]);
  });
});
