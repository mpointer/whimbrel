import type { ActionItemView, SnapshotsView, SnapshotView, WorkspaceView } from "../generated/types";
import { t } from "../l10n";

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

/** The Mac's "Transcript language" choices, each in its own name as on the Mac. */
export const TRANSCRIPT_LANGUAGES: { code: string; label: string }[] = [
  { code: "en", label: "English" },
  { code: "es", label: "Español" },
  { code: "fr", label: "Français" },
];

/**
 * The "Transcript language" options: Automatic (`""`, detect again), then the Mac's three.
 * A detected language outside them is listed too, in its own name, so the picker can show it.
 */
export function transcriptLanguageOptions(current: string | null, automatic: string): { code: string; label: string }[] {
  const options = [{ code: "", label: automatic }, ...TRANSCRIPT_LANGUAGES];
  if (current && !options.some((o) => o.code === current)) {
    let label = current;
    try {
      label = new Intl.DisplayNames([current], { type: "language" }).of(current) ?? current;
    } catch {
      // An unknown code shows as itself.
    }
    options.push({ code: current, label });
  }
  return options;
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
      return t("Before the meeting");
    case "during":
      return t("Recording");
    case "cancelled":
      return t("Cancelled");
    default:
      return t("After the meeting");
  }
}

/**
 * A slide thumbnail's cache key. The file name and modification time are in it because a
 * build that replaces a slide writes a new file, sometimes under an earlier name.
 */
export function snapshotKey(snapshot: SnapshotView): string {
  return `${snapshot.id}:${snapshot.fileName}:${snapshot.modifiedUnixMs}`;
}

/** The slides whose thumbnail isn't cached or already on its way. */
export function thumbnailsToFetch(snapshots: SnapshotView[], known: ReadonlySet<string>): SnapshotView[] {
  return snapshots.filter((s) => !known.has(snapshotKey(s)));
}

/** The cache without thumbnails of slides that are gone or were replaced. */
export function prunedThumbnails<T>(cache: ReadonlyMap<string, T>, snapshots: SnapshotView[]): Map<string, T> {
  const live = new Set(snapshots.map(snapshotKey));
  return new Map([...cache].filter(([key]) => live.has(key)));
}

/** The strip after deleting a slide, before the next poll confirms it. */
export function withoutSnapshot(view: SnapshotsView, id: number): SnapshotsView {
  return { ...view, snapshots: view.snapshots.filter((s) => s.id !== id) };
}

/** The strip's heading, e.g. "Slides (3)". */
export function slidesLabel(view: SnapshotsView): string {
  return t("Slides ({n})", { n: view.snapshots.length });
}
