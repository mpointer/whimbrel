import { useCallback, useEffect, useRef, useState } from "react";
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
  meetingWorkspace,
} from "../api";
import { describeError, isCommandError } from "../errors";
import type { ActionItemView, WorkspaceView } from "../generated/types";
import { DeviceChip } from "../library/LibraryView";
import { spanLabel } from "../library/rows";
import { ActionItems } from "./ActionItems";
import { elapsedLabel } from "./recordingModel";
import { SlidesStrip } from "./SlidesStrip";
import { TranscriptPanel } from "./TranscriptPanel";
import { overrideOf, phaseTitle } from "./workspaceModel";

/** The Mac's autosave delay for the workspace's editors. */
const AUTOSAVE_MS = 1_500;

function message(e: unknown): string {
  return isCommandError(e) ? describeError(e) : String(e);
}

/** Saves `value` once the user stops typing for `AUTOSAVE_MS`. A failed save is shown:
 *  silently losing an edit is worse than an error line. */
function useAutosave(value: string | null, save: (value: string) => Promise<unknown>, onError: (message: string) => void) {
  const first = useRef(true);
  useEffect(() => {
    if (value == null) return;
    if (first.current) {
      first.current = false;
      return;
    }
    const timer = setTimeout(() => void save(value).catch((e: unknown) => onError(message(e))), AUTOSAVE_MS);
    return () => clearTimeout(timer);
  }, [value, save, onError]);
}

export function Workspace({ meetingId, onChanged }: { meetingId: number; onChanged: () => void }) {
  const [view, setView] = useState<WorkspaceView | null>(null);
  const [notes, setNotes] = useState<string | null>(null);
  const [summary, setSummary] = useState<string | null>(null);
  const [items, setItems] = useState<ActionItemView[]>([]);
  const [conflict, setConflict] = useState(false);
  const [forgetting, setForgetting] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [now, setNow] = useState(Date.now());

  const load = useCallback(
    (reset: boolean) =>
      meetingWorkspace(meetingId)
        .then((next) => {
          setView(next);
          if (next && reset) {
            setNotes(next.personalNotes);
            setSummary(next.summary ?? "");
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

  const saveNotes = useCallback((value: string) => meetingSaveNotes(meetingId, value), [meetingId]);
  useAutosave(notes, saveNotes, setError);
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
  useAutosave(summary, saveSummary, setError);

  if (!view) {
    return <section className="card">{error ? <p role="alert">{error}</p> : "Loading…"}</section>;
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

  return (
    <section className="card" aria-label={`${view.row.title} workspace`}>
      <header className="workspace-header">
        <h2 className="section-label">{phaseTitle(view)}</h2>
        <h3 className="row-title">{view.row.title}</h3>
        <p className="row-meta">
          {spanLabel(view.row.tsUnixMs, view.row.endUnixMs, { full: true })} <DeviceChip device={view.row.device} />
        </p>
        {view.attendees.length > 0 && <p className="muted small">{view.attendees.join(", ")}</p>}
        {view.conferenceLink && (
          <p className="small">
            <a href={view.conferenceLink}>{view.conferenceLink}</a>
          </p>
        )}
        {view.processingStage && <p role="status">{view.processingStage}</p>}
      </header>

      {(view.phase === "during" || view.phase === "after") && <SlidesStrip meetingId={meetingId} refresh={now} />}

      {view.phase === "before" && (
        <div className="toolbar">
          <button className="mode mode--active" disabled={busy || view.readOnly} onClick={() => act(meetingRecordStart)}>
            Start recording
          </button>
        </div>
      )}

      {view.phase === "during" && (
        <>
          <p role="status" className="recording-line">
            <span className="recording-dot" aria-hidden="true" />
            Recording · {elapsedLabel(view.row.tsUnixMs, now)}
          </p>
          <label className="field">
            <span>Notes</span>
            <textarea className="input" rows={6} value={notes ?? ""} onChange={(e) => setNotes(e.target.value)} />
          </label>
          <div className="toolbar">
            <button className="mode" disabled={busy} onClick={() => act(meetingRecordStop)}>
              Stop recording
            </button>
          </div>
        </>
      )}

      {view.phase === "after" && (
        <>
          <label className="field">
            <span>
              Summary {view.summaryIsEdited && <em className="muted small">(edited)</em>}
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
              <span>Decisions</span>
              <ul className="bullets">
                {view.decisions.map((d, i) => (
                  <li key={i}>{d}</li>
                ))}
              </ul>
            </div>
          )}
          {view.openQuestions.length > 0 && (
            <div className="field">
              <span>Open questions</span>
              <ul className="bullets">
                {view.openQuestions.map((q, i) => (
                  <li key={i}>{q}</li>
                ))}
              </ul>
            </div>
          )}

          <ActionItems meetingId={meetingId} items={items} onItems={setItems} readOnly={view.readOnly} />

          <label className="field">
            <span>Your notes</span>
            <textarea
              className="input"
              rows={4}
              value={notes ?? ""}
              readOnly={view.readOnly}
              onChange={(e) => setNotes(e.target.value)}
            />
          </label>

          <TranscriptPanel meetingId={meetingId} hasTranscript={view.hasTranscript} />

          <div className="toolbar">
            {!view.readOnly && (
              <button className="mode" disabled={busy || !view.hasTranscript} onClick={askThenRegenerate}>
                Regenerate summary
              </button>
            )}
            <button className="mode" disabled={busy} onClick={() => setForgetting(true)}>
              Forget meeting
            </button>
          </div>
          {view.readOnly && <p className="muted small">Recorded on another device, so its audio and summary live there.</p>}
        </>
      )}

      {conflict && (
        <div className="card confirm" role="alertdialog" aria-label="New summary">
          <p>You've edited this meeting. What should happen to your edits?</p>
          <div className="toolbar">
            <button className="mode" onClick={() => regenerate(true)}>
              Keep my edits
            </button>
            <button className="mode mode--active" onClick={() => regenerate(false)}>
              Replace with the new summary
            </button>
            <button className="mode" onClick={() => setConflict(false)}>
              Cancel
            </button>
          </div>
        </div>
      )}

      {forgetting && (
        <div className="card confirm" role="alertdialog" aria-label="Forget meeting">
          <p>Forget this meeting? It removes the recording, transcript and summary from this PC. Copies already synced to other devices stay.</p>
          <div className="toolbar">
            <button className="mode mode--active" disabled={busy} onClick={() => act(() => meetingForget(meetingId))}>
              Forget it
            </button>
            <button className="mode" onClick={() => setForgetting(false)}>
              Keep it
            </button>
          </div>
        </div>
      )}

      {error && <p role="alert">{error}</p>}
    </section>
  );
}
