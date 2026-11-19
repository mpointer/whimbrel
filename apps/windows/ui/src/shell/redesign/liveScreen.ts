// Whether the live meeting screen is on screen, so the shell does not draw the recording bar a second time above it
// (docs/ui-redesign/screens/live-meeting.md: the bar sits at the top of the screen's column and is not drawn twice).

import { useSyncExternalStore } from "react";

let shown = 0;
let title: string | null = null;
const listeners = new Set<() => void>();
const subscribe = (listener: () => void) => {
  listeners.add(listener);
  return () => listeners.delete(listener);
};

/** The live screen mounted (true) or unmounted (false). Counted, so two panes cannot hide the bar by accident. */
export function setLiveScreenShown(on: boolean): void {
  shown = Math.max(0, shown + (on ? 1 : -1));
  if (shown === 0) title = null;
  listeners.forEach((l) => l());
}

/** The meeting's title, for the header row while the screen shows (the mockup titles the page with the meeting). */
export function setLiveScreenTitle(next: string | null): void {
  title = next;
  listeners.forEach((l) => l());
}

export function useLiveScreenShown(): boolean {
  return useSyncExternalStore(subscribe, () => shown > 0);
}

export function useLiveScreenTitle(): string | null {
  return useSyncExternalStore(subscribe, () => title);
}
