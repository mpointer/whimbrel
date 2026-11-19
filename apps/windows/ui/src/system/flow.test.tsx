import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import { WFlowPage, WMoreMenu, WNotice, WNumberedList, WProgressLine, WTextField, flowKeyAction } from ".";

const html = (node: React.ReactNode) => renderToStaticMarkup(<>{node}</>);
const target = (tag: string, type?: string, role?: string) => ({ tag, type, role });

describe("the keys of a flow page", () => {
  it("Return does the primary action from a field, the title and the page", () => {
    for (const tag of ["input", "h1", "div", "main", "label"]) expect(flowKeyAction({ key: "Enter", target: target(tag, tag === "input" ? "password" : undefined) }), tag).toBe("primary");
  });

  it("Return is left alone on buttons, links, checkboxes, radios and menu items, so they act as themselves", () => {
    expect(flowKeyAction({ key: "Enter", target: target("button") })).toBeNull();
    expect(flowKeyAction({ key: "Enter", target: target("a") })).toBeNull();
    expect(flowKeyAction({ key: "Enter", target: target("input", "checkbox") })).toBeNull();
    expect(flowKeyAction({ key: "Enter", target: target("button", undefined, "radio") })).toBeNull();
    expect(flowKeyAction({ key: "Enter", target: target("button", undefined, "menuitem") })).toBeNull();
  });

  it("in a text area Return is a new line, and Ctrl or Cmd with it is the primary action", () => {
    expect(flowKeyAction({ key: "Enter", target: target("textarea") })).toBeNull();
    expect(flowKeyAction({ key: "Enter", ctrlKey: true, target: target("textarea") })).toBe("primary");
    expect(flowKeyAction({ key: "Enter", metaKey: true, target: target("textarea") })).toBe("primary");
  });

  it("Escape goes back", () => {
    expect(flowKeyAction({ key: "Escape", target: target("input", "password") })).toBe("back");
    expect(flowKeyAction({ key: "Escape", target: target("div") })).toBe("back");
  });

  it("nothing happens inside a dialog, while composing, after a field took the key, or for other keys", () => {
    expect(flowKeyAction({ key: "Enter", target: { tag: "div", inDialog: true } })).toBeNull();
    expect(flowKeyAction({ key: "Escape", target: { tag: "div", inDialog: true } })).toBeNull();
    expect(flowKeyAction({ key: "Enter", composing: true, target: target("input") })).toBeNull();
    expect(flowKeyAction({ key: "Enter", defaultPrevented: true, target: target("input") })).toBeNull();
    expect(flowKeyAction({ key: "a", target: target("input") })).toBeNull();
  });
});

describe("a flow page", () => {
  it("draws the step line, the title, the paragraph, the work, the actions and the fine print in that order", () => {
    const out = html(
      <WFlowPage step={{ current: 2, total: 4 }} title="TITLE" intro={<p>INTRO</p>} actions={<button>ACTIONS</button>} finePrint={<p>FINE</p>}>
        <p>WORK</p>
      </WFlowPage>,
    );
    const order = ["Step 2 of 4", "TITLE", "INTRO", "WORK", "ACTIONS", "FINE"].map((s) => out.indexOf(s));
    expect(order.every((i) => i >= 0)).toBe(true);
    expect([...order].sort((a, b) => a - b)).toEqual(order);
  });

  it("announces the step line, and has no step line for a one-step flow", () => {
    expect(html(<WFlowPage step={{ current: 1, total: 4 }} title="T" />)).toMatch(/<p class="w-flow-step" role="status">Step 1 of 4<\/p>/);
    expect(html(<WFlowPage title="T" />)).not.toContain("Step ");
  });

  it("the title can take focus when a step appears", () => {
    expect(html(<WFlowPage title="T" />)).toMatch(/<h1 class="w-flow-title" tabindex="-1">T<\/h1>/);
  });

  it("shows a quiet bar and says what is going on while working", () => {
    const out = html(<WFlowPage title="T" busyLabel="Checking…" />);
    expect(out).toContain("w-quiet-bar");
    expect(out).toContain('aria-busy="true"');
    expect(out).toContain("Checking…");
    expect(html(<WFlowPage title="T" />)).not.toContain("w-quiet-bar");
  });
});

describe("a text field", () => {
  const field = (props: Partial<React.ComponentProps<typeof WTextField>> = {}) => html(<WTextField label="Passphrase" value="" onChange={() => {}} {...props} />);

  it("has a visible label tied to its input", () => {
    const out = field();
    const id = out.match(/<input[^>]* id="([^"]+)"/)![1];
    expect(out).toContain(`<label class="w-field-label" for="${id}">Passphrase</label>`);
  });

  it("states the rule before typing and ties the help, the count and the error to the field", () => {
    const out = field({ help: "At least 12 characters.", status: { text: "0 of at least 12 characters" }, error: "Use at least 12 characters." });
    const described = out.match(/aria-describedby="([^"]+)"/)![1].split(" ");
    expect(described).toHaveLength(3);
    for (const id of described) expect(out).toContain(`id="${id}"`);
    expect(out.indexOf("At least 12 characters.")).toBeLessThan(out.indexOf("<input"));
    // The error is under the field, and the field is marked invalid.
    expect(out.indexOf("Use at least 12 characters.")).toBeGreaterThan(out.indexOf("<input"));
    expect(out).toContain('aria-invalid="true"');
  });

  it("has no invalid mark and no error text when nothing is wrong, but keeps its live region", () => {
    const out = field();
    expect(out).not.toContain("aria-invalid");
    expect(out).not.toContain("w-field-error");
    expect(out).toContain('aria-live="polite"');
  });

  it("is a text area in the mono face when it has rows", () => {
    expect(field({ rows: 6 })).toMatch(/<textarea class="w-input w-input-mono" rows="6"/);
  });
});

describe("notices and progress", () => {
  it("a notice has a glyph and a box as well as a color, and an error is an alert", () => {
    for (const tone of ["info", "ok", "warning", "error"] as const) {
      const out = html(<WNotice tone={tone}>Words</WNotice>);
      expect(out).toContain("<svg");
      expect(out).toContain(`w-notice-${tone}`);
      expect(out).toContain(tone === "error" ? 'role="alert"' : 'role="status"');
    }
  });

  it("a progress line is a named progress bar with its percent, and never leaves 0 to 100", () => {
    const out = html(<WProgressLine label="Encrypting" value={0.42} />);
    expect(out).toContain('role="progressbar"');
    expect(out).toContain('aria-label="Encrypting"');
    expect(out).toContain('aria-valuenow="42"');
    expect(out).toContain("42%");
    expect(html(<WProgressLine label="x" value={7} />)).toContain('aria-valuenow="100"');
    expect(html(<WProgressLine label="x" value={Number.NaN} />)).toContain('aria-valuenow="0"');
    expect(html(<WProgressLine label="x" value={0.5} labelHidden />)).toContain('class="w-visually-hidden">x');
  });
});

describe("the numbered list", () => {
  const words = Array.from({ length: 24 }, (_, i) => `word${i + 1}`);

  it("is a named list numbered 1. to 24., three columns of eight, for a screen reader to read one by one", () => {
    const out = html(<WNumberedList items={words} label="Your 24 recovery words" />);
    expect(out).toContain('role="list"');
    expect(out).toContain('aria-label="Your 24 recovery words"');
    expect(out.match(/<li /g)).toHaveLength(24);
    expect(out).toContain(">1.</span><span>word1</span>");
    expect(out).toContain(">24.</span><span>word24</span>");
    expect(out).toContain("grid-template-rows:repeat(8, auto)");
  });
});

describe("more options", () => {
  it("is one button, closed", () => {
    const closed = html(<WMoreMenu items={[{ key: "a", label: "Put it back", onPress: () => {} }]} />);
    expect(closed).toContain("More options");
    expect(closed).toContain('aria-haspopup="menu"');
    expect(closed).toContain('aria-expanded="false"');
    expect(closed).not.toContain('role="menu"');
  });
});
