// Under a Windows contrast theme the header's empty Search button (a disabled primary button) was dark words on a faded Highlight fill,
// nearly unreadable. In the redesign shell a disabled button is drawn in system colors: GrayText words and edge on the button face.

import { describe, expect, it } from "vitest";
import css from "./shell.css?raw";

describe("a disabled button under forced colors, in the redesign shell", () => {
  const at = css.indexOf("@media (forced-colors: active)");
  const block = at < 0 ? "" : css.slice(at);
  it("has a forced-colors rule for .w-shell .w-button:disabled", () => {
    expect(block).toContain(".w-shell .w-button:disabled");
    expect(block).toContain('.w-shell .w-button[aria-disabled="true"]');
  });
  it("draws it in system colors only: ButtonFace, GrayText, no fade", () => {
    expect(block).toMatch(/background:\s*ButtonFace/);
    expect(block).toMatch(/color:\s*GrayText/);
    expect(block).toMatch(/border-color:\s*GrayText/);
    expect(block).toMatch(/opacity:\s*1/);
    expect(block).not.toMatch(/#[0-9a-f]{3,8}|rgb|hsl/i);
  });
});
