import type { GmailAccount, GmailAccounts } from "../generated/types";
import { timeLabel } from "../library/rows";

// What Settings → Gmail says, apart from the component.

/** What connecting Gmail is for. */
export const GMAIL_INTRO =
  "Connect Gmail so Whimbrel can send the emails your routines draft, such as the end-of-day recap, once you approve them. " +
  "A routine can also be set to send without asking; it never does unless you turn that on for it. " +
  "Whimbrel also reads mail that arrives after you connect, not mail from before.";

/** Where the mail goes and what uses it. Must stay true to what the poller and the jobs do. */
export const GMAIL_MAIL_NOTE =
  "Mail Whimbrel reads is kept on this PC and is not synced to your other devices. It has no expiry: it stays until Gmail reports the message permanently deleted. " +
  "Thread summaries use it, and only a local Ollama model reads it for that. " +
  "Meeting Prep sends short, redacted excerpts of mail with a meeting's attendees to the model your routing picks, which is a cloud model unless Local-only is on. " +
  "Disconnecting stops reading but keeps the mail already stored.";

/** What Google will ask, said plainly, because the permissions Google lists are broader than the way they are used. */
export const GMAIL_PERMISSIONS =
  "Google will ask to let Whimbrel read and compose email. Whimbrel uses it to send what you approve and to read new mail. " +
  "These are sensitive permissions, so Google may call this an unverified app. That is expected for a personal Google Cloud project.";

/** The question before mail is deleted: what goes, and that it cannot be undone. */
export const deleteMailQuestion = (email: string): string =>
  `Delete all mail Whimbrel stored from ${email}? Thread summaries and reply scores made from it go too. This cannot be undone. ` +
  "Mail is read again only if it arrives after this.";

export const connectLabel = (connected: number): string => (connected > 0 ? "Connect another Gmail account" : "Connect Gmail");

/** When the account was last read, or why the last round failed. A failure comes first: a time next to it would look healthy. */
export function readStatus(account: { lastReadUnixMs?: number | null; readError?: string | null }): string {
  if (account.readError) return `Last read failed: ${account.readError}`;
  return account.lastReadUnixMs ? `Read mail ${timeLabel(account.lastReadUnixMs)}` : "Not read yet";
}

/** What is wrong with an account, or null when it is fine. */
export function accountNote(account: Pick<GmailAccount, "needsReconnect">): string | null {
  return account.needsReconnect ? "Needs reconnecting: Google has not given Whimbrel permission to send from this account." : null;
}

/** Whether Connect may be used: not while something stops Gmail from being connected at all. */
export const canConnect = (state: Pick<GmailAccounts, "blocked">): boolean => state.blocked == null;
