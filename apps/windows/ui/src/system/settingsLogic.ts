// The decisions inside the Settings parts, as plain functions so they can be tested without a browser. The parts
// call these and draw the answer.

/* ---- Section list keys ---- */

/** Where focus goes when a key is pressed on item `current` of a vertical list of `count`, or null to leave the key
 *  alone. Up and Down wrap; Home and End go to the ends. Left and Right are not used by a vertical list. */
export function navFocusIndex(key: string, current: number, count: number): number | null {
  if (count <= 0) return null;
  if (key === "ArrowDown") return (current + 1) % count;
  if (key === "ArrowUp") return (current - 1 + count) % count;
  if (key === "Home") return 0;
  if (key === "End") return count - 1;
  return null;
}

/* ---- Paths ---- */

/** A path with a place to break after each separator (a zero-width space after "\\" and "/"), so a long folder wraps at its
 *  folders instead of in the middle of a word. The text a person sees and copies is the same path. */
export function breakAtSeparators(path: string): string {
  return path.replace(/([\\/])(?!\u200b)/g, "$1\u200b");
}

/* ---- Number box ---- */

export type NumberProblem = "empty" | "invalid" | "range" | "fraction";
export type NumberParse = { ok: true; value: number } | { ok: false; problem: NumberProblem };

export interface NumberLimits {
  min?: number;
  max?: number;
  /** Whole numbers only. */
  integer?: boolean;
  /** An amount of money: a leading "$" is allowed (and dropped). */
  money?: boolean;
}

/** Reads what was typed. A point is the decimal mark. A comma is read as a decimal mark only when exactly one or two
 *  digits follow it at the end ("5,5"); a comma or point before three digits ("1,000", "1,000.50", "1.000,50") could
 *  mean a thousand or a fraction, so it is a problem and never a guess. Money may start with "$". Anything that is not
 *  a plain number, or is out of range, is a problem, never a number: the caller must not store it. */
export function parseNumberDraft(text: string, limits: NumberLimits = {}): NumberParse {
  let s = text.trim();
  if (limits.money) s = s.replace(/^\$\s*/, "");
  if (s === "") return { ok: false, problem: "empty" };
  let normal: string;
  // "1.000" is a European thousand as often as a fraction: refused like "1,000", never guessed. Money also stops at
  // two decimals, so "1.0000" is not an amount of money (a plain number may have more).
  if (/^-?\d+\.\d{3}$/.test(s) || (limits.money && /^-?\d+\.\d{3,}$/.test(s))) return { ok: false, problem: "invalid" };
  if (/^-?\d+(?:\.\d+)?$/.test(s)) {
    normal = s;
  } else if (/^-?\d+,\d{1,2}$/.test(s)) normal = s.replace(",", ".");
  else return { ok: false, problem: "invalid" };
  const value = Number(normal);
  if (!Number.isFinite(value)) return { ok: false, problem: "invalid" };
  if (limits.integer && !Number.isInteger(value)) return { ok: false, problem: "fraction" };
  if ((limits.min !== undefined && value < limits.min) || (limits.max !== undefined && value > limits.max)) return { ok: false, problem: "range" };
  return { ok: true, value };
}

export interface NumberCommit {
  /** The number to store, or null when nothing must be written (unchanged, empty, or not valid). */
  emit: number | null;
  problem: NumberProblem | null;
}

/** What blur or Return does with the typed text. Writes only a valid number that differs from the stored one. `stored`
 *  null means nothing is stored (the box only shows a remembered number): any valid number is then a change. */
export function commitNumberDraft(text: string, stored: number | null, limits: NumberLimits = {}): NumberCommit {
  const parsed = parseNumberDraft(text, limits);
  if (!parsed.ok) return { emit: null, problem: parsed.problem };
  return { emit: parsed.value === stored ? null : parsed.value, problem: null };
}

/** The text a number box shows for a stored number. */
export function numberText(value: number): string {
  return String(value);
}

/* ---- Secret field ---- */

export type SecretView = "empty" | "saved" | "editing";

/** Which of the three faces the secret row shows. `saved` is whether the app holds a value; `editing` is whether the
 *  person asked to replace it. */
export function secretView(saved: boolean, editing: boolean): SecretView {
  if (!saved) return "empty";
  return editing ? "editing" : "saved";
}

/** What Save sends: the typed value without stray spaces or a pasted line break, or null when there is nothing. */
export function secretToSave(draft: string): string | null {
  const v = draft.trim();
  return v === "" ? null : v;
}

/* ---- Adding to a list ---- */

export type AddDraft = { ok: true; value: string } | { ok: false; problem: "empty" | "duplicate" };

/** Checks a new list entry: not empty, and not already there (compared without regard to case or edge spaces). */
export function checkAddDraft(text: string, existing: readonly string[] = []): AddDraft {
  const value = text.trim();
  if (value === "") return { ok: false, problem: "empty" };
  const key = value.toLocaleLowerCase();
  if (existing.some((e) => e.trim().toLocaleLowerCase() === key)) return { ok: false, problem: "duplicate" };
  return { ok: true, value };
}

/* ---- Dialog ---- */

/** Everything in a dialog a person can Tab to: buttons, fields, menus, links and anything given a tab stop. */
export const DIALOG_FOCUSABLE =
  'button:not(:disabled), input:not(:disabled):not([type="hidden"]), select:not(:disabled), textarea:not(:disabled), a[href], [tabindex]:not([tabindex="-1"]):not(:disabled)';

/** Where Tab goes inside a dialog, or null to let the browser move focus on its own. `box` is the dialog itself. */
export function trapTabTarget<E extends object>(items: readonly E[], active: unknown, box: unknown, shift: boolean): E | null {
  if (items.length === 0) return null;
  const first = items[0];
  const last = items[items.length - 1];
  // The dialog itself has focus: Tab goes to its first control, Shift+Tab to its last.
  if (active === box) return shift ? last : first;
  // Focus is outside every control (or on something not in the list): bring it back in.
  if (!items.includes(active as E)) return shift ? last : first;
  if (shift && active === first) return last;
  if (!shift && active === last) return first;
  return null;
}

/** What Return does in a dialog. In a text box it does nothing at all (it must never press a button the person
 *  cannot see is about to be pressed); on a button it is that button's own press. */
export function dialogEnterAction(target: { tag: string; type?: string; role?: string }): "swallow" | "native" {
  const tag = target.tag.toLowerCase();
  if (tag === "textarea") return "swallow";
  if (tag === "input") {
    const type = (target.type ?? "text").toLowerCase();
    return type === "checkbox" || type === "radio" || type === "button" || type === "submit" ? "native" : "swallow";
  }
  return "native";
}

/** Whether what was typed is the word that confirms. Exactly the word: no trimming, so " DELETE " does not count. */
export function typedConfirmMatches(typed: string, word: string): boolean {
  return word !== "" && typed === word;
}
