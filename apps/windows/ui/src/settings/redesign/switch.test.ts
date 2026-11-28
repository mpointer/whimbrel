import { describe, expect, it } from "vitest";
import { SETTINGS_SEARCH_ENTRIES } from "../settingsSearch";
import { stampAnchors } from "./RedesignedSettings";

describe("search anchors", () => {
  /** A small stand-in for a page: elements with an id and attributes, found by the two selectors the frame uses. */
  class Fake {
    attrs: Record<string, string> = {};
    constructor(public id: string | null) {
      if (id !== null) this.attrs.id = id;
    }
    getAttribute = (n: string) => this.attrs[n] ?? null;
    setAttribute = (n: string, v: string) => void (this.attrs[n] = v);
    removeAttribute = (n: string) => void delete this.attrs[n];
  }
  const page = (els: Fake[]) => ({
    querySelector: (sel: string) => {
      const id = /id="(.+)"/.exec(sel)![1];
      return els.find((e) => e.attrs.id === id) ?? null;
    },
    querySelectorAll: (sel: string) => (sel === "[data-settings-anchor]" ? els.filter((e) => "data-settings-anchor" in e.attrs) : []),
  });

  it("writes data-settings-anchor on an element whose id is a registry anchor, and only there", () => {
    const els = [new Fake("capture.localOnly"), new Fake("ai.keys"), new Fake("not-an-anchor")];
    stampAnchors(page(els) as unknown as ParentNode);
    expect(els[0].attrs["data-settings-anchor"]).toBe("capture.localOnly");
    expect(els[1].attrs["data-settings-anchor"]).toBe("ai.keys");
    expect("data-settings-anchor" in els[2].attrs).toBe(false);
    expect(SETTINGS_SEARCH_ENTRIES.some((e) => e.id === "not-an-anchor")).toBe(false);
  });

  it("removes a stamp from a node that is reused for another row (the plan rows after a state change)", () => {
    const node = new Fake("plan.manage");
    const root = page([node]) as unknown as ParentNode;
    stampAnchors(root);
    expect(node.attrs["data-settings-anchor"]).toBe("plan.manage");
    // The plan became an upgrade: the same node now has no id.
    delete node.attrs.id;
    stampAnchors(root);
    expect("data-settings-anchor" in node.attrs).toBe(false);
    // Reused for a different known anchor: the stamp follows the id.
    node.attrs.id = "plan.provider";
    node.attrs["data-settings-anchor"] = "plan.manage";
    stampAnchors(root);
    expect(node.attrs["data-settings-anchor"]).toBe("plan.provider");
  });
});
