// The safe retention rules (spec decision 4) as one pure function, so the rules can be tested without a screen.
//
// - Ticking "Keep forever" saves null at once: it only stops deleting.
// - Unticking saves nothing. The row shows the last stored number until the person commits a period.
// - Committing a period shorter than the stored one, or any period when the stored one was forever, asks first.
// - A longer period saves at once (it deletes nothing new). The same number writes nothing.
// - An empty, invalid or out of range number saves nothing; the number row says why.

import { parseNumberDraft, type NumberProblem } from "../../../../../system/settingsLogic";

export const RETENTION_MIN_DAYS = 1;
export const RETENTION_MAX_DAYS = 3650;
/** Shown as the starting number when a window was never stored with a number. */
export const RETENTION_DEFAULT_DAYS = 30;

export type RetentionIntent =
  | { kind: "tickForever" }
  | { kind: "untick" }
  /** A number the number row already checked. It is checked again here. */
  | { kind: "commit"; value: number }
  /** What was typed, before anything has read it. */
  | { kind: "commitText"; text: string };

export type RetentionDecision =
  | { kind: "save"; days: number | null }
  | { kind: "confirm"; days: number; why: "shorter" | "fromForever"; from: number | null }
  | { kind: "reject"; problem: NumberProblem }
  /** Nothing is written. `show` is the number the row shows after an untick. */
  | { kind: "none"; show?: number };

/** The number a window row shows: the stored one, else the last one it had, else the default. */
export function shownDays(stored: number | null | undefined, remembered: number | null | undefined): number {
  return stored ?? remembered ?? RETENTION_DEFAULT_DAYS;
}

export function retentionDecision(stored: number | null | undefined, lastNumber: number, intent: RetentionIntent): RetentionDecision {
  const was = stored ?? null;
  if (intent.kind === "tickForever") return was === null ? { kind: "none" } : { kind: "save", days: null };
  if (intent.kind === "untick") return { kind: "none", show: was ?? lastNumber };
  const parsed = parseNumberDraft(intent.kind === "commit" ? String(intent.value) : intent.text, {
    min: RETENTION_MIN_DAYS,
    max: RETENTION_MAX_DAYS,
    integer: true,
  });
  if (!parsed.ok) return { kind: "reject", problem: parsed.problem };
  const days = parsed.value;
  if (was === null) return { kind: "confirm", days, why: "fromForever", from: null };
  if (days === was) return { kind: "none" };
  if (days < was) return { kind: "confirm", days, why: "shorter", from: was };
  return { kind: "save", days };
}
