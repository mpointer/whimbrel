import { describe, expect, it } from "vitest";
import { addressLooksUsable, chosenRecipients, recapSentLine, recapSubject } from "./recapModel";

describe("recap", () => {
  it("says how many people the recap went to", () => {
    expect(recapSentLine(1)).toBe("Sent the recap to 1 person");
    expect(recapSentLine(3)).toBe("Sent the recap to 3 people");
  });

  it("titles the email with the meeting and its date", () => {
    const subject = recapSubject("Planning", Date.UTC(2026, 10, 14, 15), "en-US");
    expect(subject).toBe("Meeting recap: Planning (Nov 14, 2026)");
  });

  it("accepts an ordinary address and refuses ones that could break the To line", () => {
    expect(addressLooksUsable(" ana@example.com ")).toBe(true);
    for (const bad of ["ana", "ana@", "ana@example", "a b@x.com", "a@x.com, b@x.com", "a@x.com;b@x.com", "<a@x.com>"]) {
      expect(addressLooksUsable(bad)).toBe(false);
    }
  });

  it("sends to each address once, ticked ones first", () => {
    expect(chosenRecipients(["Ana@x.com", "bo@x.com"], ["ana@x.com", " cy@x.com "])).toEqual(["Ana@x.com", "bo@x.com", "cy@x.com"]);
  });
});
