// The Thread list in the sidebar (Home only): Home says which parts of its line have content, the sidebar lists them,
// and choosing one scrolls Home to it. A tiny store so the two do not need to share a parent's state.

import { useSyncExternalStore } from "react";

export type ThreadPartId = "today" | "yesterday" | "lastWeek";

export interface ThreadPart {
  id: ThreadPartId;
  label: string;
  /** "9:52 AM", "Nov 16": small text after the label. */
  note?: string;
}

let parts: readonly ThreadPart[] = [];
const listeners = new Set<() => void>();
const emit = () => listeners.forEach((l) => l());

/** Home publishes its parts; an empty list (or leaving Home) hides the sidebar list. */
export function setThreadParts(next: readonly ThreadPart[]): void {
  const same = next.length === parts.length && next.every((p, i) => p.id === parts[i].id && p.label === parts[i].label && p.note === parts[i].note);
  if (same) return;
  parts = next;
  emit();
}

export function useThreadParts(): readonly ThreadPart[] {
  return useSyncExternalStore(
    (l) => {
      listeners.add(l);
      return () => listeners.delete(l);
    },
    () => parts,
  );
}

export const threadAnchorId = (id: ThreadPartId): string => `th-part-${id}`;

/** Scrolls Home to a part and moves focus there, so keyboard and screen reader users arrive too. */
export function scrollToThreadPart(id: ThreadPartId): void {
  const el = document.getElementById(threadAnchorId(id));
  if (!el) return;
  const calm = window.matchMedia?.("(prefers-reduced-motion: reduce)").matches;
  el.scrollIntoView({ block: "start", behavior: calm ? "auto" : "smooth" });
  el.focus({ preventScroll: true });
}
