import { describe, expect, it } from "vitest";
import { episodeSummaryText } from "./rows";

describe("episodeSummaryText", () => {
  it("shows the summary when there is one", () => {
    expect(episodeSummaryText({ summary: "Edited the plan.", status: "summarized" })).toBe("Edited the plan.");
  });

  it("says summaries come from Ollama whatever the provider, and why one failed", () => {
    const failed = episodeSummaryText({ summary: null, status: "summarize_failed" });
    expect(failed).toMatch(/^Not summarized\. Episode summaries are written by Ollama on this PC/);
    expect(failed).toContain("Settings → AI & routing");
    expect(episodeSummaryText({ summary: null, status: "pending" })).toMatch(/^Not summarized yet\./);
  });
});
