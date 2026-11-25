// Source marks in an answer, placed on the rendered markdown tree instead of on its text. A `[n]` whose number the core reported as cited
// by the answer (the page passes exactly that list, never every source) becomes a link to `#src-n` (the page draws that link as a source mark). Code, link text and anything the parser has already
// read as a link or a definition are left exactly as written. The markers are the ones the core reads: digits only, closed by `]`.

interface HastNode {
  type: string;
  value?: string;
  tagName?: string;
  properties?: Record<string, unknown>;
  children?: HastNode[];
}

/** Elements whose text is never a mark: code is code, and a link inside a link is not a link. */
const LEFT_ALONE = new Set(["code", "pre", "a", "kbd", "samp", "script", "style"]);

const MARKER = /\[(\d+)\]/g;

function marksIn(text: string, valid: ReadonlySet<number>): HastNode[] | null {
  const out: HastNode[] = [];
  let at = 0;
  let found = false;
  for (const m of text.matchAll(MARKER)) {
    const n = Number(m[1]);
    if (!valid.has(n)) continue;
    found = true;
    const start = m.index ?? 0;
    if (start > at) out.push({ type: "text", value: text.slice(at, start) });
    out.push({ type: "element", tagName: "a", properties: { href: `#src-${n}` }, children: [{ type: "text", value: String(n) }] });
    at = start + m[0].length;
  }
  if (!found) return null;
  if (at < text.length) out.push({ type: "text", value: text.slice(at) });
  return out;
}

function walk(node: HastNode, valid: ReadonlySet<number>): void {
  if (!node.children) return;
  if (node.type === "element" && node.tagName && LEFT_ALONE.has(node.tagName)) return;
  const next: HastNode[] = [];
  for (const child of node.children) {
    if (child.type === "text" && child.value) {
      const marks = marksIn(child.value, valid);
      if (marks) next.push(...marks);
      else next.push(child);
    } else {
      walk(child, valid);
      next.push(child);
    }
  }
  node.children = next;
}

/** A rehype plugin for react-markdown: `rehypePlugins={[[rehypeSourceMarks, validNumbers]]}`. */
export function rehypeSourceMarks(valid: ReadonlySet<number>) {
  return (tree: HastNode): void => walk(tree, valid);
}
