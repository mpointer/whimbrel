// The recap on the redesigned Today, drawn as formatted text. The Daily Brief and the day's recap are written by a model and
// carry Markdown ("**Kite Practice**"), which the old Today shows raw. Model text is untrusted, so this is the same safe
// renderer the Ask answers use (react-markdown builds React elements and never emits raw HTML), narrowed further: only
// paragraphs, bold, italics, lists, headings, links and line breaks become elements; everything else (code, quotes, tables,
// rules, raw HTML such as "<script>") shows its words as plain text. Nothing in the recap is a control: the Rust open_external
// opens only a fixed list of addresses, so a link that called it was a button that did nothing. A link is drawn as its words,
// and for a web address (http or https) that differs from the words, the address after it in brackets as plain selectable text.
// Any other target (mailto:, javascript:, file:, data:, //host) shows the words only. An image shows its words and loads nothing.

import { Children, isValidElement, type ReactNode } from "react";
import ReactMarkdown from "react-markdown";

const ALLOWED = ["p", "strong", "em", "ul", "ol", "li", "h1", "h2", "h3", "h4", "h5", "h6", "a", "br", "img"];

/** The address a link may open: only http and https. Anything else (javascript:, file:, data:, a fragment) is not a link. */
export function safeLinkTarget(href: string | undefined): string | null {
  if (!href) return null;
  return /^https?:\/\/[^\s]+$/i.test(href) ? href : null;
}

/** The words of a link as one string, so they can be compared with its address. */
function plainText(node: ReactNode): string {
  return Children.toArray(node)
    .map((c) => (typeof c === "string" || typeof c === "number" ? String(c) : isValidElement(c) ? plainText((c.props as { children?: ReactNode }).children) : ""))
    .join("");
}

/** Every heading level draws as one heading under the section's own (h2), so the page outline stays in order. */
const heading = ({ children }: { children?: ReactNode }) => <h3 className="th-md-heading">{children}</h3>;

export function recapComponents() {
  return {
    a: ({ children, href }: { children?: ReactNode; href?: string }) => {
      const target = safeLinkTarget(href);
      if (target === null || target === plainText(children).trim()) return <span>{children}</span>;
      return (
        <span>
          {children} <span className="th-md-addr">({target})</span>
        </span>
      );
    },
    img: ({ alt }: { alt?: string }) => <span>{alt ?? ""}</span>,
    h1: heading,
    h2: heading,
    h3: heading,
    h4: heading,
    h5: heading,
    h6: heading,
  };
}

const components = recapComponents();

export function RecapMarkdown({ text }: { text: string }) {
  return (
    <div className="th-md">
      <ReactMarkdown allowedElements={ALLOWED} unwrapDisallowed components={components}>
        {text}
      </ReactMarkdown>
    </div>
  );
}
