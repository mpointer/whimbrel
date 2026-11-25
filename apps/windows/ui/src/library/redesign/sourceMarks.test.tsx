import { renderToStaticMarkup } from "react-dom/server";
import ReactMarkdown from "react-markdown";
import { describe, expect, it } from "vitest";
import { rehypeSourceMarks } from "./sourceMarks";

/** The answer as the page draws it: every `[n]` whose n is in the cited list becomes <mark data-n="n">, nothing else changes. */
const draw = (text: string, valid: number[] = [1, 2, 3]) =>
  renderToStaticMarkup(
    <ReactMarkdown
      rehypePlugins={[[rehypeSourceMarks, new Set(valid)]]}
      components={{ a: ({ href, children }) => (/^#src-(\d+)$/.test(href ?? "") ? <mark data-n={/\d+/.exec(href ?? "")![0]}>{children}</mark> : <a href={href}>{children}</a>) }}
    >
      {text}
    </ReactMarkdown>,
  );

describe("source marks in a drawn answer", () => {
  it("marks a source in running text, a list item and a heading", () => {
    expect(draw("Sam wants a tier [1].")).toContain('<mark data-n="1">1</mark>');
    const list = draw("- first [1]\n- second [2]");
    expect(list).toContain('<li>first <mark data-n="1">1</mark></li>');
    expect(list).toContain('<mark data-n="2">2</mark>');
    expect(draw("## Pricing [3]")).toContain('<mark data-n="3">3</mark>');
  });
  it("marks adjacent markers separately", () => {
    const out = draw("See [1][2].");
    expect(out).toContain('<mark data-n="1">1</mark><mark data-n="2">2</mark>');
  });
  it("leaves a number that is not a source as the text the model wrote", () => {
    const out = draw("Annual discount [7].");
    expect(out).toContain("[7]");
    expect(out).not.toContain("<mark");
  });
  it("leaves inline code alone", () => {
    const out = draw("Use `a[1]` to read it [2].");
    expect(out).toContain("<code>a[1]</code>");
    expect(out).toContain('<mark data-n="2">2</mark>');
    expect(out).not.toContain('data-n="1"');
  });
  it("leaves a fenced block alone, whatever it holds", () => {
    const out = draw("```js\nconst x = a[1];\nconst y = b[2];\n```\n\nThen [3].");
    expect(out).toContain("const x = a[1];");
    expect(out).toContain("const y = b[2];");
    expect(out).toContain('<mark data-n="3">3</mark>');
    expect(out.match(/<mark/g)).toHaveLength(1);
  });
  it("does not turn the text of a link into a mark, or nest one inside it", () => {
    const out = draw("Read [the plan [1]](https://example.test/plan) and then [2].");
    expect(out).toContain('<a href="https://example.test/plan">the plan [1]</a>');
    expect(out).toContain('<mark data-n="2">2</mark>');
    expect(out.match(/<mark/g)).toHaveLength(1);
  });
  it("leaves a reference line alone", () => {
    const out = draw("Sam said so [2].\n\n[1]: https://example.test/ref");
    expect(out).toContain('<mark data-n="2">2</mark>');
    expect(out).not.toContain('data-n="1"');
  });
  it("reads only digits closed by a bracket, as the core does", () => {
    const out = draw("[a] [1 ] [ 1] [1,2] [01]", [1]);
    expect(out).toContain("[a] [1 ] [ 1] [1,2]");
    expect(out).toContain('<mark data-n="1">1</mark>');
  });
});

describe("marks only for what the core says the answer cites", () => {
  it("draws no mark for any number when nothing is cited", () => {
    for (const text of ["See [1].", "See [1][2].", "A \\[1\\] and [1](http://x.test)", "[01]"]) {
      expect(draw(text, [])).not.toContain("<mark");
    }
    expect(draw("See [1].", [])).toContain("[1]");
  });
  it("draws a mark for a cited number, and leaves a source that is not cited as text", () => {
    const out = draw("One [1] and two [2].", [2]);
    expect(out).toContain("[1]");
    expect(out).toContain('<mark data-n="2">2</mark>');
    expect(out.match(/<mark/g)).toHaveLength(1);
  });
  it("marks adjacent cited markers separately, and only the cited one of a pair", () => {
    expect(draw("[1][2]", [1, 2])).toContain('<mark data-n="1">1</mark><mark data-n="2">2</mark>');
    const half = draw("[1][2]", [2]);
    expect(half).toContain('[1]<mark data-n="2">2</mark>');
  });
  it("leaves a link as a link, whatever is cited", () => {
    const out = draw("Read [1](http://x.test) now [2].", [1, 2]);
    expect(out).toContain('<a href="http://x.test">1</a>');
    expect(out).not.toContain('data-n="1"');
    expect(out).toContain('<mark data-n="2">2</mark>');
  });
  it("leaves inline and fenced code as written even when the number is cited", () => {
    const out = draw("Use `a[1]` here [2].\n\n```\nb[1]\n```", [1, 2]);
    expect(out).toContain("<code>a[1]</code>");
    expect(out).toContain("b[1]");
    expect(out.match(/<mark/g)).toHaveLength(1);
  });
  it("leaves a reference definition out of the marks", () => {
    const out = draw("Said so [2].\n\n[1]: https://example.test/ref", [1, 2]);
    expect(out).not.toContain('data-n="1"');
    expect(out).toContain('<mark data-n="2">2</mark>');
  });
  it("reads [01] as the number the core reads it as, and only when that number is cited", () => {
    expect(draw("[01]", [1])).toContain('<mark data-n="1">1</mark>');
    expect(draw("[01]", [2])).toContain("[01]");
  });
  it("leaves an escaped bracket pair alone when its number is not cited", () => {
    const out = draw("literal \\[1\\] here [2]", [2]);
    expect(out).toContain("[1]");
    expect(out).not.toContain('data-n="1"');
  });
});
