// One question and its answer, from the shell's events to a `SourcedTurn` the screens draw. Shared by Today's answer and the conversation
// page, so both read the same events the Find page reads (`applySourcedEvent`) and say the same words.

import { describeError, isCommandError } from "../errors";
import type { ChatEvent, FindFilterInput } from "../generated/types";
import { applySourcedEvent, newSourcedTurn, type SourcedTurn } from "../library/redesign/askSession";
import { localOnlyFailure } from "../library/redesign/findModel";

export type TurnEnd = "completed" | "cancelled" | "failed" | "scope_empty";

export interface TurnHooks {
  /** The turn as it is now: called for the first (nothing yet) and after every event. */
  onTurn(turn: SourcedTurn): void;
  /** Called once, when the question has ended however it ended. `turn` is its last state. */
  onEnd(end: TurnEnd, turn: SourcedTurn): void;
  /** A generated title arrived (the shell sends it after the answer, on the same channel). */
  onTitled?(conversationId: number, title: string): void;
  /** Stops a run by its id (`chat_stop`). Called by `RunHandle.stop`, at once when the id is known and as soon as it arrives when not. */
  stopRun?(runId: string): void;
  /** The answer was asked held to this PC: a failure for want of a local model says so in the existing words. */
  requireLocal?: boolean;
}

export const NO_FILTER_INPUT: FindFilterInput = { startUnixMs: null, endUnixMs: null, app: null, person: null, personKey: null };

export const messageOf = (e: unknown): string => (isCommandError(e) ? describeError(e) : String(e));

/** Starts a question. `start` calls the shell with the handler for its events; a refused call ends it as a failure. The shell's reply says
 *  only that the answer started, never that it ended: the question ends on its own terminal event. */
export interface RunHandle {
  /** Asks for the run to stop. A run whose id has not arrived yet is stopped the moment it does, so Stop, Clear, a new question or leaving the page
   *  never lets an answer carry on unseen. */
  stop(): void;
}

export function runTurn(query: string, start: (onEvent: (event: ChatEvent) => void) => Promise<unknown>, hooks: TurnHooks): RunHandle {
  let turn = newSourcedTurn(query);
  let over = false;
  let stopWanted = false;
  let stopSent = false;
  const sendStop = () => {
    if (!stopWanted || stopSent || !turn.runId) return;
    stopSent = true;
    hooks.stopRun?.(turn.runId);
  };
  const end = (kind: TurnEnd) => {
    if (over) return;
    over = true;
    hooks.onEnd(kind, turn);
  };
  hooks.onTurn(turn);
  const handle = (event: ChatEvent) => {
    if (event.kind === "titled") return hooks.onTitled?.(event.conversationId, event.title);
    if (over) return;
    if (event.kind === "scope_empty") return end("scope_empty");
    turn = applySourcedEvent(turn, event);
    // A question that had to stay on this PC never hears it could have gone to a hosted model.
    const local = event.kind === "failed" ? localOnlyFailure(event.error, hooks.requireLocal === true) : null;
    if (local) turn = { ...turn, error: local };
    hooks.onTurn(turn);
    sendStop();
    if (event.kind === "completed" || event.kind === "cancelled" || event.kind === "failed") end(event.kind);
  };
  start(handle).catch((e: unknown) => {
    if (over) return;
    turn = { ...turn, status: "failed", error: messageOf(e) };
    hooks.onTurn(turn);
    end("failed");
  });
  return {
    stop() {
      if (over) return;
      stopWanted = true;
      sendStop();
    },
  };
}

/** A turn that has not ended (nothing asked yet, preparing, waiting, streaming). */
export const inFlight = (turn: SourcedTurn | null | undefined): boolean =>
  !!turn && (turn.status === "starting" || turn.status === "waiting" || turn.status === "streaming" || turn.status === "blocking");
