// The hidden redesign switch (`redesign.enabled` in settings.json, off unless set to true by hand; no UI turns it on).
// The shell reads it (src-tauri `SettingsFile::redesign_enabled`) and answers the `redesign_enabled` command; it also
// keeps the Thread skin out of `skins_list` while the switch is off. New screens check `useRedesign()` and draw
// exactly what they drew before while it is false, so a release cut part-way looks the same as today.

import { useEffect, useState } from "react";
import { redesignEnabledCommand } from "./api";

/** True only for a real `true` from the shell; a missing command, an error or anything else is off. */
export async function redesignEnabled(load: () => Promise<unknown> = redesignEnabledCommand): Promise<boolean> {
  try {
    return (await load()) === true;
  } catch {
    return false;
  }
}

/** What the shell answered before the first render (`primeRedesign`), or null while nobody has asked. */
let known: boolean | null = null;

/** Asks the shell once and keeps the answer, so the first render already knows. main.tsx awaits this before it
 *  renders, the way it awaits the language: nothing paints in the new look and then switches, or the other way round. */
export async function primeRedesign(load: () => Promise<unknown> = redesignEnabledCommand): Promise<boolean> {
  known = await redesignEnabled(load);
  return known;
}

/** The answer `primeRedesign` kept, for code outside a component; null before it ran. */
export function redesignKnown(): boolean | null {
  return known;
}

/** The switch for a component: the primed answer when there is one, else false until the shell has answered, and
 *  false if it never does. */
export function useRedesign(): boolean {
  const [on, setOn] = useState(known ?? false);
  useEffect(() => {
    if (known !== null) return;
    let live = true;
    void redesignEnabled().then((value) => {
      known = value;
      if (live) setOn(value);
    });
    return () => {
      live = false;
    };
  }, []);
  return on;
}
