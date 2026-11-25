// The redesign switch is on for everyone (docs/ui-redesign/screens/switch-removal.md, A1). Nothing asks the shell any
// more: every answer here is "on" at once, so no old screen can draw on a fresh install, before the shell is ready, or
// after a failed ask. The exports and their call sites stay; the old branches behind `useRedesign()` go in the clean-up
// (step 11, part B), together with this file. The shell's `redesign_enabled` command still exists and is not called.

/** Always true. The argument is kept so a call site that passes a loader still type-checks; it is not called. */
export async function redesignEnabled(_load?: () => Promise<unknown>): Promise<boolean> {
  return true;
}

/** Always true, with no ask: main.tsx still waits on it with the language, which now costs nothing. */
export async function primeRedesign(_load?: () => Promise<unknown>): Promise<boolean> {
  return true;
}

/** Always true. The LaunchGate still calls it once the launch opened; there is nothing left to re-ask. */
export async function recheckRedesign(_load?: () => Promise<unknown>): Promise<boolean> {
  return true;
}

/** Always true: it is known before anything asks. */
export function redesignKnown(): boolean {
  return true;
}

/** The switch for a component: true on the first render, with no wait. */
export function useRedesign(): boolean {
  return true;
}
