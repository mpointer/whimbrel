import { isValidElement, type ReactElement, type ReactNode } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";
import { StatusRows } from "./StatusSection";
import type { StatusRowSpec } from "./StatusSectionModel";

type Props = { label?: string; onPress?: () => void; description?: string; kind?: string; children?: ReactNode };

function find(node: ReactNode, match: (el: ReactElement<Props>) => boolean): ReactElement<Props>[] {
  if (Array.isArray(node)) return node.flatMap((n) => find(n, match));
  if (!isValidElement(node)) return [];
  const el = node as ReactElement<Props>;
  return [...(match(el) ? [el] : []), ...find(el.props.children, match)];
}

const rows: StatusRowSpec[] = [
  { key: "sync", title: "Sync", lines: ["Sync is off."], target: "devices" },
  { key: "plan", title: "Plan", lines: ["Free plan"], target: "plan" },
  { key: "ai", title: "AI", lines: ["Summaries use Ollama, llama3."], target: "ai" },
  { key: "storage", title: "Storage", lines: ["Meetings and recordings: Keep forever."], target: "capture" },
  { key: "updates", title: "Updates", lines: ["Whimbrel 1.4.0", "Up to date."], target: "general" },
  { key: "routines", title: "Routines", lines: ["Routines run on any device."], target: "automations" },
];

describe("the status rows, pressed", () => {
  it("each section row calls onSection with its section, and Routines opens the Automations tab", () => {
    const onSection = vi.fn();
    const onOpenAutomations = vi.fn();
    const tree = StatusRows({ rows, onSection, onOpenAutomations });
    const links = find(tree, (el) => typeof el.props.onPress === "function");
    expect(links.map((l) => l.props.label)).toEqual(["Sync", "Plan", "AI", "Storage", "Updates", "Routines"]);
    for (const l of links) l.props.onPress!();
    expect(onSection.mock.calls.map((c) => c[0])).toEqual(["devices", "plan", "ai", "capture", "general"]);
    expect(onOpenAutomations).toHaveBeenCalledTimes(1);
  });

  it("draws the lines as the row's description, in a headed card, and never says capture", () => {
    const html = renderToStaticMarkup(<StatusRows rows={rows} onSection={() => {}} onOpenAutomations={() => {}} />);
    expect(html).toContain("Meetings and recordings: Keep forever.");
    expect(html).toContain("Whimbrel 1.4.0. Up to date.");
    expect(html).toContain("<h3");
    expect(html.match(/<button/g)).toHaveLength(6);
    expect(html).not.toMatch(/capture/i);
  });
});
