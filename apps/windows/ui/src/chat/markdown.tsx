import type { ReactNode } from "react";

/**
 * Answers are model output, so they are untrusted. react-markdown builds React elements
 * and never emits raw HTML. Links and images render as text, so nothing in an answer can
 * navigate the app window or load a remote resource.
 */
export const markdownComponents = {
  a: ({ children, href }: { children?: ReactNode; href?: string }) => (
    <span className="md-link" title={href}>
      {children}
    </span>
  ),
  img: ({ alt }: { alt?: string }) => <span>{alt ?? ""}</span>,
};
