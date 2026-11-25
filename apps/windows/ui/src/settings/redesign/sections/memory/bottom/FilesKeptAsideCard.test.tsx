import { beforeEach, describe, expect, it, vi } from "vitest";
import { Tree, text, type TNode } from "../../../../../testkit/tree";

vi.mock("react", async (actual) => (await import("../../../../../testkit/tree")).withStubbedHooks(await actual<typeof import("react")>()));
vi.mock("../../../../../l10n", async (actual) => {
  const real = await actual<typeof import("../../../../../l10n")>();
  return { ...real, useL10n: () => ({ t: real.t, locale: "en", lang: "en" }) };
});
const h = vi.hoisted(() => ({ calls: [] as [string, unknown][], view: { count: 3, bytes: 12_000_000 } as { count: number; bytes: number } }));
vi.mock("@tauri-apps/api/core", () => ({
  invoke: async (cmd: string, args?: unknown) => {
    h.calls.push([cmd, args ?? null]);
    if (cmd === "foreign_files_list") return h.view;
    if (cmd === "foreign_files_delete") {
      h.view = { count: 0, bytes: 0 };
      return { removed: 3, removedBytes: 12_000_000, failed: [] };
    }
    return null;
  },
}));
vi.stubGlobal("document", { activeElement: null, getElementById: () => null });
const { FilesKeptAsideCard } = await import("./FilesKeptAsideCard");
const click = (b: TNode) => (b.props.onClick as () => void)();
const deletes = () => h.calls.filter(([c]) => c === "foreign_files_delete");

beforeEach(() => {
  h.calls = [];
  h.view = { count: 3, bytes: 12_000_000 };
});

describe("Files kept aside", () => {
  it("is not drawn when no files are kept aside", async () => {
    h.view = { count: 0, bytes: 0 };
    const tree = new Tree(() => <FilesKeptAsideCard />).render();
    await tree.settle();
    expect(tree.root).toEqual([]);
  });

  it("deletes only after a confirm that says the count and size, and passes the confirmed count", async () => {
    const tree = new Tree(() => <FilesKeptAsideCard />).render();
    await tree.settle();
    expect(tree.one((n) => n.tag === "section" && n.props.id === "capture.foreignFiles").tag).toBe("section");
    expect(tree.has("3 files kept aside, 12.0 MB")).toBe(true);
    click(tree.button("Delete these files"));
    tree.render();
    expect(deletes()).toEqual([]);
    const dialog = tree.one((n) => n.props.role === "alertdialog");
    expect(text(dialog)).toContain("This deletes 3 files kept aside (12.0 MB). They cannot be recovered.");
    expect(tree.button("Cancel").props["data-initial"]).toBe(true);
    const confirm = tree.all((n) => n.tag === "button" && text(n).trim() === "Delete these files").pop()!;
    click(confirm);
    await tree.settle();
    expect(deletes()).toEqual([["foreign_files_delete", { confirmed: 3 }]]);
    expect(tree.has("Deleted 3 files kept aside (12.0 MB).")).toBe(true);
  });

  it("Cancel deletes nothing", async () => {
    const tree = new Tree(() => <FilesKeptAsideCard />).render();
    await tree.settle();
    click(tree.button("Delete these files"));
    tree.render();
    click(tree.button("Cancel"));
    tree.render();
    expect(deletes()).toEqual([]);
    expect(tree.all((n) => n.props.role === "alertdialog")).toEqual([]);
  });
});
