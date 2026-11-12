import { describe, expect, it } from "vitest";
import { calendarAccessNote } from "./accountsModel";

describe("calendarAccessNote", () => {
  const accounts = {
    accessIssues: [
      { email: "a@x.com", code: "calendar_access_not_granted" },
      { email: "b@x.com", code: "calendar_access_blocked" },
    ],
  };

  it("says how to grant access when the calendar box was left unticked", () => {
    expect(calendarAccessNote(accounts, "a@x.com")).toBe(
      "Google didn't give Whimbrel access to your calendar. Click Connect another calendar, sign in as this address again, and allow calendar access.",
    );
  });

  it("says an organization may be blocking it, and nothing for a working account", () => {
    expect(calendarAccessNote(accounts, "b@x.com")).toMatch(/^Google is blocking calendar access/);
    expect(calendarAccessNote(accounts, "c@x.com")).toBeNull();
  });
});
