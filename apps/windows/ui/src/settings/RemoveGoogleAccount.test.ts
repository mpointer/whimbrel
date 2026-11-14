import { describe, expect, it } from "vitest";
import { removalMessage } from "./RemoveGoogleAccount";

describe("removing a Google account", () => {
  it("says the data leaves this PC and whether access is revoked", () => {
    expect(removalMessage("gmail", true)).toMatch(/Gmail data from this PC and revokes Whimbrel's access/);
    expect(removalMessage("gmail", false)).toMatch(/keeps its Google access to this address/);
    expect(removalMessage("calendar", true)).toMatch(/Calendar events from this PC and revokes.*Meetings you recorded are kept/);
    expect(removalMessage("calendar", false)).toMatch(/keeps its Google access.*Meetings you recorded are kept/);
  });
});
