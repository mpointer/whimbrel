// The in-meeting window's page (redesign flag on only; the shell opens the window only then): meeting title, state word, recorded
// time, level meter, Pause or Resume, Stop, an ask box and the audio line. The ask box hands the question to the main window,
// where the answer shows on the live meeting screen. The whole top strip drags the window.

import { useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { meetingBarAsk, meetingBarCancelAsk, meetingLiveTranscript, meetingWorkspace } from "../api";
import { ASK_WAIT_MS, askOutcome } from "./askHandoff";
import { useL10n } from "../l10n";
import { meetingTitleLabel } from "../meetings/sentinels";
import { audioIsArriving, liveControls, recordedSeconds } from "../meetings/redesign/liveModel";
import { useAudioLevel } from "../meetings/redesign/useAudioLevel";
import { useAnnouncement } from "../meetings/redesign/RecordingBar";
import { WButton, WIcon, WLevelMeter, WRoot, WINDOW_DESIGN, formatClock, recordingWord, spokenDuration, useCompactWindow, type WRecordingState } from "../system";
import { useRecording } from "../shell/redesign/useRecording";
import "./meetingBar.css";

const WORD: Record<string, WRecordingState> = { recording: "recording", paused: "paused", resuming: "resuming", starting: "starting", problem: "problem", processing: "processing" };

export function MeetingBarView() {
  const { t } = useL10n();
  // The shell grows this window with the Windows text size (meeting_bar.rs). When the work area capped it, the compact layout keeps the
  // state word, the time, Pause or Resume and Stop and drops what can wait (the level meter, the footer, the "always on top" note).
  const compact = useCompactWindow(WINDOW_DESIGN.meetingBar);
  const rec = useRecording(true);
  const [title, setTitle] = useState<string | null>(null);
  const [askOn, setAskOn] = useState(false);
  const [question, setQuestion] = useState("");
  const [sending, setSending] = useState(false);
  const [kept, setKept] = useState(false);
  const meetingId = rec.view?.meetingId ?? null;
  useEffect(() => {
    if (meetingId == null) return;
    meetingWorkspace(meetingId).then((v) => v && setTitle(meetingTitleLabel(v.row.title)), () => {});
    meetingLiveTranscript(meetingId).then((v) => setAskOn(v.enabled), () => {});
  }, [meetingId]);
  const level = useAudioLevel(rec.state === "recording" || rec.state === "problem" || rec.state === "resuming");
  const announcement = useAnnouncement(rec.state);
  const controls = liveControls(rec.state, rec.busy);
  const barState = WORD[rec.state] ?? "recording";
  const seconds = rec.view && rec.view.startedUnixMs != null ? recordedSeconds(rec.view, rec.now, rec.readAt) : 0;
  const ask = () => {
    const text = question.trim();
    if (!text || meetingId == null || sending) return;
    // The text stays in the box until the main window says it took the question; it is never cleared on a guess.
    setSending(true);
    setKept(false);
    const began = Date.now();
    let acknowledged = false;
    const stop = listen<number>("meeting-bar-ask-taken", () => {
      acknowledged = true;
      setQuestion("");
      setSending(false);
    });
    const finish = () => {
      void stop.then((off) => off());
      if (askOutcome(Date.now() - began, acknowledged) === "kept") {
        void meetingBarCancelAsk().catch(() => {});
        setKept(true);
        setSending(false);
      }
    };
    meetingBarAsk(meetingId, text).catch(() => {
      acknowledged = false;
    });
    setTimeout(finish, ASK_WAIT_MS);
  };
  return (
    <WRoot className="mb-root">
      <main className={compact ? "mb mb--compact" : "mb"} aria-label={t("Whimbrel, in-meeting window")} data-state={rec.state}>
        <header className="mb-head" data-tauri-drag-region>
          <span className="mb-title" data-tauri-drag-region>
            {title ?? t("Meeting")}
          </span>
          <span className="mb-top" data-tauri-drag-region>
            {t("Always on top")}
          </span>
        </header>
        {/* Not a live region: one whose name held the ticking time was announced every second, so Narrator said "Whimbrel" over and
            over through a recording. State changes are announced once, by the hidden region at the end (`useAnnouncement`). */}
        <div className="mb-state">
          <span className="mb-word" data-state={barState}>
            {recordingWord(barState)}
          </span>
          <span className="mb-time" role="timer" aria-label={spokenDuration(seconds)}>
            {formatClock(seconds)}
          </span>
        </div>
        <div className="mb-meter">
          <WLevelMeter level={level} arriving={audioIsArriving(level)} idle={rec.state === "paused"} bars={28} />
        </div>
        <div className="mb-buttons">
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
        </div>
        {askOn ? (
          <form
            className="mb-ask"
            onSubmit={(e) => {
              e.preventDefault();
              ask();
            }}
          >
            <WIcon name="spark" size={16} />
            <input value={question} onChange={(e) => setQuestion(e.target.value)} aria-label={t("Ask about this meeting")} placeholder={t("Ask about this meeting")} />
            <button type="submit" className="mb-send" aria-label={t("Ask")} disabled={!question.trim() || sending}>
              <WIcon name="chevron" size={16} />
            </button>
          </form>
        ) : null}
        {kept ? (
          <p role="status" className="mb-kept">
            {t("Open Whimbrel to see the answer.")}
          </p>
        ) : null}
        <p className="mb-foot">
          <WIcon name="mic" size={14} />
          {t("Audio stays on this PC.")}
        </p>
        <span className="w-visually-hidden" role="status" aria-live="polite">
          {announcement}
        </span>
      </main>
    </WRoot>
  );
}
