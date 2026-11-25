// Whether Ask can answer now and which model it would use, as the old composer works it out. One read is shared by the header and the Find
// page, kept across tabs so a page that mounts again already knows, and read again when the window is focused (a key added in Settings).
// `null` means nothing has been read yet: callers must not take that for "not set up".

import { useEffect, useSyncExternalStore } from "react";
import { aiSettings, whimbrelAiStatus } from "../../api";
import { askAvailable, askModelLabel } from "../../today/todayModel";
import { routePreviews } from "./routePreview";

export interface AskStateValue {
  available: boolean;
  model: string;
}

let current: AskStateValue | null = null;
let reading: Promise<void> | null = null;
const listeners = new Set<() => void>();

function publish(next: AskStateValue): void {
  if (current && current.available === next.available && current.model === next.model) return;
  current = next;
  listeners.forEach((l) => l());
}

/** Reads the settings again (one read at a time). The route previews kept so far were made under the old settings, so they go. A failed
 *  read keeps what was known; with nothing known it says Ask is not available, so nothing waits on it forever. */
export function refreshAskState(): Promise<void> {
  routePreviews.clear();
  if (reading) return reading;
  reading = Promise.all([aiSettings(), whimbrelAiStatus().catch(() => null)])
    .then(([s, w]) => publish({ available: askAvailable(s.routing, s.keys, !!w?.signedIn), model: askModelLabel(s.routing) }))
    .catch(() => {
      if (!current) publish({ available: false, model: "" });
    })
    .finally(() => {
      reading = null;
    });
  return reading;
}

/** One focus listener for however many components use the state (the header and the Find page both do): it goes on with the first and comes
 *  off with the last, so one of them unmounting never switches off the other's refresh. */
export function createFocusRefresh(target: { addEventListener(type: "focus", l: () => void): void; removeEventListener(type: "focus", l: () => void): void } | null, refresh: () => void) {
  let count = 0;
  return {
    retain(): () => void {
      if (count === 0) target?.addEventListener("focus", refresh);
      count += 1;
      let released = false;
      return () => {
        if (released) return;
        released = true;
        count -= 1;
        if (count === 0) target?.removeEventListener("focus", refresh);
      };
    },
    count: () => count,
  };
}
const focusRefresh = createFocusRefresh(typeof window !== "undefined" ? window : null, () => void refreshAskState());

export function useAskState(): AskStateValue | null {
  const value = useSyncExternalStore(
    (l) => (listeners.add(l), () => listeners.delete(l)),
    () => current,
  );
  useEffect(() => {
    void refreshAskState();
    return focusRefresh.retain();
  }, []);
  return value;
}

/** Tests only. */
export function resetAskState(): void {
  current = null;
  reading = null;
  listeners.forEach((l) => l());
}
