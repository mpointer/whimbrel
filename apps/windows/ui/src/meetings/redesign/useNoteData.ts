// What the finished note holds and does (redesign flag on only): the workspace, the notes and summary being edited, the action
// items, the sources, the marks, and the actions that change them. It is Workspace.tsx's logic for a finished meeting, in a hook,
// with the same commands, the same autosave and the same regenerate conflict question; the screen only draws it differently.

import { useCallback, useEffect, useRef, useState } from "react";
import {
  meetingForget,
  meetingMarkerDelete,
  meetingMarkerList,
  meetingMarkerSetLabel,
  meetingNoteSources,
  meetingRegenerate,
  meetingRegenerationConflict,
  meetingResetOverrides,
  meetingResummarizeWhimbrel,
  meetingSaveNotes,
  meetingSaveOverrides,
  meetingSeedActionItems,
  meetingSetSpokenLanguage,
  meetingTranscribeAgain,
  meetingTrimInfo,
  meetingWorkspace,
} from "../../api";
import { describeError, isCommandError } from "../../errors";
import type { ActionItemView, MeetingMarkerView, NoteSourcesView, TrimInfo, WorkspaceView } from "../../generated/types";
import { useAutosave } from "../Workspace";
import { overrideOf } from "../workspaceModel";
import { sortedMarks } from "./marksModel";

const message = (e: unknown): string => (isCommandError(e) ? describeError(e) : String(e));

/** How often the page asks again while something may be moving (a stage, a finished run). The old workspace's two seconds. */
const POLL_MS = 2_000;

export function useNoteData(meetingId: number, onChanged: () => void) {
  const [view, setView] = useState<WorkspaceView | null>(null);
  const [notes, setNotes] = useState<string | null>(null);
  const [summary, setSummary] = useState<string | null>(null);
  const [items, setItemsState] = useState<ActionItemView[]>([]);
  const [sources, setSources] = useState<NoteSourcesView | null>(null);
  const [marks, setMarks] = useState<MeetingMarkerView[]>([]);
  const [trim, setTrim] = useState<TrimInfo | null>(null);
  const [conflict, setConflict] = useState(false);
  const [viaWhimbrel, setViaWhimbrel] = useState(false);
  const [resummarizeError, setResummarizeError] = useState<string | null>(null);
  const [namesChanged, setNamesChanged] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  // Action items whose text the person changed here: theirs from then on, so no AI ink and no source.
  const [editedItems, setEditedItems] = useState<ReadonlySet<number>>(new Set());
  // Bumped when the transcript may have changed under the page (a run finished, a language, a trim), so its view reads it again.
  const [transcriptRound, setTranscriptRound] = useState(0);
  const lastStage = useRef<string | null>(null);
  const loadedNotes = useRef<string | null>(null);
  const loadedSummary = useRef<string | null>(null);
  const items_ = useRef<ActionItemView[]>([]);
  items_.current = items;
  // Answers that arrive after the meeting changed, or out of order, are dropped.
  const current = useRef(meetingId);
  current.current = meetingId;
  const loadSeq = useRef(0);
  const inflight = useRef(false);
  const resetPending = useRef(false);
  const sourcesSeq = useRef(0);
  const lastView = useRef("");

  const loadSources = useCallback(() => {
    const mine = ++sourcesSeq.current;
    meetingNoteSources(meetingId).then(
      (next) => mine === sourcesSeq.current && current.current === meetingId && setSources(next),
      () => mine === sourcesSeq.current && current.current === meetingId && setSources(null),
    );
  }, [meetingId]);

  const load = useCallback(
    (reset: boolean) => {
      // A poll never stacks on a poll still waiting.
      if (!reset && inflight.current) return Promise.resolve();
      const mine = ++loadSeq.current;
      if (reset) resetPending.current = true;
      inflight.current = true;
      return meetingWorkspace(meetingId)
        .then((next) => {
          // A newer load started since (a reload after a change): it carries the answer, and a reset it replaced is still owed.
          if (mine !== loadSeq.current || current.current !== meetingId) return;
          reset = resetPending.current;
          resetPending.current = false;
          // The page redraws only when the workspace is not what it already holds.
          const json = JSON.stringify(next);
          if (json !== lastView.current) {
            lastView.current = json;
            setView(next);
          }
          // A run that just finished replaced the summary: it shows unless the person has an unsaved edit. Notes stay as typed.
          const finished = lastStage.current != null && next?.processingStage == null;
          lastStage.current = next?.processingStage ?? null;
          if (next && reset) {
            loadedNotes.current = next.personalNotes;
            loadedSummary.current = next.summary ?? "";
            setNotes(next.personalNotes);
            setSummary(next.summary ?? "");
          } else if (next && finished) {
            const fresh = next.summary ?? "";
            setSummary((typed) => {
              if (typed !== loadedSummary.current) return typed;
              loadedSummary.current = fresh;
              return fresh;
            });
          }
          if (finished) setTranscriptRound((n) => n + 1);
          if (reset || finished) {
            loadSources();
            meetingSeedActionItems(meetingId).then(
              (list) => current.current === meetingId && setItemsState(list),
              () => {},
            );
          }
        })
        .catch((e: unknown) => {
          if (mine === loadSeq.current) resetPending.current = false;
          if (current.current === meetingId) setError(message(e));
        })
        .finally(() => {
          if (mine === loadSeq.current) inflight.current = false;
        });
    },
    [meetingId, loadSources],
  );

  useEffect(() => {
    // A new meeting starts from nothing: no state of the last one (items, trim, a busy flag, an error) carries over.
    loadSeq.current++;
    inflight.current = false;
    resetPending.current = false;
    lastStage.current = null;
    lastView.current = "";
    loadedNotes.current = null;
    loadedSummary.current = null;
    setView(null);
    setNotes(null);
    setSummary(null);
    setSources(null);
    setItemsState([]);
    setMarks([]);
    setTrim(null);
    setConflict(false);
    setViaWhimbrel(false);
    setResummarizeError(null);
    setNamesChanged(false);
    setBusy(false);
    setError(null);
    setEditedItems(new Set());
    void load(true);
    meetingMarkerList(meetingId).then((list) => current.current === meetingId && setMarks(sortedMarks(list ?? [])), () => {});
  }, [meetingId, load]);

  useEffect(() => {
    const timer = setInterval(() => void load(false), POLL_MS);
    return () => clearInterval(timer);
  }, [load]);

  const phase = view?.phase;
  const stage = view?.processingStage ?? null;
  useEffect(() => {
    let alive = true;
    meetingTrimInfo(meetingId).then(
      (info) => alive && setTrim(info),
      () => alive && setTrim(null),
    );
    return () => {
      alive = false;
    };
  }, [meetingId, phase, stage]);

  const saveNotes = useCallback((value: string) => meetingSaveNotes(meetingId, value), [meetingId]);
  useAutosave(notes, saveNotes, setError, loadedNotes);
  // Core keeps an override until it is reset, so "same as generated" means clear it rather than store a copy.
  const generated = view?.summaryIsEdited ? null : (view?.summary ?? null);
  const saveSummary = useCallback(
    (value: string) => {
      const override = overrideOf(value, generated);
      return override == null ? meetingResetOverrides(meetingId) : meetingSaveOverrides(meetingId, override, null, null);
    },
    [meetingId, generated],
  );
  useAutosave(summary, saveSummary, setError, loadedSummary);

  // Leaving the meeting (another one is picked, or the page closes) saves what was typed in the last moments instead of dropping it.
  const pending = useRef({ notes, summary, saveNotes, saveSummary });
  pending.current = { notes, summary, saveNotes, saveSummary };
  useEffect(
    () => () => {
      const p = pending.current;
      if (p.notes != null && p.notes !== loadedNotes.current) void p.saveNotes(p.notes).catch(() => {});
      if (p.summary != null && p.summary !== loadedSummary.current) void p.saveSummary(p.summary).catch(() => {});
    },
    [meetingId], // eslint-disable-line react-hooks/exhaustive-deps
  );

  /** Runs a change that replaces the page's content (regenerate, transcribe again, a language, forget), then reloads. */
  const act = (run: () => Promise<unknown>) => {
    setBusy(true);
    setError(null);
    run()
      .then(() => load(true))
      .then(() => {
        if (current.current !== meetingId) return;
        setTranscriptRound((n) => n + 1);
        onChanged();
      })
      .catch((e: unknown) => current.current === meetingId && setError(message(e)))
      .finally(() => current.current === meetingId && setBusy(false));
  };

  const regenerate = (keepEdits: boolean, whimbrel = viaWhimbrel) => {
    setConflict(false);
    setViaWhimbrel(false);
    if (!whimbrel) {
      act(() => meetingRegenerate(meetingId, keepEdits));
      return;
    }
    // A failed re-summary keeps the summary and says why under the notice.
    setBusy(true);
    setResummarizeError(null);
    meetingResummarizeWhimbrel(meetingId, keepEdits)
      .then(() => load(true))
      .then(() => current.current === meetingId && onChanged())
      .catch((e: unknown) => current.current === meetingId && setResummarizeError(message(e)))
      .finally(() => current.current === meetingId && setBusy(false));
  };

  const askThenRegenerate = (whimbrel = false) => {
    setBusy(true);
    meetingRegenerationConflict(meetingId)
      .then((needsPrompt) => {
        setViaWhimbrel(whimbrel);
        return needsPrompt ? setConflict(true) : regenerate(false, whimbrel);
      })
      .catch((e: unknown) => current.current === meetingId && setError(message(e)))
      .finally(() => current.current === meetingId && setBusy(false));
  };

  /** The action items from the editor; a changed text marks that item as the person's. */
  const setItems = (next: ActionItemView[]) => {
    const before = new Map(items_.current.map((i) => [i.id, i.text]));
    const changed = next.filter((i) => before.has(i.id) && before.get(i.id) !== i.text).map((i) => i.id);
    if (changed.length > 0) setEditedItems((all) => new Set([...all, ...changed]));
    setItemsState(next);
  };

  return {
    view,
    error,
    setError,
    notes,
    setNotes,
    summary,
    setSummary,
    items,
    setItems,
    sources,
    marks,
    trim,
    conflict,
    setConflict,
    resummarizeError,
    namesChanged,
    setNamesChanged,
    busy,
    editedItems,
    transcriptRound,
    /** The transcript changed (a trim): its view reads it again. */
    bumpTranscript: () => setTranscriptRound((n) => n + 1),
    act,
    regenerate,
    askThenRegenerate,
    /** The names in the transcript changed: the sources show the new names. */
    refreshSources: loadSources,
    forget: () => act(() => meetingForget(meetingId)),
    transcribeAgain: () => act(() => meetingTranscribeAgain(meetingId)),
    setLanguage: (code: string | null) => act(() => meetingSetSpokenLanguage(meetingId, code)),
    removeMark: (id: number) => void meetingMarkerDelete(id).then(() => setMarks((all) => all.filter((m) => m.id !== id)), (e: unknown) => setError(message(e))),
    renameMark: (id: number, label: string | null) =>
      void meetingMarkerSetLabel(id, label).then(
        (ok) => ok && setMarks((all) => all.map((m) => (m.id === id ? { ...m, label: label?.trim() ? label.trim() : null } : m))),
        (e: unknown) => setError(message(e)),
      ),
  };
}
