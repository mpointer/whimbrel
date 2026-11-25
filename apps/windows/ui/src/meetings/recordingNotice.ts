// The recording notice (Release 1 item 1.7, W-1.7): an optional reminder to tell people a meeting
// is being recorded, with wording to copy. One shared state, so the workspace, the Record a meeting
// card and Settings agree, and Done in one place hides the card in all of them.

import { useSyncExternalStore } from "react";
import { recordingNoticeSettings, setRecordingNoticeSettings } from "../api";
import type { RecordingNoticeSettings } from "../generated/types";
import { t } from "../l10n";

/** The wording to show and copy: the custom text, else the default in the app's language. */
export function noticeText(custom: string): string {
  return custom.trim() || t("I'm recording this meeting with Whimbrel, on my own computer, for my notes.");
}

/** The card shows while this PC records `meetingId`, with the setting on, until Done for that meeting. */
export function noticeVisible(settings: RecordingNoticeSettings | null, meetingId: number | null | undefined, dismissed: number | null): boolean {
  return !!settings?.enabled && meetingId != null && meetingId !== dismissed;
}

/** The meeting the Meetings screen itself shows the notice for, or null. While this PC records, the notice has to be on the
 *  screen whichever meeting is selected. The live screen draws it when the meeting being recorded is the selected one, so the
 *  Meetings screen draws it in every other case (another meeting selected, or none), never both at once. */
export function meetingsNoticeFor(recordingId: number | null | undefined, selectedId: number | null, settings: RecordingNoticeSettings | null, dismissed: number | null): number | null {
  if (recordingId == null || recordingId === selectedId) return null;
  return noticeVisible(settings, recordingId, dismissed) ? recordingId : null;
}

/** Whether the live screen for `meetingId` draws the notice. It does for the meeting this PC is recording. A live screen open on
 *  some other meeting (a row left as "recording" by an earlier run, say) does not: this PC is not recording that one, and the
 *  Meetings screen is already showing the notice for the one it is (`meetingsNoticeFor`). With no recording id known yet (a
 *  recording still starting, or a screen outside the shell), the screen's own meeting decides, as before. */
export function liveNoticeShown(meetingId: number | null | undefined, recordingId: number | null | undefined, settings: RecordingNoticeSettings | null, dismissed: number | null): boolean {
  if (recordingId != null && meetingId != null && recordingId !== meetingId) return false;
  return noticeVisible(settings, meetingId, dismissed);
}

interface State {
  settings: RecordingNoticeSettings | null;
  /** The meeting whose card was dismissed. Kept until the app restarts, as on the Mac. */
  dismissed: number | null;
}

let state: State = { settings: null, dismissed: null };
const listeners = new Set<() => void>();
let loading = false;

function set(change: Partial<State>) {
  state = { ...state, ...change };
  listeners.forEach((listener) => listener());
}

function subscribe(listener: () => void) {
  listeners.add(listener);
  if (!state.settings && !loading) {
    loading = true;
    recordingNoticeSettings()
      .then((settings) => set({ settings }), () => {})
      .finally(() => (loading = false));
  }
  return () => listeners.delete(listener);
}

export function useRecordingNotice(): State {
  return useSyncExternalStore(subscribe, () => state);
}

export function dismissNotice(meetingId: number): void {
  set({ dismissed: meetingId });
}

/** Saves the setting; resolves with what Rust stored (blank wording comes back as the default). */
export function saveNoticeSettings(next: RecordingNoticeSettings): Promise<void> {
  set({ settings: next });
  return setRecordingNoticeSettings(next).then((settings) => set({ settings }));
}
