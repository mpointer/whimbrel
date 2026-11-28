// What a skin image draws (richer skins row 8): header and badge only, a scrim under a header image that is not
// marked text-free, nothing for a bad asset name, and always decorative.

import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import { SkinImageView } from "./skinImage";

describe("drawing a skin image", () => {
  const header = { asset: "a.png", placement: "header" as const, textFree: false, scrim: 0.6 };
  const html = (entry: typeof header | undefined, url: string | null, placement: "header" | "badge" | "background" = "header") =>
    renderToStaticMarkup(<SkinImageView entry={entry} url={url} placement={placement} />);

  it("is decorative: empty alt text and hidden from Narrator", () => {
    const out = html(header, "blob:x");
    expect(out).toContain('aria-hidden="true"');
    expect(out).toContain('alt=""');
  });

  it("has a scrim over a header image, and none over a text-free one or a badge", () => {
    expect(html(header, "blob:x")).toContain("skin-image__scrim");
    expect(html({ ...header, textFree: true, scrim: 0 }, "blob:x")).not.toContain("skin-image__scrim");
    expect(html({ ...header, placement: "badge" as never, scrim: 0 }, "blob:x", "badge")).not.toContain("skin-image__scrim");
  });

  it("draws nothing for a bad asset name (no bytes came back), a missing entry or a background", () => {
    expect(html(header, null)).toBe("");
    expect(html(undefined, "blob:x")).toBe("");
    expect(html({ ...header, placement: "background" as never }, "blob:x", "background")).toBe("");
  });
});

