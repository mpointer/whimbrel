// One recording state for the whole redesigned shell: the shell polls once and hands it to the bar and to the live
// meeting screen, so a Pause pressed in one place shows in the other at once.

import { createContext, useContext } from "react";
import type { RecordingControls } from "../../shell/redesign/useRecording";

export const RecordingContext = createContext<RecordingControls | null>(null);

/** The shell's recording state, or null outside the redesigned shell (a gallery page, a test). */
export function useRecordingContext(): RecordingControls | null {
  return useContext(RecordingContext);
}

/** Dispatched by the shell for Ctrl+Shift+M. The live screen answers it by marking with the note line being written
 *  and calling `preventDefault`; unanswered, the shell marks with no label. */
export const MARK_EVENT = "whimbrel:mark-moment";
