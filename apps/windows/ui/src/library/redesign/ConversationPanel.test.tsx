import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import type { ConversationRow } from "../../generated/types";
import { conversationActionLabels } from "../../chat/chatModel";
import { ConversationPanel } from "./ConversationPanel";

const row = (id: number, title: string, starred = false): ConversationRow => ({ id, title, starred, updatedAtUnixMs: 1 });
const noop = () => {};

describe("a conversation row's buttons", () => {
  it("name the conversation they act on, so Tab reads which one", () => {
    const html = renderToStaticMarkup(
      <ConversationPanel rows={[row(1, "Bake sale prices"), row(2, "Shuttle times", true)]} activeId={null} answeringId={null} canStartNew onNew={noop} onOpen={noop} onChange={noop} onDeleted={noop} onError={noop} />,
    );
    const labels = [...html.matchAll(/class="w-icon-button"[^>]*aria-label="([^"]+)"/g)].map((m) => m[1]);
    expect(labels).toEqual(["Star Bake sale prices", "Rename Bake sale prices", "Delete Bake sale prices", "Unstar Shuttle times", "Rename Shuttle times", "Delete Shuttle times"]);
  });

  it("keep the same names for the legacy Ask view", () => {
    expect(conversationActionLabels(row(3, "Plant sale"))).toEqual({ star: "Star Plant sale", rename: "Rename Plant sale", delete: "Delete Plant sale" });
    expect(conversationActionLabels(row(3, "Plant sale", true)).star).toBe("Unstar Plant sale");
  });
});
