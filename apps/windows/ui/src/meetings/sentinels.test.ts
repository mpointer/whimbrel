import { describe, expect, it, vi } from "vitest";

// A stand-in language that marks every translated string, so the test can tell what went
// through the catalog and what passed by untouched.
vi.mock("../l10n", () => ({
  t: (key: string, args?: Record<string, string | number>) =>
    `«${key.replace(/\{(\w+)\}/g, (whole, name: string) => (args && name in args ? String(args[name]) : whole))}»`,
}));

import { meetingTitleLabel, PLACEHOLDER_TITLE, speakerLabel } from "./sentinels";

describe("speakerLabel", () => {
  it("translates the stored speaker sentinels for display", () => {
    expect(speakerLabel("Me")).toBe("«Me»");
    expect(speakerLabel("Others")).toBe("«Others»");
    expect(speakerLabel("Speaker 3")).toBe("«Speaker 3»");
    expect(speakerLabel("Speaker 12")).toBe("«Speaker 12»");
  });

  it("passes a real name, or anything that only looks close, through unchanged", () => {
    for (const name of ["Ana", "me", "Speaker", "Speaker 2b", "Speakers 2", "Mel"]) {
      expect(speakerLabel(name)).toBe(name);
    }
  });

  it("leaves the stored value as it was", () => {
    const stored = { speaker: "Me" };
    speakerLabel(stored.speaker);
    expect(stored.speaker).toBe("Me");
    expect(stored.speaker === "Me").toBe(true);
  });
});

describe("meetingTitleLabel", () => {
  it("translates only the placeholder title", () => {
    expect(meetingTitleLabel(PLACEHOLDER_TITLE)).toBe("«Meeting»");
    expect(meetingTitleLabel("Budget sync")).toBe("Budget sync");
    expect(meetingTitleLabel("Meeting with Ana")).toBe("Meeting with Ana");
  });

  it("leaves the stored title English", () => {
    const row = { title: "Meeting" };
    meetingTitleLabel(row.title);
    expect(row.title).toBe(PLACEHOLDER_TITLE);
  });
});
