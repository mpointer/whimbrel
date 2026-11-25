// Which meeting's recording is paused, for the places that show a meeting's status word (the Meetings list, Today's
// timeline, Library, the workspace). A paused meeting stays stored as "recording", so they ask the recording status.
// With the redesign on or off: a pause can outlive the flag being turned off.

import { useEffect, useState } from "react";
import { meetingRecordingStatus } from "../api";
import { pausedMeetingId } from "./recordingModel";

/** How often the status is read while a meeting shown records; the tray popup or the bar can pause it at any time. */
const PAUSE_POLL_MS = 3_000;

/** The paused meeting's id, or null. Reads nothing unless `active` (a meeting shown is recording). */
export function usePausedMeetingId(active: boolean): number | null {
  const [id, setId] = useState<number | null>(null);
  useEffect(() => {
    if (!active) {
      setId(null);
      return;
    }
    let alive = true;
    const load = () =>
      meetingRecordingStatus().then(
        (view) => alive && setId(pausedMeetingId(view)),
        () => {},
      );
    load();
    const timer = setInterval(load, PAUSE_POLL_MS);
    return () => {
      alive = false;
      clearInterval(timer);
    };
  }, [active]);
  return id;
}
