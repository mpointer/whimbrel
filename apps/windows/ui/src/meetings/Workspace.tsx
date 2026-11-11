import { type MutableRefObject, useCallback, useEffect, useRef, useState } from "react";
import {
  meetingForget,
  meetingRecordStart,
  meetingRecordStop,
  meetingRegenerate,
  meetingRegenerationConflict,
  meetingResetOverrides,
  meetingSaveNotes,
  meetingSaveOverrides,
  meetingSeedActionItems,
  meetingSetSpokenLanguage,
  meetingTranscribeAgain,
  meetingTrimInfo,
  meetingWorkspace,
} from "../api";
import { describeError, isCommandError } from "../errors";
import type { ActionItemView, LiveTranscriptView, TrimInfo, WorkspaceView } from "../generated/types";
import { TrimSheet } from "./TrimSheet";
import { useL10n } from "../l10n";
import { DeviceChip } from "../library/LibraryView";
import { spanLabel } from "../library/rows";
import { ActionItems } from "./ActionItems";
import { elapsedLabel } from "./recordingModel";
import { meetingTitleLabel } from "./sentinels";
import { RecordingNoticeCard } from "./RecordingNoticeCard";
import { LiveAlertsSection } from "./LiveAlertsSection";
import { LiveTranscriptPane } from "./LiveTranscriptPane";
import { MeetingAskBar } from "./MeetingAskBar";
import { disabledReason } from "./meetingAskModel";
import { SlidesStrip } from "./SlidesStrip";
import { TranscriptPanel } from "./TranscriptPanel";
import { TopicChip, useTopicLookup } from "../topics/TopicChip";
import { overrideOf, phaseTitle, transcriptLanguageOptions, transcriptionNoticeText } from "./workspaceModel";

/** The Mac's autosave delay for the workspace's editors. */
const AUTOSAVE_MS = 1_500;

function message(e: unknown): string {
  return isCommandError(e) ? describeError(e) : String(e);
}

/** Saves `value` once the user stops typing for `AUTOSAVE_MS`. A failed save is shown:
 *  silently losing an edit is worse than an error line. Only the user's edits save: `stored`
 *  is what the shell has (set by the workspace's load, and by each save), and a value equal to
 *  it doesn't save. Nor does a new `save` alone, so a summary that changes underneath (a
 *  transcription run finishing) never re-saves stale text over it. */
function useAutosave(
  value: string | null,
  save: (value: string) => Promise<unknown>,
  onError: (message: string) => void,
  stored: MutableRefObject<string | null>,
) {
  const latestSave = useRef(save);
  useEffect(() => {
    latestSave.current = save;
  }, [save]);
  useEffect(() => {
    if (value == null || value === stored.current) return;
    const timer = setTimeout(() => {
      stored.current = value;
      void latestSave.current(value).catch((e: unknown) => onError(message(e)));
    }, AUTOSAVE_MS);
    return () => clearTimeout(timer);
  }, [value, onError, stored]);
}

export function Workspace({ meetingId, onChanged }: { meetingId: number; onChanged: () => void }) {
  const [view, setView] = useState<WorkspaceView | null>(null);
  const [notes, setNotes] = useState<string | null>(null);
  const [summary, setSummary] = useState<string | null>(null);
  const [items, setItems] = useState<ActionItemView[]>([]);
  const [conflict, setConflict] = useState(false);
  // A speaker's name changed: the summary may still use the old one.
  const [namesChanged, setNamesChanged] = useState(false);
  const [forgetting, setForgetting] = useState(false);
  // "Trim recording…": whether it's offered here, and the sheet while it's open.
  const [trim, setTrim] = useState<TrimInfo | null>(null);
  const [trimming, setTrimming] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [now, setNow] = useState(Date.now());
  const { t, locale } = useL10n();
  // Bumped when a transcription run on this meeting finishes, so the transcript reloads.
  const [transcriptRound, setTranscriptRound] = useState(0);
  const lastStage = useRef<string | null>(null);
  const loadedNotes = useRef<string | null>(null);
  const loadedSummary = useRef<string | null>(null);
  // The meeting's topic (topics piece 2), at the top of the page as on the Mac.
  const [topic] = useTopicLookup([{ kind: "meeting", id: meetingId }]);
  // The live transcript as last polled: the meeting Ask bar waits for its first lines.
  const [liveView, setLiveView] = useState<LiveTranscriptView | null>(null);
  // Bumped by a meeting Ask citation to open the transcript panel.
  const [transcriptOpen, setTranscriptOpen] = useState(0);
  const liveBox = useRef<HTMLDivElement>(null);
  const transcriptBox = useRef<HTMLDivElement>(null);

  const load = useCallback(
    (reset: boolean) =>
      meetingWorkspace(meetingId)
        .then((next) => {
          setView(next);
          // A run that just finished (a new transcript language, say) replaced the summary and
          // the transcript. The new summary is shown unless the user has an unsaved edit; notes
          // are the user's own, so they stay as typed.
          const finished = lastStage.current != null && next?.processingStage == null;
          lastStage.current = next?.processingStage ?? null;
          if (finished) {
            // The panel remounts closed; an old citation click must not reopen it.
            setTranscriptRound((n) => n + 1);
            setTranscriptOpen(0);
          }
          if (next && reset) {
            loadedNotes.current = next.personalNotes;
            loadedSummary.current = next.summary ?? "";
            setNotes(next.personalNotes);
            setSummary(next.summary ?? "");
          } else if (next && finished) {
            const fresh = next.summary ?? "";
            setSummary((current) => {
              if (current !== loadedSummary.current) return current;
              loadedSummary.current = fresh;
              return fresh;
            });
          }
        })
        .catch((e: unknown) => setError(message(e))),
    [meetingId],
  );

  useEffect(() => {
    setNotes(null);
    setSummary(null);
    setConflict(false);
    setForgetting(false);
    setLiveView(null);
    setTranscriptOpen(0);
    void load(true);
    // Seeding is idempotent in core: it only fills an empty list from the summary.
    meetingSeedActionItems(meetingId)
      .then(setItems)
      .catch(() => {});
  }, [meetingId, load]);

  useEffect(() => {
    // While something is recording or processing, the header and stage line move.
    const timer = setInterval(() => {
      setNow(Date.now());
      void load(false);
    }, 2_000);
    return () => clearInterval(timer);
  }, [load]);

  // Re-asked when the meeting's phase or stage changes: trim waits for the recording to stop.
  const phase = view?.phase;
  const stage = view?.processingStage ?? null;
  useEffect(() => {
    setTrimming(false);
    meetingTrimInfo(meetingId).then(setTrim, () => setTrim(null));
  }, [meetingId, phase, stage]);

  const saveNotes = useCallback((value: string) => meetingSaveNotes(meetingId, value), [meetingId]);
  useAutosave(notes, saveNotes, setError, loadedNotes);
  // The generated summary this edit is measured against: core keeps an override until it
  // is reset, so "same as generated" means clear it rather than store a copy.
  const generated = view?.summaryIsEdited ? null : (view?.summary ?? null);
  const saveSummary = useCallback(
    (value: string) => {
      const override = overrideOf(value, generated);
      return override == null ? meetingResetOverrides(meetingId) : meetingSaveOverrides(meetingId, override, null, null);
    },
    [meetingId, generated],
  );
  useAutosave(summary, saveSummary, setError, loadedSummary);

  if (!view) {
    return <section className="card">{error ? <p role="alert">{error}</p> : t("Loading…")}</section>;
  }

  const act = (run: () => Promise<unknown>) => {
    setBusy(true);
    setError(null);
    run()
      .then(() => load(true))
      .then(onChanged)
      .catch((e: unknown) => setError(message(e)))
      .finally(() => setBusy(false));
  };

  const regenerate = (keepEdits: boolean) => {
    setConflict(false);
    act(() => meetingRegenerate(meetingId, keepEdits));
  };

  const askThenRegenerate = () => {
    setBusy(true);
    meetingRegenerationConflict(meetingId)
      .then((needsPrompt) => (needsPrompt ? setConflict(true) : regenerate(false)))
      .catch((e: unknown) => setError(message(e)))
      .finally(() => setBusy(false));
  };

  // The stored title can be core's English placeholder; it's shown translated, never saved back.
  const title = meetingTitleLabel(view.row.title);
  return (
    <section className="card" aria-label={t("{title} workspace", { title })}>
      <header className="workspace-header">
        <h2 className="section-label">{phaseTitle(view)}</h2>
        {topic && <TopicChip topic={topic} />}
        <h3 className="row-title">{title}</h3>
        <p className="row-meta">
          {spanLabel(view.row.tsUnixMs, view.row.endUnixMs, { full: true, locale })} <DeviceChip device={view.row.device} />
        </p>
        {view.attendees.length > 0 && <p className="muted small">{view.attendees.join(", ")}</p>}
        {view.conferenceLink && (
          <p className="small">
            <a href={view.conferenceLink}>{view.conferenceLink}</a>
          </p>
        )}
        {view.processingStage && <p role="status">{view.processingStage}</p>}
      </header>

      {view.phase === "after" && <SlidesStrip meetingId={meetingId} refresh={now} />}

      {view.phase === "before" && (
        <div className="toolbar">
          <button className="mode mode--active" disabled={busy || view.readOnly} onClick={() => act(meetingRecordStart)}>
            {t("Start recording")}
          </button>
        </div>
      )}

      {view.phase === "during" && (
        <>
          <p role="status" className="recording-line">
            <span className="recording-dot" aria-hidden="true" />
            {t("Recording · {elapsed}", { elapsed: elapsedLabel(view.row.tsUnixMs, now) })}
          </p>
          {!view.readOnly && <RecordingNoticeCard meetingId={meetingId} />}
          <LiveAlertsSection meetingId={meetingId} recording={true} endedUnixMs={null} />
          {/* The slides so far, live, under the notices and above the transcript (the Mac's in-meeting view). */}
          <SlidesStrip meetingId={meetingId} refresh={now} />
          <div ref={liveBox}>
            <LiveTranscriptPane meetingId={meetingId} onView={setLiveView} />
          </div>
          {/* With the live transcript off there is nothing to ask about until the call ends. */}
          {liveView?.enabled && (
            <MeetingAskBar
              key={`live-${meetingId}`}
              meetingId={meetingId}
              live={true}
              disabledReason={disabledReason({ recording: true, liveLines: liveView.lines.length, processing: false, hasTranscript: false })}
              onCite={() => liveBox.current?.scrollIntoView({ block: "nearest" })}
            />
          )}
          <label className="field">
            <span>{t("Notes")}</span>
            <textarea className="input" rows={6} value={notes ?? ""} onChange={(e) => setNotes(e.target.value)} />
          </label>
          <div className="toolbar">
            <button className="mode" disabled={busy} onClick={() => act(meetingRecordStop)}>
              {t("Stop recording")}
            </button>
          </div>
        </>
      )}

      {view.phase === "after" && (
        <>
          <label className="field">
            <span>
              {t("Summary")} {view.summaryIsEdited && <em className="muted small">{t("(edited)")}</em>}
            </span>
            <textarea
              className="input"
              rows={5}
              value={summary ?? ""}
              readOnly={view.readOnly}
              onChange={(e) => setSummary(e.target.value)}
            />
          </label>

          {view.decisions.length > 0 && (
            <div className="field">
              <span>{t("Decisions")}</span>
              <ul className="bullets">
                {view.decisions.map((d, i) => (
                  <li key={i}>{d}</li>
                ))}
              </ul>
            </div>
          )}
          {view.openQuestions.length > 0 && (
            <div className="field">
              <span>{t("Open questions")}</span>
              <ul className="bullets">
                {view.openQuestions.map((q, i) => (
                  <li key={i}>{q}</li>
                ))}
              </ul>
            </div>
          )}

          <ActionItems meetingId={meetingId} items={items} onItems={setItems} readOnly={view.readOnly} />

          <label className="field">
            <span>{t("Your notes")}</span>
            <textarea
              className="input"
              rows={4}
              value={notes ?? ""}
              readOnly={view.readOnly}
              onChange={(e) => setNotes(e.target.value)}
            />
          </label>

          {view.transcription && (
            <div className="field" role="status">
              <p className="muted small">{transcriptionNoticeText(view.transcription)}</p>
              <div className="toolbar">
                <button
                  className="mode"
                  disabled={busy || view.processingStage != null}
                  onClick={() => act(() => meetingTranscribeAgain(meetingId))}
                >
                  {t("Transcribe again")}
                </button>
              </div>
            </div>
          )}

          {/* After the meeting it keeps polling for half an hour, while drafts may still land. */}
          <LiveAlertsSection meetingId={meetingId} recording={false} endedUnixMs={view.row.endUnixMs ?? null} />

          {namesChanged && !view.readOnly && (
            <div className="field-row" role="status">
              <span>{t("Names changed. Update the summary?")}</span>
              <button
                className="mode mode--active"
                disabled={busy || view.processingStage != null}
                onClick={() => {
                  setNamesChanged(false);
                  askThenRegenerate();
                }}
              >
                {t("Update the summary")}
              </button>
            </div>
          )}

          <MeetingAskBar
            key={`after-${meetingId}`}
            meetingId={meetingId}
            live={false}
            disabledReason={disabledReason({
              recording: false,
              liveLines: 0,
              processing: view.processingStage != null,
              hasTranscript: view.hasTranscript,
            })}
            onCite={() => {
              // No scroll-to-line yet (plan Decision 7): a citation opens the transcript.
              setTranscriptOpen((n) => n + 1);
              transcriptBox.current?.scrollIntoView({ block: "start" });
            }}
          />

          <div ref={transcriptBox}>
            <TranscriptPanel
              key={transcriptRound}
              meetingId={meetingId}
              hasTranscript={view.hasTranscript}
              onNamesChanged={() => setNamesChanged(true)}
              openRequest={transcriptOpen}
            />
          </div>

          {view.canChooseLanguage && (
            <label className="field-row">
              <span>{t("Transcript language")}</span>
              <select
                className="input"
                value={view.spokenLanguage ?? ""}
                disabled={busy || view.processingStage != null}
                // A new choice transcribes the meeting again in it; Automatic detects again.
                onChange={(e) => act(() => meetingSetSpokenLanguage(meetingId, e.target.value || null))}
              >
                {transcriptLanguageOptions(view.spokenLanguage ?? null, t("Automatic")).map((option) => (
                  <option key={option.code} value={option.code}>
                    {option.label}
                  </option>
                ))}
              </select>
            </label>
          )}

          {view.summaryFailure != null && (
            <p className="muted small" role="status">
              {t("The summary didn't finish. Regenerate summary tries again. What went wrong: {reason}", { reason: view.summaryFailure })}
            </p>
          )}
          <div className="toolbar">
            {!view.readOnly && (
              <button className="mode" disabled={busy || !view.hasTranscript} onClick={askThenRegenerate}>
                {t("Regenerate summary")}
              </button>
            )}
            {trim?.offered && !view.readOnly && (
              <button className="mode" disabled={busy || view.processingStage != null} onClick={() => setTrimming(true)}>
                {t("Trim recording…")}
              </button>
            )}
            <button className="mode" disabled={busy} onClick={() => setForgetting(true)}>
              {t("Forget meeting")}
            </button>
          </div>
          {trimming && trim && (
            <TrimSheet
              meetingId={meetingId}
              lengthMs={trim.lengthMs}
              onCancel={() => setTrimming(false)}
              onTrimmed={() => {
                setTrimming(false);
                setTranscriptRound((n) => n + 1);
                setTranscriptOpen(0);
                // The summary is redone from what's left, as Regenerate does.
                askThenRegenerate();
              }}
            />
          )}
          {view.readOnly && <p className="muted small">{t("Recorded on another device, so its audio and summary live there.")}</p>}
        </>
      )}

      {conflict && (
        <div className="card confirm" role="alertdialog" aria-label={t("New summary")}>
          <p>{t("You've edited this meeting. What should happen to your edits?")}</p>
          <div className="toolbar">
            <button className="mode" onClick={() => regenerate(true)}>
              {t("Keep my edits")}
            </button>
            <button className="mode mode--active" onClick={() => regenerate(false)}>
              {t("Replace with the new summary")}
            </button>
            <button className="mode" onClick={() => setConflict(false)}>
              {t("Cancel")}
            </button>
          </div>
        </div>
      )}

      {forgetting && (
        <div className="card confirm" role="alertdialog" aria-label={t("Forget meeting")}>
          {/* Deletion sync (PR 584): forgetting removes the meeting on every device, as on the Mac. */}
          <p>
            <strong>{t("Forget this meeting?")}</strong>
          </p>
          <p>{t("This removes it from all your devices.")}</p>
          <div className="toolbar">
            <button className="mode mode--active" disabled={busy} onClick={() => act(() => meetingForget(meetingId))}>
              {t("Forget it")}
            </button>
            <button className="mode" onClick={() => setForgetting(false)}>
              {t("Keep it")}
            </button>
          </div>
        </div>
      )}

      {error && <p role="alert">{error}</p>}
    </section>
  );
}
