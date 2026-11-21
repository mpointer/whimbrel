// The title a screen puts in the shell's header row (the finished note names itself there, as the mockup shows). Unlike the live
// screen's `liveScreen` it does not hide the shell's recording bar: a meeting can record while an old one is open.

import { useSyncExternalStore } from "react";

let title: string | null = null;
const listeners = new Set<() => void>();
const subscribe = (listener: () => void) => {
  listeners.add(listener);
  return () => listeners.delete(listener);
};

/** The title to show, or null to go back to the section's own name. */
export function setScreenTitle(next: string | null): void {
  if (next === title) return;
  title = next;
  listeners.forEach((l) => l());
}

export function useScreenTitle(): string | null {
  return useSyncExternalStore(subscribe, () => title);
}
