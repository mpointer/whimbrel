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
