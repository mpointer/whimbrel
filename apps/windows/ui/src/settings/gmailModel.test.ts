import { describe, expect, it } from "vitest";
import { GMAIL_INTRO, deleteMailQuestion, GMAIL_MAIL_NOTE, GMAIL_PERMISSIONS, accountNote, canConnect, connectLabel, readStatus } from "./gmailModel";

describe("the Gmail card's words", () => {
  it("says what connecting is for, and that sending needs approval unless the user turns that off", () => {
    expect(GMAIL_INTRO).toMatch(/once you approve them/);
    expect(GMAIL_INTRO).toMatch(/never does unless you turn that on/);
  });

  it("says Whimbrel reads new mail, and only mail that arrives after connecting", () => {
    expect(GMAIL_INTRO).toMatch(/reads mail that arrives after you connect/);
    expect(GMAIL_INTRO).toMatch(/not mail from before/);
  });

  it("says where the mail is kept, for how long, and what uses it", () => {
    expect(GMAIL_MAIL_NOTE).toMatch(/kept on this PC/);
    expect(GMAIL_MAIL_NOTE).toMatch(/not synced to your other devices/);
    expect(GMAIL_MAIL_NOTE).toMatch(/no expiry/);
    expect(GMAIL_MAIL_NOTE).toMatch(/Thread summaries.*local Ollama/s);
  });

  it("is plain that Meeting Prep may send excerpts to a cloud model, and how to prevent that", () => {
    expect(GMAIL_MAIL_NOTE).toMatch(/Meeting Prep/);
    expect(GMAIL_MAIL_NOTE).toMatch(/short, redacted excerpts/);
    expect(GMAIL_MAIL_NOTE).toMatch(/cloud model unless Local-only is on/);
  });

  it("says disconnecting stops reading but keeps the mail already stored", () => {
    expect(GMAIL_MAIL_NOTE).toMatch(/Disconnecting stops reading but keeps the mail already stored/);
  });

  it("is honest that Google asks for more than Whimbrel uses today", () => {
    expect(GMAIL_PERMISSIONS).toMatch(/read and compose email/);
    expect(GMAIL_PERMISSIONS).toMatch(/to send what you approve and to read new mail/);
    expect(GMAIL_PERMISSIONS).not.toMatch(/does not read your mail/);
    expect(GMAIL_PERMISSIONS).toMatch(/unverified app/);
  });

  it("offers to connect another account once there is one", () => {
    expect(connectLabel(0)).toBe("Connect Gmail");
    expect(connectLabel(1)).toBe("Connect another Gmail account");
    expect(connectLabel(3)).toBe("Connect another Gmail account");
  });
});

describe("deleting stored mail", () => {
  it("names the account and says what goes with the mail and that it cannot be undone", () => {
    const q = deleteMailQuestion("me@x.com");
    expect(q).toMatch(/me@x\.com/);
    expect(q).toMatch(/Thread summaries and reply scores/);
    expect(q).toMatch(/cannot be undone/);
  });

  it("says old mail is not pulled back in", () => {
    expect(deleteMailQuestion("me@x.com")).toMatch(/only if it arrives after this/);
  });
});

describe("what the last read says", () => {
  it("says it has not read yet before the first round", () => {
    expect(readStatus({ lastReadUnixMs: null, readError: null })).toBe("Not read yet");
    expect(readStatus({})).toBe("Not read yet");
  });

  it("gives the time of the last good read", () => {
    const at = new Date(2026, 9, 15, 14, 5).getTime();
    expect(readStatus({ lastReadUnixMs: at, readError: null })).toMatch(/^Read mail /);
  });

  it("puts a failure first, because the time alone would look healthy", () => {
    const at = new Date(2026, 9, 15, 14, 5).getTime();
    expect(readStatus({ lastReadUnixMs: at, readError: "offline" })).toBe("Last read failed: offline");
    expect(readStatus({ lastReadUnixMs: null, readError: "offline" })).toBe("Last read failed: offline");
  });
});

describe("an account's state", () => {
  it("is quiet when the account can send", () => {
    expect(accountNote({ needsReconnect: false })).toBeNull();
  });

  it("says to reconnect when Google has not given permission to send", () => {
    expect(accountNote({ needsReconnect: true })).toMatch(/Needs reconnecting/);
  });

  it("blocks connecting only when something stops it, and says nothing when nothing does", () => {
    expect(canConnect({ blocked: null })).toBe(true);
    expect(canConnect({ blocked: undefined })).toBe(true);
    expect(canConnect({ blocked: "Node.js isn't installed." })).toBe(false);
  });
});
