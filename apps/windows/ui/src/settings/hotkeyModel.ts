// The hotkey panel's shortcut in Settings → Capture: the status line, and turning a key press into
// the accelerator `set_hotkey_shortcut` takes ("Control+Alt+Space", tauri-plugin-global-shortcut's form).

import type { HotkeyStatus } from "../generated/types";
import { t } from "../l10n";

/** The shell's `hotkey_panel::DEFAULT_SHORTCUT`.
 *  Ctrl+Alt is what AltGr sends on many European layouts, so on those AltGr+Space can open the
 *  panel too (and the shortcut can take AltGr+Space from an app that types with it). */
export const DEFAULT_SHORTCUT = "Control+Alt+Space";

const MODIFIERS: Record<string, string> = { control: "Ctrl", ctrl: "Ctrl", alt: "Alt", option: "Alt", shift: "Shift", super: "Win", command: "Win", cmd: "Win" };

/** "Control+Alt+Space" → "Ctrl+Alt+Space"; "Control+Shift+KeyK" → "Ctrl+Shift+K". */
export function displayShortcut(accelerator: string): string {
  return accelerator
    .split("+")
    .map((part) => {
      const p = part.trim();
      const modifier = MODIFIERS[p.toLowerCase()];
      if (modifier) return modifier;
      if (/^Key[A-Z]$/.test(p)) return p.slice(3);
      if (/^Digit[0-9]$/.test(p)) return p.slice(5);
      if (p.startsWith("Arrow")) return p.slice(5);
      return p;
    })
    .join("+");
}

/** The Settings line: registered, or why not and where else to open the panel. */
export function hotkeyStatusLine(status: HotkeyStatus): string {
  const shortcut = displayShortcut(status.shortcut);
  if (status.registered) return t("Registered: {shortcut}", { shortcut });
  const why = status.error ? ` (${status.error})` : "";
  return (
    t("{shortcut} couldn't be registered: another app may already use it.", { shortcut }) +
    why +
    " " +
    t("Choose another shortcut, or use Ask about this window… in the tray menu.")
  );
}

/** What `set_hotkey_shortcut` answered: null when the new shortcut is now registered, else why not. */
export function changeResultLine(attempt: HotkeyStatus, current: HotkeyStatus): string | null {
  if (attempt.registered) return null;
  const why = attempt.error ? ` (${attempt.error})` : "";
  return (
    t("{shortcut} couldn't be registered: another app may already use it.", { shortcut: displayShortcut(attempt.shortcut) }) +
    why +
    (current.registered ? " " + t("Still using {shortcut}.", { shortcut: displayShortcut(current.shortcut) }) : "")
  );
}

/** The parts of a keydown the recorder reads. */
export interface KeyPress {
  code: string;
  ctrlKey: boolean;
  altKey: boolean;
  shiftKey: boolean;
  metaKey: boolean;
}

export type Recorded = { kind: "shortcut"; accelerator: string } | { kind: "waiting" } | { kind: "refused"; reason: string };

const MODIFIER_CODES = /^(Control|Alt|Shift|Meta|OS)(Left|Right)?$/;

/** A key press while recording: a shortcut, nothing yet (a modifier alone), or why it can't be one.
 *  Like the Mac: Ctrl or Alt, or two modifiers, so a global shortcut never takes plain typing,
 *  a capital letter, or a Windows-key shortcut. */
export function recordKey(e: KeyPress): Recorded {
  if (MODIFIER_CODES.test(e.code) || e.code === "" || e.code === "Unidentified") return { kind: "waiting" };
  const mods = [e.ctrlKey && "Control", e.altKey && "Alt", e.shiftKey && "Shift", e.metaKey && "Super"].filter(Boolean) as string[];
  const accelerator = [...mods, e.code].join("+");
  if (mods.length === 0) {
    return { kind: "refused", reason: t("{shortcut} has no modifier key, so it would block normal typing. Include Ctrl or Alt.", { shortcut: displayShortcut(accelerator) }) };
  }
  if (!e.ctrlKey && !e.altKey && mods.length < 2) {
    return {
      kind: "refused",
      reason: t("{shortcut} would take a shortcut apps or Windows use themselves. Include Ctrl or Alt, or use two modifier keys.", {
        shortcut: displayShortcut(accelerator),
      }),
    };
  }
  return { kind: "shortcut", accelerator };
}
