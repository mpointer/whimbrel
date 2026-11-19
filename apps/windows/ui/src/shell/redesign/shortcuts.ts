// The shell's keyboard shortcuts on Windows, redesign flag on only: Ctrl+1 to Ctrl+6 for the six sections, in the
// order of the sidebar, and Ctrl+K for the search field (docs/ui-redesign/screens/home.md, "The shell").

import type { TabId } from "../../tabs";

export type ShortcutAction = { kind: "section"; id: TabId } | { kind: "search" };

export interface KeyLike {
  key: string;
  ctrlKey: boolean;
  metaKey?: boolean;
  altKey?: boolean;
  shiftKey?: boolean;
}

/** What a key press asks for. Only plain Ctrl (not Ctrl+Alt, Ctrl+Shift or Meta) with 1 to 6 or K. */
export function shortcutOf(e: KeyLike, order: readonly TabId[]): ShortcutAction | null {
  if (!e.ctrlKey || e.metaKey || e.altKey || e.shiftKey) return null;
  if (e.key === "k" || e.key === "K") return { kind: "search" };
  if (/^[1-9]$/.test(e.key)) {
    const id = order[Number(e.key) - 1];
    return id ? { kind: "section", id } : null;
  }
  return null;
}

export type LiveShortcut = "pause" | "mark";

/** The live meeting's two keys, redesign flag on only: Ctrl+Shift+P pauses or resumes the recording, Ctrl+Shift+M marks a
 *  moment. (Ctrl+M alone would clash with Minimize on the Mac, so both OSes use Shift.) */
export function liveShortcutOf(e: KeyLike): LiveShortcut | null {
  if (!e.ctrlKey || !e.shiftKey || e.metaKey || e.altKey) return null;
  const key = e.key.toLowerCase();
  return key === "p" ? "pause" : key === "m" ? "mark" : null;
}

/** "Ctrl+1" for the sidebar's hint on the section at `index`. */
export const shortcutHint = (index: number): string => `Ctrl+${index + 1}`;
