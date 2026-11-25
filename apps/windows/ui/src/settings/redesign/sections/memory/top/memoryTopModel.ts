// The words and choices of the top half of Memory & Privacy (Memory, Never captured, Your data, Delete everything).
// Pure. The old models (captureSummary, deleteEverythingModel) are reused where their words are fine; where a line
// names "Activity capture" or the "memory database" this file says it in the redesign's words ("Memory", "your library").

import type { CaptureSettings, CaptureStatus, DeleteEverythingPreview, RunningApp } from "../../../../../generated/types";
import { t } from "../../../../../l10n";
import { memoryControls } from "../../../../../system";
import { memoryWords, popupMemory, stoppedLine, type PopupMemory } from "../../../../../popup/popupModel";
import { captureSummary } from "../../../../captureSummary";
import { syncAccountLine } from "../../../../deleteEverythingModel";
import { diskSize } from "../../../../statusModel";

/** The words of one state of the Memory row, from the tray popup's own model (popup/popupModel.ts: `popupMemory`,
 *  `memoryWords`, `stoppedLine`), so the two never say different things about the same state. */
export interface MemoryRow {
  /** The popup's state, or null while the status has not been read. */
  memory: PopupMemory | null;
  /** The state in the popup's words: "Memory on", "Memory paused until tomorrow at 4:00 AM". */
  words: string;
  /** Why the switch is off, or what the state means ("The screen is locked."). Null when there is nothing to add. */
  reason: string | null;
  /** The switch cannot be used: there is no library. `reason` says why. A stopped Memory keeps its switch: turning it
   *  off writes the setting, so Memory will not start again. */
  switchDisabled: boolean;
  /** What using the switch does when that is not obvious (a stopped Memory), or null. */
  switchHint: string | null;
  /** What the Pause row offers: timed choices while on, Resume now during a timed pause, nothing in any other state. */
  pause: "choices" | "resume" | "none";
  /** Why Pause is not offered, in words, when it is not. */
  pauseReason: string | null;
}

/** The fixed reasons the shell gives for pausing capture (capture/session_gate.rs `describe`), in the app's words. The
 *  shell's own English text is never shown: an unknown one says nothing. */
export function pausedByAppReason(paused: string | null | undefined): string | null {
  switch (paused) {
    case "the screen is locked":
      return t("The screen is locked.");
    case "the display is off":
      return t("The display is off.");
    case "the PC is going to sleep":
      return t("The PC is going to sleep.");
    case "no input for 5 minutes":
      return t("There has been no input for 5 minutes.");
    default:
      return null;
  }
}

export const noLibraryLine = () => t("Memory is off because Whimbrel could not open your library. Restart Whimbrel to try again.");

/** The Memory row: the same state the tray popup shows, in the same words. `noLibrary` is the launch state's
 *  "opened without a library", in which Memory cannot run and the popup shows a notice instead of a row. */
export function memoryRow(status: CaptureStatus, now: number, locale: string, noLibrary = false): MemoryRow {
  if (noLibrary) {
    return { memory: { state: "off" }, words: memoryWords({ state: "off" }, now, locale), reason: noLibraryLine(), switchDisabled: true, switchHint: null, pause: "none", pauseReason: noLibraryLine() };
  }
  const memory = popupMemory(status, now) ?? { state: "off" as const };
  const words = memoryWords(memory, now, locale);
  const controls = memoryControls(memory.state);
  switch (memory.state) {
    case "stopped":
      return { memory, words, reason: stoppedLine(memory), switchDisabled: false, switchHint: t("Turning it off means Memory will not start again until you turn it on."), pause: "none", pauseReason: stoppedLine(memory) };
    case "off":
      return { memory, words, reason: null, switchDisabled: false, switchHint: null, pause: "none", pauseReason: t("Turn Memory on to pause it.") };
    case "pausedByApp":
      return { memory, words, reason: pausedByAppReason(status.paused), switchDisabled: false, switchHint: null, pause: "none", pauseReason: t("Memory is already paused.") };
    case "paused":
      return { memory, words, reason: null, switchDisabled: false, switchHint: null, pause: controls.resumeNow ? "resume" : "none", pauseReason: null };
    case "on":
      return { memory, words, reason: null, switchDisabled: false, switchHint: null, pause: controls.pause ? "choices" : "none", pauseReason: null };
  }
}

/** The old status line (what was captured last), shown under the state row only while Memory is on and running. Never
 *  the shell's raw error or pause text. */
export function memoryDetailLine(status: CaptureStatus, now: number): string | null {
  if (popupMemory(status, now)?.state !== "on") return null;
  return captureSummary(status, new Date(now));
}

const clip = (title: string) => (title.length > 50 ? `${title.slice(0, 50)}…` : title);

/** Running apps not yet excluded (compared without case, as the old chooser does), as select options. */
export function excludeCandidates(settings: Pick<CaptureSettings, "excluded" | "runningApps">): { value: string; label: string }[] {
  const excluded = new Set(settings.excluded.map((e) => e.toLowerCase()));
  return settings.runningApps.filter((a: RunningApp) => !excluded.has(a.appId.toLowerCase())).map((a) => ({ value: a.appId, label: t("{app}: {title}", { app: a.exe, title: clip(a.title) }) }));
}

/** The text-recognition line: the languages Windows can read, or what is missing and where to add it. Same strings as the old card. */
export function ocrLine(ocr: CaptureSettings["ocr"]): { covered: boolean; text: string } {
  if (ocr.covered) return { covered: true, text: t("OCR languages on this PC: {languages}.", { languages: ocr.available.join(", ") }) };
  const missing =
    ocr.profile[0] != null
      ? t(
          "Windows has no OCR language pack for {language}, so apps without accessible text (some video and remote-desktop windows) won't be captured. Add the language under Settings → Time & language → Language & region, including \"Optical character recognition\".",
          { language: ocr.profile[0] },
        )
      : t(
          "Windows has no OCR language pack for your display language, so apps without accessible text (some video and remote-desktop windows) won't be captured. Add the language under Settings → Time & language → Language & region, including \"Optical character recognition\".",
        );
  return { covered: false, text: ocr.available.length > 0 ? `${missing} ${t("Installed: {languages}.", { languages: ocr.available.join(", ") })}` : missing };
}

/** The Delete everything card's sentence (the old one said "memory"). */
export const deleteIntro = () => t("Removes your meetings, library, recordings and logs from this PC. Your settings (settings.json), downloaded models and API keys stay.");

/**
 * What goes and what stays, as the confirmation says it. The old confirmLines with "memory database" and
 * "Activity capture" put into the redesign's words. `snapshotsRefused` and `endsMeeting` are written by the shell
 * and shown as it wrote them.
 */
export function redesignDeleteLines(p: DeleteEverythingPreview, alsoDeleteSyncAccount = false): string[] {
  const lines = [
    t("This removes your library, the logs and everything else in the data folder ({folder}, {size}), then restarts Whimbrel.", { folder: p.dataFolder, size: diskSize(p.dataBytes) }),
  ];
  if (p.snapshotsFolder) {
    lines.push(t("It also removes the meeting slide images Whimbrel saved in {folder} ({size}).", { folder: p.snapshotsFolder, size: diskSize(p.snapshotsBytes) }));
  }
  if (p.snapshotsRefused) lines.push(p.snapshotsRefused);
  const google = p.googleAccounts;
  if (google == null) {
    lines.push(
      p.storeOpen
        ? t("Whimbrel couldn't list your connected Google accounts. Their sign-ins stay in Credential Manager, but you'll need to connect the accounts again after the restart.")
        : t("Whimbrel can't list your connected Google accounts while it runs without its library. Their sign-ins stay in Credential Manager, but you'll need to connect the accounts again after the restart."),
    );
  } else if (google.length > 0) {
    lines.push(t("These Google accounts stay connected: {accounts}. Their calendar events, and their mail where Whimbrel can read it, are downloaded again into the new library.", { accounts: google.join(", ") }));
  }
  lines.push(t("Your library's keys in Credential Manager go with it."));
  if (p.endsMeeting) lines.push(p.endsMeeting);
  lines.push(t("Memory is turned off now and comes back on after the restart if it is on in Settings."));
  lines.push(t("After the restart Whimbrel starts like a new install, and its first-run notices show again."));
  const accountGoes = p.syncOn && alsoDeleteSyncAccount;
  if (accountGoes) lines.push(t("The sync key goes with the account."));
  const signInStays = p.syncSignedIn && !accountGoes;
  lines.push(
    signInStays
      ? t("These stay: your settings (settings.json), downloaded models, and everything else Whimbrel keeps in Credential Manager, such as your API keys, your own Google client ID and secret, this PC's sync identity and its sync sign-in.")
      : t("These stay: your settings (settings.json), downloaded models, and everything else Whimbrel keeps in Credential Manager, such as your API keys, your own Google client ID and secret, and this PC's sync identity."),
  );
  return lines;
}

/** The sync-account choice's line. The old words name neither "memory" nor "capture", so they are reused. */
export { syncAccountLine };

/** Names the redesigned copy must not show (docs/ui-redesign/screens/settings.md, "Naming"). */
export const BANNED_PHRASES = ["memory database", "Activity capture", "Capture & Privacy", "AI & routing"] as const;
