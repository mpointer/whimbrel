// What the in-meeting window does with a question it handed to the main window: it keeps the text until the main window says
// it took it, and past a few seconds it stops waiting, keeps the text and says where the answer will be. Pure.

/** How long the window waits for the main window to take the question. */
export const ASK_WAIT_MS = 5_000;

export type AskOutcome = "waiting" | "delivered" | "kept";

/** Delivered once acknowledged (even late is still delivered), kept after the wait, waiting before that. */
export function askOutcome(waitedMs: number, acknowledged: boolean): AskOutcome {
  if (acknowledged) return "delivered";
  return waitedMs >= ASK_WAIT_MS ? "kept" : "waiting";
}
