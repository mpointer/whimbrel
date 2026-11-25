// The finished note's pure model (redesign flag on only): clock labels, which summary lines carry a numbered source, how
// the action items pick up their owner, due date and source, and which transcript line a time or an id points at.
// Spec: docs/ui-redesign/screens/note.md. Nothing here reads the shell.

import type { ActionItemView, NoteSourcesView, NoteSourceView, SourcedLineView, TranscriptLineView } from "../../generated/types";
import { t } from "../../l10n";
import { speakerLabel } from "../sentinels";

/** "18:25" from centiseconds into the meeting, or "1:02:03" past an hour. */
export function offsetClock(cs: number): string {
  const total = Math.max(0, Math.floor(cs / 100));
  const h = Math.floor(total / 3600);
  const m = Math.floor((total % 3600) / 60);
  const s = total % 60;
  const two = (n: number) => String(n).padStart(2, "0");
  return h > 0 ? `${h}:${two(m)}:${two(s)}` : `${m}:${two(s)}`;
}

/** The centiseconds of "4:05", "125:30" or "1:02:03" (a live alert's "asked at"); null for anything else. */
export function clockToCs(text: string): number | null {
  const parts = text.trim().split(":");
  // The leading part may run past 99 ("125:30" is minutes in a long meeting); the others are two digits at most.
  if (parts.length < 2 || parts.length > 3 || parts.some((p, i) => !(i === 0 ? /^\d{1,4}$/ : /^\d{1,2}$/).test(p))) return null;
  const nums = parts.map(Number);
  const [h, m, s] = parts.length === 3 ? nums : [0, nums[0], nums[1]];
  if (s > 59 || (parts.length === 3 && m > 59)) return null;
  return (h * 3600 + m * 60 + s) * 100;
}

/** How a source mark is named for a screen reader: "Source 3, Jo, 18:25". */
export function sourceName(number: number, speaker: string, startCs: number): string {
  return t("Source {number}, {speaker}, {time}", { number, speaker, time: offsetClock(startCs) });
}

/** A transcript line's speaker as shown: a name, a guess as "Name?", or the stored label translated. */
export function lineSpeaker(line: Pick<TranscriptLineView, "label" | "speaker" | "guess">): string {
  if (line.label === line.speaker) return speakerLabel(line.speaker);
  return line.guess ? `${line.label}?` : line.label;
}

/** A source's speaker as shown: the shell sends a name or the stored label. */
export function sourceSpeaker(source: Pick<NoteSourceView, "speaker">): string {
  return speakerLabel(source.speaker);
}

/** One decision or open question as the note draws it. */
export interface NoteLine {
  key: string;
  text: string;
  /** Whimbrel's own words: drawn in the AI ink. A line the person edited is theirs. */
  ai: boolean;
  /** The source mark's number, or null when the line has no source. */
  number: number | null;
}

export interface ActionRow {
  item: ActionItemView;
  ai: boolean;
  owner: string | null;
  due: string | null;
  number: number | null;
}

export interface NoteSourceEntry {
  number: number;
  source: NoteSourceView;
}

export interface NoteModel {
  decisions: NoteLine[];
  questions: NoteLine[];
  actions: ActionRow[];
  /** Every numbered source, in number order. */
  entries: NoteSourceEntry[];
}

export interface NoteInput {
  decisions: string[];
  decisionsAreEdited: boolean;
  openQuestions: string[];
  openQuestionsAreEdited: boolean;
  /** The action items in display order. */
  items: ActionItemView[];
  sources: NoteSourcesView | null;
  /** Action items whose text the person changed on this screen: theirs from then on. */
  editedItems?: ReadonlySet<number>;
}

/** Each text paired with the first unused sourced line that has the same words (a list can repeat itself). */
function pair(texts: string[], sourced: SourcedLineView[]): (NoteSourceView | null)[] {
  const used = new Set<number>();
  return texts.map((text) => {
    const at = sourced.findIndex((s, i) => !used.has(i) && s.text === text);
    if (at < 0) return null;
    used.add(at);
    return sourced[at].source ?? null;
  });
}

/** The note's lines with their sources numbered 1, 2, 3 in reading order: decisions, open questions, action items. */
export function buildNote(input: NoteInput): NoteModel {
  const entries: NoteSourceEntry[] = [];
  const number = (source: NoteSourceView | null): number | null => {
    if (!source) return null;
    const n = entries.length + 1;
    entries.push({ number: n, source });
    return n;
  };
  const lines = (kind: string, texts: string[], edited: boolean, sourced: SourcedLineView[] | undefined): NoteLine[] => {
    const found = edited || !sourced ? texts.map(() => null) : pair(texts, sourced);
    return texts.map((text, i) => ({ key: `${kind}-${i}`, text, ai: !edited, number: number(found[i]) }));
  };
  const decisions = lines("decision", input.decisions, input.decisionsAreEdited, input.sources?.decisions);
  const questions = lines("question", input.openQuestions, input.openQuestionsAreEdited, input.sources?.openQuestions);
  const sourcedItems = input.sources?.actionItems ?? [];
  const taken = new Set<number>();
  const actions = input.items.map((item): ActionRow => {
    // A stored line belongs to an item only while the words are identical (an id alone is not enough: the text may have changed).
    const at = sourcedItems.findIndex((s, i) => !taken.has(i) && s.text === item.text && (s.itemId == null || s.itemId === item.id));
    // Edited, durably: core stored lines for this meeting's action items, and none of them has these words any more. With no
    // stored lines at all (an old meeting) there is nothing to compare, so a generated item stays Whimbrel's plain line.
    const changed = at < 0 && sourcedItems.length > 0;
    const mine = input.editedItems?.has(item.id) === true || (item.source === "generated" && changed);
    const found = !mine && at >= 0 ? sourcedItems[at] : null;
    if (found) taken.add(at);
    return {
      item,
      ai: item.source === "generated" && !mine,
      owner: found?.owner ?? null,
      due: found?.due ?? null,
      number: number(found?.source ?? null),
    };
  });
  return { decisions, questions, actions, entries };
}

/** The line whose start is closest to a time, the earlier one on a tie; null for no lines. */
export function nearestLineId(lines: readonly Pick<TranscriptLineView, "id" | "startCs">[], cs: number): number | null {
  let best: number | null = null;
  let gap = Infinity;
  for (const line of lines) {
    const d = Math.abs(line.startCs - cs);
    if (d < gap) {
      gap = d;
      best = line.id;
    }
  }
  return best;
}

/** What opens the transcript: a line by id (a source, a lane span, an Ask citation) or a moment by time (a mark, an alert). */
export type LineTarget = { kind: "line"; id: number } | { kind: "time"; cs: number };

/** The id of the line a target means among the lines loaded: the line itself, or the nearest one. Null when it points at nothing. */
export function resolveTarget(lines: readonly Pick<TranscriptLineView, "id" | "startCs">[], target: LineTarget | null): number | null {
  if (!target || lines.length === 0) return null;
  if (target.kind === "line") return lines.some((l) => l.id === target.id) ? target.id : null;
  return nearestLineId(lines, target.cs);
}

/** Which of the two views the page shows. */
export type NoteMode = "note" | "transcript";

/** What the page says about its summary, from the workspace: one state at a time. */
export type NoteState = "processing" | "failed" | "noSummary" | "ready";

export function noteState(v: { processingStage?: string | null; summaryFailure?: string | null; summary?: string | null; hasTranscript: boolean; decisions: string[]; actionItems: unknown[] }): NoteState {
  if (v.processingStage != null) return "processing";
  const written = (v.summary ?? "").trim() !== "" || v.decisions.length > 0 || v.actionItems.length > 0;
  if (v.summaryFailure != null && !written) return "failed";
  if (!written) return "noSummary";
  return "ready";
}

/** Whether Whimbrel wrote anything on the page: the summary (unless the person edited it), or a decision, question or action item in its ink. */
export function hasWhimbrelLines(model: NoteModel, summaryText: string, summaryIsEdited: boolean): boolean {
  if (summaryText.trim() !== "" && !summaryIsEdited) return true;
  return model.decisions.some((l) => l.ai) || model.questions.some((l) => l.ai) || model.actions.some((a) => a.ai);
}

/** The margin's second line: only for a note that has lines to source and none of them has one, on a meeting this PC can summarize. */
export function offerSources(model: NoteModel, canRegenerate: boolean, hasSummaryLines: boolean): boolean {
  return canRegenerate && hasSummaryLines && model.entries.length === 0;
}

/** Whether the margin column has anything to hold: a source card, or the margin's lines ("Whimbrel wrote the marked lines..." and,
 *  where it applies, "Regenerate the summary to add sources."), which are drawn whenever Whimbrel wrote a line. A note with neither
 *  (a meeting with no transcript and no summary) folds the column away and the note takes its width. */
export function marginShown(model: NoteModel, aiWrote: boolean): boolean {
  return model.entries.length > 0 || aiWrote;
}


/** Whether the margin column shows while the person edits. Typing can change what `marginShown` says (an edited line is the person's,
 *  not Whimbrel's), and a column that folds or unfolds under the cursor moves the text being typed. So the answer is taken when Edit
 *  begins (`held` is null until then) and kept until Edit ends. Returns the answer to draw and the value to hold for the next draw. */
export function holdMargin(held: boolean | null, editing: boolean, live: boolean): { shown: boolean; held: boolean | null } {
  if (!editing) return { shown: live, held: null };
  const kept = held ?? live;
  return { shown: kept, held: kept };
}
