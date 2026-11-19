// The recording state for the bar at the top of every section and for the live meeting screen (redesign flag on only):
// the status polled every 3 s as the Meetings card does, a once-a-second clock, and the controls.

import { useCallback, useEffect, useState } from "react";
import { meetingRecordPause, meetingRecordResume, meetingRecordStop, meetingRecordingStatus } from "../../api";
import type { MeetingRecordingView } from "../../generated/types";
import { liveStateOf, type LiveState } from "../../meetings/redesign/liveModel";

export interface RecordingControls {
  view: MeetingRecordingView | null;
  /** When `view` was read: while paused, recorded time is frozen at this moment (see `recordedMs`). */
  readAt: number;
  state: LiveState;
  /** The page's clock, ticking once a second. */
  now: number;
  /** A command is running: the buttons wait for it. */
  busy: boolean;
  stop: () => void;
  pause: () => void;
  resume: () => void;
}

export function useRecording(active: boolean): RecordingControls {
  const [read, setRead] = useState<{ view: MeetingRecordingView | null; at: number }>({ view: null, at: 0 });
  const [now, setNow] = useState(() => Date.now());
  const [busy, setBusy] = useState(false);
  useEffect(() => {
    if (!active) return;
    const load = () => meetingRecordingStatus().then((view) => setRead({ view, at: Date.now() }), () => {});
    load();
    // The tray popup can stop or pause a recording too, so the bar keeps checking, as the Meetings card does.
    const status = setInterval(load, 3_000);
    const clock = setInterval(() => setNow(Date.now()), 1_000);
    return () => {
      clearInterval(status);
      clearInterval(clock);
    };
  }, [active]);
  const run = useCallback((command: () => Promise<MeetingRecordingView>) => {
    setBusy(true);
    command()
      .then((view) => setRead({ view, at: Date.now() }))
      .catch(() => {})
      .finally(() => setBusy(false));
  }, []);
  const stop = useCallback(() => run(meetingRecordStop), [run]);
  const pause = useCallback(() => run(meetingRecordPause), [run]);
  const resume = useCallback(() => run(meetingRecordResume), [run]);
  return { view: read.view, readAt: read.at, state: liveStateOf(read.view), now, busy, stop, pause, resume };
}
