import { describe, expect, it } from "vitest";
import { menuGroups, type NoteMenuItem } from "./NoteMenu";

const item = (id: string, more: Partial<NoteMenuItem> = {}): NoteMenuItem => ({ id, label: id, onPress: () => {}, ...more });

describe("the menu's groups", () => {
  it("groups the choices under a heading and keeps the items after them outside it", () => {
    const groups = menuGroups([item("regen"), item("again"), item("lang-a", { heading: "Transcript language" }), item("lang-b", { grouped: true }), item("trim"), item("forget")]);
    expect(groups.map((g) => [g.heading ?? null, g.items.map((i) => i.id)])).toEqual([
      [null, ["regen", "again"]],
      ["Transcript language", ["lang-a", "lang-b"]],
      [null, ["trim", "forget"]],
    ]);
  });
  it("has one group of loose items when nothing has a heading", () => {
    expect(menuGroups([item("a"), item("b")]).map((g) => g.items.length)).toEqual([2]);
    expect(menuGroups([])).toEqual([]);
  });
});
