// What a key press means while the hotkey row is recording a new shortcut. The same rules as the old card: Escape
// cancels, Tab ends it and moves focus on, a modifier alone keeps waiting, a refused combination keeps recording and says why, anything else applies.

import { recordKey, type KeyPress } from "../../../../hotkeyModel";

export type RecorderStep = { kind: "cancel" } | { kind: "leave" } | { kind: "waiting" } | { kind: "refused"; reason: string } | { kind: "apply"; accelerator: string };

export function recorderStep(e: KeyPress & { key: string; repeat?: boolean }): RecorderStep {
  if (e.key === "Escape") return { kind: "cancel" };
  // Tab moves focus on, as in any other part of the page: recording ends and the key is left alone.
  if (e.key === "Tab" && !e.ctrlKey && !e.altKey && !e.metaKey) return { kind: "leave" };
  // The key that started the recording (Return on the button) may repeat; it is not a shortcut.
  if (e.repeat) return { kind: "waiting" };
  const recorded = recordKey(e);
  if (recorded.kind === "shortcut") return { kind: "apply", accelerator: recorded.accelerator };
  return recorded;
}
