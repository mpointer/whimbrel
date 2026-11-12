import { describe, expect, it } from "vitest";
import type { CalendarAccounts, GmailAccounts } from "../generated/types";
import { expiredSignIns } from "./signInModel";

const calendar = (signInExpired: string[]): CalendarAccounts => ({
  emails: ["a@x.com", "b@x.com"],
  upcoming: 0,
  signInExpired,
  accessIssues: [],
});
const gmail = (expired: Record<string, boolean>): GmailAccounts => ({
  accounts: Object.entries(expired).map(([email, signInExpired]) => ({ email, signInExpired, needsReconnect: false })),
});

describe("expiredSignIns", () => {
  it("is empty when nothing expired or nothing loaded", () => {
    expect(expiredSignIns(null, null)).toEqual([]);
    expect(expiredSignIns(calendar([]), gmail({ "a@x.com": false }))).toEqual([]);
  });

  it("lists calendar then Gmail, keeping an address that expired on both", () => {
    expect(expiredSignIns(calendar(["b@x.com"]), gmail({ "a@x.com": false, "b@x.com": true }))).toEqual([
      { connector: "calendar", email: "b@x.com" },
      { connector: "gmail", email: "b@x.com" },
    ]);
  });
});
