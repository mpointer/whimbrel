import type { DeleteEverythingPreview } from "../generated/types";
import { t } from "../l10n";
import { diskSize } from "./statusModel";

/** What the user types to confirm (the Mac's sheet asks for the same word). The shell checks it too. */
export const CONFIRM_WORD = "DELETE";

/** The confirmation's state: closed, or open with what was typed, the sync-account choice, whether the delete is running and its error. */
export interface DialogState {
  open: boolean;
  typed: string;
  alsoDeleteSyncAccount: boolean;
  busy: boolean;
  error: string | null;
}

export const closed: DialogState = { open: false, typed: "", alsoDeleteSyncAccount: false, busy: false, error: null };

export type DialogAction =
  | { kind: "open" }
  | { kind: "cancel" }
  | { kind: "typed"; text: string }
  | { kind: "syncAccount"; on: boolean }
  | { kind: "busy" }
  | { kind: "failed"; error: string };

export function dialogReducer(s: DialogState, a: DialogAction): DialogState {
  switch (a.kind) {
    case "open":
      return { ...closed, open: true };
    case "cancel":
      // Once the delete runs, Cancel can't take it back: the slide images may already be gone.
      return s.busy ? s : closed;
    case "typed":
      return { ...s, typed: a.text };
    case "syncAccount":
      return s.busy ? s : { ...s, alsoDeleteSyncAccount: a.on };
    case "busy":
      return { ...s, busy: true, error: null };
    case "failed":
      return { ...s, busy: false, error: a.error };
  }
}

/** "Delete and restart" is on only once DELETE is typed exactly, nothing is running and the preview is in. */
export function canDelete(s: DialogState, preview: DeleteEverythingPreview | null): boolean {
  return s.typed === CONFIRM_WORD && !s.busy && preview != null;
}

/** What the shell is sent on confirm: the typed word (it checks it), the sync-account choice, and whether the dialog said the slide folder is left alone. */
export function confirmArgs(s: DialogState, p: DeleteEverythingPreview) {
  return {
    confirm: s.typed,
    alsoDeleteSyncAccount: p.syncOn && s.alsoDeleteSyncAccount,
    slidesLeftAlone: p.snapshotsRefused != null,
  };
}

/** What goes and what stays, as the confirmation says it (the Mac's sheet, with this PC's facts). Every line is true as written. */
export function confirmLines(p: DeleteEverythingPreview, alsoDeleteSyncAccount = false): string[] {
  const lines = [
    t("This removes the memory database, the logs and everything else in the data folder ({folder}, {size}), then restarts Whimbrel.", {
      folder: p.dataFolder,
      size: diskSize(p.dataBytes),
    }),
  ];
  if (p.snapshotsFolder) {
    lines.push(t("It also removes the meeting slide images Whimbrel saved in {folder} ({size}).", { folder: p.snapshotsFolder, size: diskSize(p.snapshotsBytes) }));
  }
  if (p.snapshotsRefused) lines.push(p.snapshotsRefused);
  // The connected Google accounts stay connected (Mike, 2026-11-17): their list is carried over
  // to the new database (connectors::kept_accounts), which needs the list read now.
  const google = p.googleAccounts;
  if (google == null) {
    lines.push(
      p.storeOpen
        ? t("Whimbrel couldn't list your connected Google accounts. Their sign-ins stay in Credential Manager, but you'll need to connect the accounts again after the restart.")
        : t("Whimbrel can't list your connected Google accounts while it runs without its memory database. Their sign-ins stay in Credential Manager, but you'll need to connect the accounts again after the restart."),
    );
  } else if (google.length > 0) {
    lines.push(t("These Google accounts stay connected: {accounts}. Their calendar events, and their mail where Whimbrel can read it, are downloaded again into the new library.", { accounts: google.join(", ") }));
  }
  lines.push(t("The memory database's keys in Credential Manager go with it."));
  if (p.endsMeeting) lines.push(p.endsMeeting);
  // In memory only, as on the Mac: the settings file keeps the choice, and it stays.
  lines.push(t("Activity capture is turned off now and comes back on after the restart if it is on in Settings."));
  // The settings file stays but its first-run state resets (settings::keys::FIRST_RUN_STATE).
  lines.push(t("After the restart Whimbrel starts like a new install, and its first-run notices show again."));
  const accountGoes = p.syncOn && alsoDeleteSyncAccount;
  if (accountGoes) lines.push(t("The sync key goes with the account."));
  // A stored sync sign-in stays (sync on or not, Ready or not) unless the account goes too.
  const signInStays = p.syncSignedIn && !accountGoes;
  lines.push(
    signInStays
      ? t("These stay: your settings (settings.json), downloaded models, and everything else Whimbrel keeps in Credential Manager, such as your API keys, your own Google client ID and secret, this PC's sync identity and its sync sign-in.")
      : t("These stay: your settings (settings.json), downloaded models, and everything else Whimbrel keeps in Credential Manager, such as your API keys, your own Google client ID and secret, and this PC's sync identity."),
  );
  return lines;
}

/** What the sync-account choice means, below its checkbox (the Mac's wording). */
export function syncAccountLine(alsoDeleteSyncAccount: boolean): string {
  return alsoDeleteSyncAccount
    ? t("The encrypted copy on the sync server is deleted and every device is signed out of sync. Your other devices keep their own data.")
    : t("Sync is on. This PC will download everything again from your other devices unless you also delete your sync account.");
}
