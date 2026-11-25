// The redesigned Today draws the recap as formatted text, not raw Markdown ("**Kite Practice**"). Model text is untrusted:
// bold, italics, lists and headings become elements; everything else stays plain text; a link opens only through openExternal.

import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";
import { Tree, text } from "../../testkit/tree";
import { openExternal } from "../../api";
import { RecapMarkdown, safeLinkTarget } from "./RecapMarkdown";

vi.mock("../../api", () => ({ openExternal: vi.fn() }));

const FIXTURE = [
  "Today you worked on **Kite Practice** and the *Airship Club Project*.",
  "",
  "## What happened",
  "",
  "- Booked the field for **Saturday**",
  "- Sent the parts list",
  "",
  "1. First thing",
  "2. Second thing",
  "",
  "Read [the field rules](https://example.com/rules) before you go.",
  "",
  "A raw <script>alert(1)</script> string and a ![tracker](https://example.com/pixel.png) image.",
].join("\n");

const html = renderToStaticMarkup(<RecapMarkdown text={FIXTURE} />);

describe("the recap as formatted text", () => {
  it("shows bold and italics as elements, with no asterisks left", () => {
    expect(html).toContain("<strong>Kite Practice</strong>");
    expect(html).toContain("<em>Airship Club Project</em>");
    expect(html).not.toContain("**");
    expect(html).not.toMatch(/\*Airship/);
  });

  it("shows lists as lists and a heading below the section heading", () => {
    expect(html).toMatch(/<ul[^>]*>\s*<li>Booked the field for <strong>Saturday<\/strong><\/li>/);
    expect(html).toMatch(/<ol[^>]*>\s*<li>First thing<\/li>/);
    expect(html).toMatch(/<h3[^>]*>What happened<\/h3>/);
    expect(html).not.toContain("<h1");
    expect(html).not.toContain("<h2");
  });

  it("shows a raw <script> string as text, never as an element", () => {
    expect(html).not.toContain("<script");
    expect(html).toContain("&lt;script&gt;");
  });

  it("draws a link as plain text with its address after it: nothing navigates the window or loads from the network", () => {
    expect(html).not.toContain("href=");
    expect(html).not.toContain("<img");
    expect(html).not.toContain("<button");
    expect(html).not.toContain("example.com/pixel");
    expect(html).toContain('the field rules <span class="th-md-addr">(https://example.com/rules)</span>');
  });

  it("leaves a link that is not http or https as plain text", () => {
    for (const href of ["javascript:alert(1)", "file:///C:/secret", "data:text/html,x", "#top", "", undefined]) {
      expect(safeLinkTarget(href as string | undefined), String(href)).toBeNull();
    }
    expect(safeLinkTarget("https://example.com/a")).toBe("https://example.com/a");
    expect(safeLinkTarget("http://example.com")).toBe("http://example.com");
  });

  it("keeps everything else plain: code, quotes, tables and rules show their words, not elements", () => {
    const out = renderToStaticMarkup(<RecapMarkdown text={"> quoted\n\n`code`\n\n---\n\n| a | b |\n|---|---|\n| 1 | 2 |"} />);
    for (const tag of ["<blockquote", "<code", "<hr", "<table", "<pre"]) expect(out, tag).not.toContain(tag);
    expect(out).toContain("quoted");
    expect(out).toContain("code");
  });

  it("draws plain text unchanged, line breaks included", () => {
    expect(renderToStaticMarkup(<RecapMarkdown text={"One line.\nAnother line."} />)).toContain("One line.");
  });
});

// Nothing in the recap is a control. The Rust open_external opens only a fixed list of addresses, so a recap link that called it was
// a button that did nothing. A link is its words, then the web address in brackets as plain selectable text.
describe("the recap draws no control and opens nothing", () => {
  const draw = (md: string) => new Tree(() => <RecapMarkdown text={md} />).render();
  const cases: [string, string, string, string?][] = [
    ["an http link", "[the rules](http://example.com/rules)", "the rules (http://example.com/rules)"],
    ["an https link", "[the rules](https://example.com/rules)", "the rules (https://example.com/rules)"],
    ["a mixed-case scheme", "[the rules](HTTP://Example.com/rules)", "the rules (HTTP://Example.com/rules)"],
    ["a link whose text is its address", "[https://example.com](https://example.com)", "https://example.com"],
    ["an href with a leading space", "[the rules]( https://example.com/rules)", "the rules"],
    ["a mailto link", "[mail me](mailto:a@example.com)", "mail me", "a@example"],
    ["a javascript link", "[click](javascript:alert(1))", "click", "alert(1))"],
    ["a file link", "[open](file:///C:/secret.txt)", "open", "secret"],
    ["a data link", "[open](data:text/html,x)", "open", "text/html"],
    ["a protocol-relative link", "[host](//evil.example/x)", "host", "evil.example"],
    ["an image", "![a pixel](https://example.com/pixel.png)", "a pixel"],
    ["a reference-style image", "![tracker][t]\n\n[t]: https://example.com/pixel.png", "tracker"],
    ["a reference-style link", "[the rules][r]\n\n[r]: https://example.com/rules", "the rules (https://example.com/rules)"],
    ["a raw script", "before <script>alert(1)</script> after", "before <script>alert(1)</script> after"],
    ["a raw anchor", 'see <a href="https://example.com/x">here</a> now', 'see <a href="https://example.com/x">here</a> now'],
    ["a raw image", 'see <img src="https://example.com/p.png" alt="p"> now', 'see <img src="https://example.com/p.png" alt="p"> now'],
  ];
  for (const [what, md, words, absent] of cases) {
    it(`${what}: no link, button or image, and nothing opened`, () => {
      vi.mocked(openExternal).mockClear();
      const tree = draw(md);
      const out = renderToStaticMarkup(<RecapMarkdown text={md} />);
      for (const tag of ["<a ", "<a>", "<button", "<img", "<script"]) expect(out, tag).not.toContain(tag);
      expect(out).not.toMatch(/\s(href|src)="/);
      expect(tree.all((n) => ["a", "button", "img", "script"].includes(n.tag))).toHaveLength(0);
      expect(tree.all((n) => typeof n.props.onClick === "function" || typeof n.props.onKeyDown === "function")).toHaveLength(0);
      expect(tree.all((n) => n.props.tabIndex !== undefined)).toHaveLength(0);
      expect(openExternal).not.toHaveBeenCalled();
      if (absent) expect(tree.text()).not.toContain(absent);
      expect(text({ tag: "div", props: {}, children: tree.root }).replace(/\s+/g, " ").trim()).toContain(words);
    });
  }

  it("shows the address only for a web link that differs from its text", () => {
    const t = (md: string) => draw(md).text();
    expect(t("[https://example.com](https://example.com)")).not.toContain("(");
    expect(t("[mail](mailto:a@example.com)")).not.toContain("example.com");
    expect(t("[x](javascript:alert(1))")).not.toContain("alert");
    expect(t("[x](//evil.example/x)")).not.toContain("evil");
  });
});
