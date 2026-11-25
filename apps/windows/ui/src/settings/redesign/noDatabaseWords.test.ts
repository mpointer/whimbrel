// The redesigned Settings calls the local database "your library". Every t() key written under settings/redesign is
// read from the source, so a sentence added later that says "memory database" or "database" fails here, in every
// language at once (a text replace at run time would only fix English).
import { readdirSync, readFileSync, statSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";

const root = __dirname;
const files: string[] = [];
const walk = (dir: string) => {
  for (const name of readdirSync(dir)) {
    const path = join(dir, name);
    if (statSync(path).isDirectory()) walk(path);
    else if (/\.(ts|tsx)$/.test(name) && !/\.test\./.test(name)) files.push(path);
  }
};
walk(root);

/** The literal first argument of every t("...") call in a file. */
function keys(source: string): string[] {
  const out: string[] = [];
  const re = /\bt\(\s*(["`])((?:\\.|(?!\1)[^\\])*)\1/g;
  let m: RegExpExecArray | null;
  while ((m = re.exec(source))) out.push(m[2].replace(/\\(.)/g, "$1"));
  return out;
}

describe("the redesigned Settings keys", () => {
  const all = files.flatMap((f) => keys(readFileSync(f, "utf8")).map((k) => ({ file: f.slice(root.length + 1), key: k })));

  it("finds the keys (so this test is not passing on nothing)", () => {
    expect(all.length).toBeGreaterThan(200);
    expect(all.some((k) => k.key === "Your library could not be opened.")).toBe(true);
  });

  it("never says \"memory database\", and never names the library \"database\"", () => {
    const bad = all.filter((k) => /\bdatabase\b/i.test(k.key));
    expect(bad).toEqual([]);
  });

  it("has no em or en dash in a key", () => {
    expect(all.filter((k) => /[–—]/.test(k.key))).toEqual([]);
  });
});
