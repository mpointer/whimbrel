import type { ActionItemView, WorkspaceView } from "../generated/types";

/**
 * Whether a regeneration would overwrite work the user did, so the workspace asks first.
 * The shell answers this from the stored draft; this is the same question for edits the
 * user has typed but that haven't autosaved yet (the Mac's `mergeUnsavedEditsIntoDraft`
 * closes that 1.5 s window).
 */
export function unsavedEditsWouldBeLost(view: WorkspaceView, local: LocalEdits): boolean {
  return (
    (local.summary ?? "") !== (view.summary ?? "") ||
    !sameList(local.decisions ?? view.decisions, view.decisions) ||
    !sameList(local.openQuestions ?? view.openQuestions, view.openQuestions)
  );
}

export interface LocalEdits {
  summary?: string;
  decisions?: string[];
  openQuestions?: string[];
}

function sameList(a: string[], b: string[]): boolean {
  return a.length === b.length && a.every((value, i) => value === b[i]);
}

/**
 * What to send as an override: `null` when the text matches what was generated, so the
 * field goes back to following the summary instead of freezing today's wording.
 */
export function overrideOf(edited: string, generated: string | null): string | null {
  return edited.trim() === (generated ?? "").trim() ? null : edited;
}

/** The list after moving one item, for the reorder buttons. Pure. */
export function movedItem(items: ActionItemView[], id: number, direction: -1 | 1): ActionItemView[] {
  const at = items.findIndex((i) => i.id === id);
  const to = at + direction;
  if (at < 0 || to < 0 || to >= items.length) return items;
  const next = [...items];
  [next[at], next[to]] = [next[to], next[at]];
  return next;
}

/** Open items first, each keeping its order; completed ones sink (the Mac's list). */
export function displayOrder(items: ActionItemView[]): ActionItemView[] {
  return [...items.filter((i) => !i.completed), ...items.filter((i) => i.completed)];
}

/** The workspace's heading for each phase. */
export function phaseTitle(view: WorkspaceView): string {
  switch (view.phase) {
    case "before":
      return "Before the meeting";
    case "during":
      return "Recording";
    case "cancelled":
      return "Cancelled";
    default:
      return "After the meeting";
  }
}
