// The live meeting screen in the Thread look (docs/ui-redesign/screens/live-meeting.md), behind the redesign flag. It takes
// Home's place while a meeting records and is what the Meetings tab shows for the live meeting. Left to right: a short
// time line, the main column (recording bar, warnings, the recording notice once, the note card) and, when the live
// transcript is on, a right column (transcript, question alerts, the ask bar). The wide layout drops the right column
// under the note card, and a narrow one drops the time line, so nothing clips at 200% text.

import { useCallback, useEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { meetingBarTakeAsk, meetingLiveTranscript, meetingMarkerAdd, meetingMarkerDelete, meetingMarkerList, meetingSaveNotes, meetingWorkspace, todaySchedule } from "../../api";
import type { LiveTranscriptView, MeetingMarkerView, ScheduleEvent, WorkspaceView } from "../../generated/types";
import { useL10n } from "../../l10n";
import { timeLabel } from "../../library/rows";
import { setLiveScreenShown, setLiveScreenTitle } from "../../shell/redesign/liveScreen";
import { useRecording, type RecordingControls } from "../../shell/redesign/useRecording";
import { WButton, WIcon, WLoadingState, WConsentBanner, WSpine, WSpineNow, WSpineStop, WTranscriptLine, WTranscriptView } from "../../system";
import { LiveAlertsSection } from "../LiveAlertsSection";
import { followsNewLines, liveClock, LIVE_POLL_MS } from "../liveTranscriptModel";
import { MeetingAskBar } from "../MeetingAskBar";
import { disabledReason } from "../meetingAskModel";
import { dismissNotice, noticeText, noticeVisible, useRecordingNotice } from "../recordingNotice";
import { meetingTitleLabel, speakerLabel } from "../sentinels";
import { useAutosave } from "../Workspace";
import { clockOffsetCs, liveControls, type LiveState } from "./liveModel";
import { markClockMs, markLabel, sortedMarks } from "./marksModel";
import { MARK_EVENT, useRecordingContext } from "./recordingContext";
import { RecordingBar } from "./RecordingBar";
import { startLine } from "./liveText";
import "./liveMeeting.css";

export interface LiveMeetingProps {
  /** The meeting; null while a start is still writing its row. */
  meetingId: number | null;
  /** Another device's meeting: no controls, no banner. Read from the workspace when not given. */
  readOnly?: boolean;
  onOpenSettings?: () => void;
}

/** The latest lines the right column shows. */
const LATEST_LINES = 40;

export function LiveMeeting({ meetingId, readOnly: readOnlyGiven }: LiveMeetingProps) {
  const { t, locale } = useL10n();
  const shared = useRecordingContext();
  const own = useRecording(!shared);
  const rec: RecordingControls = shared ?? own;
  const [workspace, setWorkspace] = useState<WorkspaceView | null>(null);
  const [notes, setNotes] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [next, setNext] = useState<ScheduleEvent | null>(null);
  const [live, setLive] = useState<LiveTranscriptView | null>(null);
  const loadedNotes = useRef<string | null>(null);
  const transcriptBox = useRef<HTMLDivElement>(null);
  const follow = useRef(true);
  const askBox = useRef<HTMLDivElement>(null);
  const [marks, setMarks] = useState<MeetingMarkerView[]>([]);
  const notesBox = useRef<HTMLTextAreaElement>(null);
  const noticeState = useRecordingNotice();
  const [copied, setCopied] = useState(false);
  // A question typed in the in-meeting window: answered here, in the main window.
  const [asked, setAsked] = useState<{ seq: number; text: string } | undefined>(undefined);
  // The shell holds the question until this screen can take it: once the transcript has lines for the ask bar to answer from.
  const [pending, setPending] = useState(0);
  useEffect(() => {
    const stop = listen("meeting-bar-ask-pending", () => setPending((n) => n + 1));
    return () => void stop.then((off) => off());
  }, []);

  // The shell does not draw its own bar above this screen.
  useEffect(() => {
    setLiveScreenShown(true);
    return () => setLiveScreenShown(false);
  }, []);

  useEffect(() => {
    setWorkspace(null);
    setNotes(null);
    loadedNotes.current = null;
    if (meetingId == null) return;
    let alive = true;
    meetingWorkspace(meetingId).then(
      (view) => {
        if (!alive || !view) return;
        setWorkspace(view);
        loadedNotes.current = view.personalNotes;
        setNotes(view.personalNotes);
      },
      (e: unknown) => alive && setError(String(e)),
    );
    return () => {
      alive = false;
    };
  }, [meetingId]);

  // The next meeting for the time line, read once.
  useEffect(() => {
    todaySchedule().then(
      (view) => setNext(view.events.find((e) => e.startUnixMs > Date.now() && e.recordedMeetingId == null) ?? null),
      () => {},
    );
  }, []);

  // The live transcript: polled while the screen shows, as the old pane does.
  useEffect(() => {
    if (meetingId == null) return;
    let alive = true;
    const load = () =>
      meetingLiveTranscript(meetingId).then(
        (v) => {
          if (!alive) return;
          if (transcriptBox.current) follow.current = followsNewLines(transcriptBox.current);
          setLive(v);
        },
        () => {},
      );
    load();
    const timer = setInterval(load, LIVE_POLL_MS);
    return () => {
      alive = false;
      clearInterval(timer);
    };
  }, [meetingId]);
  useEffect(() => {
    if (transcriptBox.current && follow.current) transcriptBox.current.scrollTop = transcriptBox.current.scrollHeight;
  }, [live]);

  // Marked moments: loaded once, then added and deleted here. Ctrl+Shift+M (the shell) and the button both land in `mark`.
  useEffect(() => {
    setMarks([]);
    if (meetingId == null) return;
    meetingMarkerList(meetingId).then((list) => setMarks(list ?? []), () => {});
  }, [meetingId]);
  const markable = useRef(false);
  const mark = useCallback(() => {
    const begun = rec.view?.startedUnixMs ?? workspace?.row.tsUnixMs ?? null;
    if (meetingId == null || begun == null) return;
    const label = markLabel(latestNotes.current ?? "", notesBox.current?.selectionStart ?? null);
    meetingMarkerAdd(meetingId, clockOffsetCs(begun, Date.now()), label).then((made) => setMarks((all) => [...all, made]), (e: unknown) => setError(String(e)));
  }, [meetingId, rec.view?.startedUnixMs, workspace?.row.tsUnixMs]);
  const markRef = useRef(mark);
  markRef.current = mark;
  useEffect(() => {
    const onMark = (e: Event) => {
      if (!markable.current) return;
      e.preventDefault();
      markRef.current();
    };
    window.addEventListener(MARK_EVENT, onMark);
    return () => window.removeEventListener(MARK_EVENT, onMark);
  }, []);

  // The notes: the existing autosave (1.5 s after typing stops), and a save on the way out so a Stop right after typing
  // loses nothing.
  const saveNotes = useCallback((value: string) => (meetingId == null ? Promise.resolve() : meetingSaveNotes(meetingId, value)), [meetingId]);
  useAutosave(notes, saveNotes, setError, loadedNotes);
  const latestNotes = useRef<string | null>(null);
  latestNotes.current = notes;
  useEffect(
    () => () => {
      const value = latestNotes.current;
      if (meetingId != null && value != null && value !== loadedNotes.current) void meetingSaveNotes(meetingId, value).catch(() => {});
    },
    [meetingId],
  );

  const readOnly = readOnlyGiven ?? workspace?.readOnly ?? false;
  const state: LiveState = readOnly ? "readOnly" : rec.state;
  const controls = liveControls(state, rec.busy);
  markable.current = controls.markEnabled && meetingId != null;
  const title = workspace ? meetingTitleLabel(workspace.row.title) : t("Meeting");
  const started = workspace?.row.tsUnixMs ?? rec.view?.startedUnixMs ?? null;
  useEffect(() => {
    setLiveScreenTitle(workspace ? title : null);
  }, [workspace, title]);
  const showRight = live?.enabled === true;
  const canAsk = showRight && (live?.lines.length ?? 0) > 0 && state !== "processing" && state !== "readOnly";
  useEffect(() => {
    if (meetingId == null || !canAsk) return;
    meetingBarTakeAsk(meetingId).then((text) => text && setAsked((a) => ({ seq: (a?.seq ?? 0) + 1, text })), () => {});
  }, [meetingId, canAsk, pending]);
  const lines = live?.lines.slice(-LATEST_LINES) ?? [];
  const noticeOn = !readOnly && state !== "processing" && meetingId != null && noticeVisible(noticeState.settings, meetingId, noticeState.dismissed);

  const copyNotice = () => {
    void navigator.clipboard?.writeText(noticeText(noticeState.settings?.text ?? "")).then(() => setCopied(true), () => {});
  };

  const when = started != null ? startLine(started, rec.now, locale, workspace?.attendees ?? [], t) : "";
  const degraded = rec.view?.degraded ?? null;

  return (
    <div className="th-live" data-state={state} data-columns={showRight ? "3" : "2"}>
      <div className="th-live-grid">
        <div className="th-live-line">
          <WSpine label={t("Time line")}>
            {next ? (
              <WSpineStop time={timeLabel(next.startUnixMs, locale)} timeNote={next.title} node="upcoming" label={`${timeLabel(next.startUnixMs, locale)}, ${next.title}`}>
                {null}
              </WSpineStop>
            ) : null}
            <WSpineNow time={timeLabel(rec.now, locale)} connector="solid" />
            {started != null ? (
              <WSpineStop time={timeLabel(started, locale)} timeNote={t("Started")} node="recording" last label={`${timeLabel(started, locale)}, ${t("Started")}`}>
                {null}
              </WSpineStop>
            ) : null}
          </WSpine>
        </div>

        <div className="th-live-main">
          <RecordingBar rec={rec} state={state} readOnlyElapsedSeconds={started != null ? Math.max(0, Math.floor((rec.now - started) / 1000)) : undefined} stage={rec.view?.processing?.label ?? null} />
          {degraded && state !== "readOnly" ? (
            <p role="status" className="th-warning">
              <WIcon name="warning" />
              <span>{degraded}</span>
            </p>
          ) : null}
          {noticeOn ? (
            <WConsentBanner
              copied={copied}
              onCopy={copyNotice}
              onDone={() => {
                if (meetingId != null) dismissNotice(meetingId);
              }}
            />
          ) : null}

          {meetingId == null ? (
            <div className="th-note-card">
              <WLoadingState title={t("Starting…")} />
            </div>
          ) : (
            <section className="th-note-card" aria-label={t("{title} workspace", { title })}>
              <header className="th-note-head">
                <div className="th-note-titles">
                  <h2 className="th-note-title">{title}</h2>
                  {when ? <p className="th-note-when">{when}</p> : null}
                </div>
                {!readOnly && state !== "processing" ? (
                  <WButton icon="flag" hint="Ctrl+Shift+M" disabled={!controls.markEnabled} onClick={mark}>
                    {t("Mark this moment")}
                  </WButton>
                ) : null}
              </header>
              <textarea
                ref={notesBox}
                className="th-notes"
                aria-label={t("Notes")}
                placeholder={readOnly ? "" : t("Jot notes as you go.")}
                value={notes ?? ""}
                readOnly={readOnly}
                onChange={(e) => setNotes(e.target.value)}
              />
              {marks.length > 0 ? (
                <section className="th-marks" aria-label={t("Moments you marked")}>
                  <h3 className="th-marks-head">
                    <WIcon name="flag" size={16} />
                    {t("Moments you marked")}
                    <span className="th-marks-count">{marks.length}</span>
                  </h3>
                  <ul>
                    {sortedMarks(marks).map((m) => (
                      <li key={m.id}>
                        <span className="th-marks-time">{started != null ? timeLabel(markClockMs(started, m.atCs), locale) : liveClock(m.atCs)}</span>
                        <span className="th-marks-label">{m.label ?? t("Marked moment")}</span>
                        {!readOnly ? (
                          <button
                            type="button"
                            className="th-marks-delete"
                            aria-label={t("Remove this mark")}
                            onClick={() => void meetingMarkerDelete(m.id).then(() => setMarks((all) => all.filter((x) => x.id !== m.id)), () => {})}
                          >
                            <WIcon name="close" size={14} />
                          </button>
                        ) : null}
                      </li>
                    ))}
                  </ul>
                </section>
              ) : null}
              {error ? <p role="alert">{error}</p> : null}
              <footer className="th-note-foot">
                <span>
                  <WIcon name="mic" size={16} />
                  {t("Audio stays on this PC.")}
                </span>
                <span>
                  <WIcon name="spark" size={16} />
                  {t("Whimbrel tidies your notes when you stop.")}
                </span>
              </footer>
            </section>
          )}
        </div>

        {showRight && meetingId != null ? (
          <aside className="th-live-side" aria-label={t("Live transcript")}>
            <LiveAlertsSection meetingId={meetingId} recording={state !== "processing"} endedUnixMs={null} />
            <section className="th-transcript-card">
              <header className="th-transcript-head">
                <h2>
                  <WIcon name="chat" size={16} />
                  {t("Live transcript")}
                </h2>
              </header>
              {lines.length === 0 ? (
                <p className="th-quiet" role="status">
                  {t("Listening. The transcript starts appearing after the first minute.")}
                </p>
              ) : (
                <div ref={transcriptBox} className="th-transcript-scroll">
                  <WTranscriptView label={t("Live transcript")}>
                    {lines.map((line, i) => (
                      <WTranscriptLine key={line.id} time={started != null ? timeLabel(started + line.startCs * 10, locale) : liveClock(line.startCs)} speaker={speakerLabel(line.speaker)} text={line.text} state={i === lines.length - 1 && state === "recording" ? "cited" : "normal"} />
                    ))}
                  </WTranscriptView>
                </div>
              )}
            </section>
            <div ref={askBox} className="th-ask-box">
              <MeetingAskBar
                key={`live-${meetingId}`}
                request={asked}
                meetingId={meetingId}
                live={true}
                disabledReason={disabledReason({ recording: state !== "processing", liveLines: live?.lines.length ?? 0, processing: state === "processing", hasTranscript: false })}
                onCite={() => transcriptBox.current?.scrollIntoView({ block: "nearest" })}
              />
            </div>
          </aside>
        ) : null}
      </div>
    </div>
  );
}
