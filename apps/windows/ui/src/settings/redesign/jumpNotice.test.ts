import { describe, expect, it } from "vitest";
import { jumpNotice } from "./jumpNotice";

describe("jumpNotice", () => {
  it("says the row is shown, to a screen reader, when it was found", () => {
    expect(jumpNotice(true, "Manage subscription")).toEqual({ kind: "shown", announce: "Showing Manage subscription." });
  });

  it("says the setting is not shown when the row never appeared", () => {
    expect(jumpNotice(false, "Manage subscription")).toEqual({ kind: "hidden", hint: "That setting isn't shown right now." });
  });

  it("uses no em dash or en dash", () => {
    const all = [jumpNotice(true, "x"), jumpNotice(false, "x")].map((n) => JSON.stringify(n)).join("");
    expect(all).not.toMatch(/[–—]/);
  });
});
