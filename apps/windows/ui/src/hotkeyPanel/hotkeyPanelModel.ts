// The hotkey panel's facts and state (the Mac's HotkeyPanelModel). The strings are the Mac's, word for word.
// Window text never reaches the page: the shell sends only these views.

import { describeError, isCommandError, providerFailureReason } from "../errors";
import type { ChatEvent, DestinationView, HotkeyPanelRead, ProviderFailureReason, WindowAskReply, WindowReadView } from "../generated/types";
import { t } from "../l10n";
import { applyEvent, newTurn } from "../chat/chatModel";

/** "This will be answered on this computer", or who the text goes to. */
export function destinationText(destination: DestinationView): string {
  if (destination.kind !== "hosted") return t("This will be answered on this computer");
  switch (destination.provider) {
    case "anthropic":
      return t("The text above will be sent to Anthropic");
    case "openai":
      return t("The text above will be sent to OpenAI");
    case "google":
      return t("The text above will be sent to Google");
    case "openrouter":
      return t("The text above will be sent to OpenRouter");
    case "whimbrel":
      return t("The text above will be sent to Anthropic, through Whimbrel's server");
    default:
      // A provider this build doesn't know: name it, never claim the text stays here.
      return t("The text above will be sent to {provider}", { provider: destination.provider });
  }
}

/** The two lines above the field. `reading` is empty when nothing was read. */
export function contextLine(outcome: WindowReadView, destination: DestinationView, truncated: boolean): { reading: string; destination: string } {
  let reading = "";
  if (outcome.kind === "read") {
    const kind = outcome.isSelection ? t("selected text") : t("visible text");
    const title = outcome.title.trim();
    reading = title
      ? t("Reading: {app}, '{title}' ({kind})", { app: outcome.appName, title, kind })
      : t("Reading: {app} ({kind})", { app: outcome.appName, kind });
    if (truncated) reading += t(" — first part only");
  }
  return { reading, destination: destinationText(destination) };
}

/** Why the panel's destination can't answer, in words true for that destination: in local-only
 *  mode an API key would not help, so "Ollama isn't running" says only to start it. */
export function cannotAnswerText(reason: ProviderFailureReason, destination?: DestinationView | null): string {
  if (reason.kind === "local_model_missing") return t("No local model is installed.");
  if (reason.kind === "ollama_not_running" && destination?.kind === "local_only") {
    return t("Ollama isn't running on this computer. Start Ollama to answer here.");
  }
  return providerFailureReason(reason) ?? describeError({ kind: "provider_failure", reason });
}

/** An ask's error, for the panel: a provider failure in the destination's words (`cannotAnswerText`). */
export function askErrorText(err: unknown, destination: DestinationView | null): string {
  if (!isCommandError(err)) return String(err);
  if (err.kind === "provider_failure" && err.reason) return cannotAnswerText(err.reason, destination);
  return describeError(err);
}

/** Why the panel can't ask, or null when it can. `cannotAnswer` is the destination's own refusal. */
export function blockedReason(
  outcome: WindowReadView,
  cannotAnswer?: ProviderFailureReason | null,
  destination?: DestinationView | null,
): string | null {
  if (outcome.kind === "excluded") return t("This app is excluded from capture.");
  if (outcome.kind === "unreadable") return t("I couldn't read this window.");
  if (cannotAnswer) return cannotAnswerText(cannotAnswer, destination);
  return null;
}

export type Phase = "reading" | "blocked" | "ready" | "asking" | "answered" | "failed";

export interface PanelState {
  phase: Phase;
  /** The current request; replies for another are ignored. */
  requestId: number | null;
  read: HotkeyPanelRead | null;
  /** The destination now shown; it changes when the settings did. */
  destination: DestinationView | null;
  /** True after a DestinationChanged, until the next ask: Return again confirms the new line. */
  destinationChanged: boolean;
  blocked: string | null;
  /** Counts asks; stream events and replies for an older ask are dropped. */
  askSeq: number;
  answer: string;
  error: string | null;
  /** Set once an ask started: the first part only. */
  truncated: boolean;
}

export const initialState: PanelState = {
  phase: "reading",
  requestId: null,
  read: null,
  destination: null,
  destinationChanged: false,
  blocked: null,
  askSeq: 0,
  answer: "",
  error: null,
  truncated: false,
};

export type PanelAction =
  | { type: "read"; read: HotkeyPanelRead }
  | { type: "read_error"; message: string }
  | { type: "ask" }
  | { type: "reply"; askSeq: number; reply: WindowAskReply }
  | { type: "event"; askSeq: number; event: ChatEvent }
  | { type: "ask_error"; askSeq: number; message: string };

export function reduce(state: PanelState, action: PanelAction): PanelState {
  switch (action.type) {
    case "read": {
      const r = action.read;
      if (state.requestId !== null && r.requestId < state.requestId) return state;
      // canAnswer false with no reason: the check timed out or failed, which is not "no provider".
      const blocked =
        !r.canAnswer && !r.cannotAnswer && r.window.kind === "read"
          ? t("Couldn't check the AI provider in time.")
          : blockedReason(r.window, r.canAnswer ? null : r.cannotAnswer, r.destination);
      return {
        ...initialState,
        requestId: r.requestId,
        read: r,
        destination: r.destination,
        blocked,
        phase: blocked ? "blocked" : "ready",
        truncated: r.window.kind === "read" && r.window.truncated,
      };
    }
    case "read_error":
      return { ...initialState, phase: "failed", error: action.message };
    case "ask":
      // A blocked or unread panel never asks. A second ask while one runs supersedes it (askSeq);
      // the view itself refuses to start one while asking.
      if (state.phase === "blocked" || state.phase === "reading" || state.requestId === null) return state;
      return { ...state, phase: "asking", askSeq: state.askSeq + 1, answer: "", error: null, destinationChanged: false };
    case "reply": {
      if (action.askSeq !== state.askSeq || action.reply.requestId !== state.requestId) return state;
      const reply = action.reply;
      if (reply.kind === "destination_changed") {
        return { ...state, phase: "ready", destination: reply.destination, destinationChanged: true, answer: "" };
      }
      if (reply.kind === "stale") return { ...state, phase: "ready" };
      return { ...state, destination: reply.destination, truncated: reply.truncated };
    }
    case "event": {
      if (action.askSeq !== state.askSeq || state.phase === "ready" || state.phase === "blocked") return state;
      const turn = applyEvent({ ...newTurn(""), text: state.answer }, action.event);
      switch (action.event.kind) {
        case "delta":
          return { ...state, phase: "asking", answer: turn.text };
        case "completed":
          return { ...state, phase: "answered", answer: turn.text };
        case "cancelled":
          return { ...state, phase: "ready", answer: "" };
        case "failed":
          return { ...state, phase: "failed", answer: turn.text, error: turn.error };
        case "fell_back":
          // No fallback in the panel; the shell never sends this, but clear rather than mix answers.
          return { ...state, answer: "" };
        default:
          return state;
      }
    }
    case "ask_error":
      if (action.askSeq !== state.askSeq) return state;
      return { ...state, phase: "failed", error: action.message };
  }
}
