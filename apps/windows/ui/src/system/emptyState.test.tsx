// WEmptyState: the optional props that let the redesigned Meetings and Topics empty states keep what the old ones had (the Import button
// in place and disabled while importing; the primary Open Topics settings button; the title a heading). The defaults are today's
// look, so no other screen changes.

import { describe, expect, it } from "vitest";
import { Tree } from "../testkit/tree";
import { WEmptyState } from "./layout";
import meetings from "../meetings/MeetingsView.tsx?raw";
import topics from "../topics/TopicsView.tsx?raw";

const draw = (props: Partial<Parameters<typeof WEmptyState>[0]> = {}) => new Tree(() => <WEmptyState title="Nothing here" actionLabel="Do it" onAction={() => {}} {...props} />).render();

describe("WEmptyState defaults", () => {
  it("draws the title as a paragraph and the action as an enabled secondary button", () => {
    const t = draw();
    expect(t.all((n) => n.tag === "p" && n.props.className === "w-state-title")).toHaveLength(1);
    expect(t.all((n) => /^h[1-6]$/.test(n.tag))).toHaveLength(0);
    const b = t.button("Do it");
    expect(b.props.className).toBe("w-button w-button-secondary");
    expect(b.props.disabled).toBeFalsy();
  });
});

describe("WEmptyState options", () => {
  it("can disable the action while keeping it on screen", () => {
    expect(draw({ actionDisabled: true }).button("Do it").props.disabled).toBe(true);
  });
  it("can make the action the primary button", () => {
    expect(draw({ actionKind: "primary" }).button("Do it").props.className).toBe("w-button w-button-primary");
  });
  it("can draw the title as a heading", () => {
    const t = draw({ titleLevel: 2 });
    expect(t.all((n) => n.tag === "h2" && n.props.className === "w-state-title")).toHaveLength(1);
  });
  it("still draws no button without an action", () => {
    expect(draw({ onAction: undefined }).all((n) => n.tag === "button")).toHaveLength(0);
  });
});

describe("the redesigned empty states keep what the old ones had", () => {
  it("Meetings: the Import Transcripts button stays and is disabled while importing, never removed", () => {
    expect(meetings).toMatch(/actionLabel=\{t\("Import Transcripts…"\)\}\s+actionDisabled=\{importing\}\s+onAction=\{\(\) => runImport\(importTranscripts\)\}/);
    expect(meetings).not.toMatch(/actionLabel=\{importing \?/);
  });
  it("Topics: Open Topics settings is the primary button and No topics yet is an h2", () => {
    expect(topics).toMatch(/actionLabel=\{t\("Open Topics settings"\)\}\s+actionKind="primary"\s+titleLevel=\{2\}/);
  });
});
