// The level meter's reading, polled about four times a second while a recording runs (redesign flag on only). Read,
// drawn and thrown away: nothing is stored here or in the shell. Zero while paused.

import { useEffect, useState } from "react";
import { meetingAudioLevel } from "../../api";

/** How often the reading is asked for: about four times a second. */
export const LEVEL_POLL_MS = 250;

/** The louder of the two sources, 0 to 1. `enabled` is false when nothing records, while paused or when the meter is not on screen. */
export function useAudioLevel(enabled: boolean): number {
  const [level, setLevel] = useState(0);
  useEffect(() => {
    if (!enabled) {
      setLevel(0);
      return;
    }
    let live = true;
    const read = () =>
      meetingAudioLevel().then(
        (v) => live && setLevel(Math.max(v.system, v.mic)),
        () => {},
      );
    read();
    const timer = setInterval(read, LEVEL_POLL_MS);
    return () => {
      live = false;
      clearInterval(timer);
    };
  }, [enabled]);
  return level;
}
