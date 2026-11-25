// The recording state for the bar at the top of every section and for the live meeting screen (redesign flag on only):
// the status polled every 3 s as the Meetings card does, a once-a-second clock, and the controls.

import { useCallback, useEffect, useState } from "react";
import { meetingRecordPause, meetingRecordResume, meetingRecordStop, meetingRecordingStatus } from "../../api";
import type { MeetingRecordingView } from "../../generated/types";
import { liveStateOf, statusPollMs, type LiveState } from "../../meetings/redesign/liveModel";

export interface RecordingControls {
  view: MeetingRecordingView | null;
  /** The last read of the status failed (cleared by the next one that works). `view` is then the last good answer, or null. */
  failed: boolean;
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
  /** Reads the status again now (the tray popup's "Try again" when it could not read it). */
  reload: () => void;
}

export function useRecording(active: boolean): RecordingControls {
  const [read, setRead] = useState<{ view: MeetingRecordingView | null; at: number }>({ view: null, at: 0 });
  const [now, setNow] = useState(() => Date.now());
  const [busy, setBusy] = useState(false);
  const [failed, setFailed] = useState(false);
  const state = liveStateOf(read.view);
  const pollMs = statusPollMs(state);
  const load = useCallback(
    () =>
      meetingRecordingStatus().then(
        (view) => {
          setRead({ view, at: Date.now() });
          setFailed(false);
        },
        () => setFailed(true),
      ),
    [],
  );
  useEffect(() => {
    if (!active) return;
    load();
    // The tray popup can stop or pause a recording too, so the bar keeps checking, as the Meetings card does (more often
    // while starting or resuming).
    const status = setInterval(load, pollMs);
    return () => clearInterval(status);
  }, [active, pollMs, load]);
  useEffect(() => {
    if (!active) return;
    const clock = setInterval(() => setNow(Date.now()), 1_000);
    return () => clearInterval(clock);
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
  const reload = useCallback(() => void load(), [load]);
  return { view: read.view, failed, readAt: read.at, state, now, busy, stop, pause, resume, reload };
}
