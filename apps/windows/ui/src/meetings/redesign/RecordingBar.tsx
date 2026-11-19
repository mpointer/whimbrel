// The recording bar: state word, recorded time, level meter, Pause or Resume, Stop. One component for the top of the live
// screen's column and for the shell's bar on every other section (redesign flag on only), so the two never differ.

import { useEffect, useRef, useState } from "react";
import { t } from "../../l10n";
import { WButton, WLevelMeter, WRecordingIndicator, type WRecordingState } from "../../system";
import type { RecordingControls } from "../../shell/redesign/useRecording";
import { announcementFor, audioIsArriving, liveControls, recordedSeconds, type LiveState } from "./liveModel";
import { useAudioLevel } from "./useAudioLevel";

/** The indicator's state for a screen state. A read-only meeting (another device's) shows as recording, with no controls. */
export function barStateOf(state: LiveState): WRecordingState | null {
  switch (state) {
    case "idle":
      return null;
    case "readOnly":
      return "recording";
    default:
      return state;
  }
}

/** The words a screen reader hears once when the state changes ("Recording paused", "Recording"). */
export function useAnnouncement(state: LiveState): string {
  const previous = useRef(state);
  const [text, setText] = useState("");
  useEffect(() => {
    const spoken = announcementFor(previous.current, state);
    previous.current = state;
    if (spoken) setText(spoken === "paused" ? t("Recording paused") : t("Recording"));
  }, [state]);
  return text;
}

export interface RecordingBarProps {
  rec: RecordingControls;
  /** The state to draw: `rec.state`, or "readOnly" for a meeting on another device. */
  state?: LiveState;
  /** Wall-clock seconds for a read-only meeting, which has no pause fields. */
  readOnlyElapsedSeconds?: number;
  /** The stage under a processing bar ("Writing the summary"). */
  stage?: string | null;
}

export function RecordingBar({ rec, state: given, readOnlyElapsedSeconds, stage }: RecordingBarProps) {
  const state = given ?? rec.state;
  const barState = barStateOf(state);
  const controls = liveControls(state, rec.busy);
  const level = useAudioLevel(state === "recording" || state === "problem" || state === "resuming");
  const announcement = useAnnouncement(state);
  if (!barState) return null;
  const seconds =
    state === "readOnly" ? readOnlyElapsedSeconds : rec.view && rec.view.startedUnixMs != null ? recordedSeconds(rec.view, rec.now, rec.readAt) : undefined;
  const showMeter = state === "recording" || state === "paused" || state === "resuming" || state === "problem";
  return (
    <div className="th-rec">
      <WRecordingIndicator
        size="bar"
        state={barState}
        elapsedSeconds={seconds}
        reason={state === "problem" ? t("Some audio may be missing") : undefined}
        meter={showMeter ? <WLevelMeter level={level} arriving={audioIsArriving(level)} idle={state === "paused"} /> : undefined}
        actions={
          controls.pause || controls.stop ? (
            <>
              {controls.pause ? (
                <WButton icon={controls.pause === "pause" ? "pause" : "play"} disabled={controls.pauseDisabled} onClick={controls.pause === "pause" ? rec.pause : rec.resume}>
                  {controls.pause === "pause" ? t("Pause") : t("Resume")}
                </WButton>
              ) : null}
              {controls.stop ? (
                <WButton icon="stop" variant="primary" disabled={controls.stopDisabled} onClick={rec.stop}>
                  {t("Stop")}
                </WButton>
              ) : null}
            </>
          ) : undefined
        }
      />
      {state === "processing" && stage ? (
        <p role="status" className="th-rec-note">
          {stage}
        </p>
      ) : null}
      <span className="w-visually-hidden" role="status" aria-live="polite">
        {announcement}
      </span>
    </div>
  );
}
