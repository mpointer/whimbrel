import type { GmailAccount, GmailAccounts } from "../generated/types";
import { t } from "../l10n";
import { timeLabel } from "../library/rows";

// What Settings → Gmail says, apart from the component. Functions, not constants, so the text
// follows the current language.

/** What connecting Gmail is for. */
export const gmailIntro = (): string =>
  t(
    "Connect Gmail so Whimbrel can send the emails your routines draft, such as the end-of-day recap, once you approve them. A routine can also be set to send without asking; it never does unless you turn that on for it.",
  );

/** Reading mail: only with the user's own Google Cloud project (Advanced), since 2026-11-12. */
export const gmailReadingIntro = (): string =>
  t("With your own Google Cloud project, Whimbrel also reads mail that arrives after you connect, not mail from before.");

/** The row note for an account Google let send but not read. */
export const sendOnlyNote = (): string => t("Sending only. Reading email needs your own Google project (Advanced).");

/** Whether anything here reads mail: the user's own client, or an older account that still can. */
export const readsMail = (state: Pick<GmailAccounts, "accounts">, ownClient: boolean): boolean =>
  ownClient || state.accounts.some((a) => !a.sendOnly);

/** Where the mail goes and what uses it. Must stay true to what the poller and the jobs do. */
export const gmailMailNote = (): string =>
  t(
    "Mail Whimbrel reads is kept on this PC and is not synced to your other devices. It has no expiry: it stays until Gmail reports the message permanently deleted. Thread summaries use it, and only a local Ollama model reads it for that. Meeting Prep sends short, redacted excerpts of mail with a meeting's attendees to the model your routing picks, which is a cloud model unless Local-only is on. Disconnecting stops reading but keeps the mail already stored.",
  );

/** What Google will ask, said plainly: on the app's client only to send, on the user's own client to read too. */
export const gmailPermissions = (ownClient: boolean): string =>
  ownClient
    ? t(
        "Google will ask to let Whimbrel read and send email. Whimbrel uses it to send what you approve and to read new mail. Google may call this an unverified app. That is expected for your own Google Cloud project.",
      )
    : t("Google will ask to let Whimbrel send email for you. Whimbrel sends only what you approve, or what a routine you set to send without asking writes.");

/** The question before mail is deleted: what goes, and that it cannot be undone. */
export const deleteMailQuestion = (email: string): string =>
  t(
    "Delete all mail Whimbrel stored from {email}? Thread summaries and reply scores made from it go too. This cannot be undone. Mail is read again only if it arrives after this.",
    { email },
  );

export const connectLabel = (connected: number): string =>
  connected > 0 ? t("Connect another Gmail account") : t("Connect Gmail");

/** When the account was last read, or why the last round failed. A failure comes first: a time next to it would look healthy. */
export function readStatus(account: { lastReadUnixMs?: number | null; readError?: string | null }, locale: string): string {
  if (account.readError) return t("Last read failed: {error}", { error: account.readError });
  return account.lastReadUnixMs ? t("Read mail {time}", { time: timeLabel(account.lastReadUnixMs, locale) }) : t("Not read yet");
}

/** What is wrong with an account, or null when it is fine. */
export function accountNote(account: Pick<GmailAccount, "needsReconnect">): string | null {
  return account.needsReconnect ? t("Needs reconnecting: Google has not given Whimbrel permission to send from this account.") : null;
}

/** Whether Connect may be used: not while something stops Gmail from being connected at all. */
export const canConnect = (state: Pick<GmailAccounts, "blocked">): boolean => state.blocked == null;
