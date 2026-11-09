import { t } from "../l10n";
import type { ChatEvent, ContextItemRef, MeetingAskExchange } from "../generated/types";
import { applyEvent, newTurn, type Turn } from "../chat/chatModel";

/**
 * The meeting Ask bar's quick questions, the Mac's `AskPreset`. The label is shown (and sent
 * as the question shown); the prompt is what the model is asked. Prompts are English on
 * purpose: the answer's language follows Settings through the routing config.
 */
export interface AskPreset {
  id: string;
  /** English: the catalog key, and what typing it matches in any app language. */
  label: string;
  prompt: string;
}

export const ASK_PRESETS: AskPreset[] = [
  {
    id: "makeMeSoundSmart",
    label: "Make me sound smart",
    prompt: [
      "Make me sound smart in this meeting. From the transcript so far, answer in Markdown with these sections:",
      "## Where we are",
      "Two sentences on what's being discussed right now.",
      "## Key points",
      "The main points and any decisions so far, as short bullets.",
      "## Where I come in",
      "Points that connect to my profile (my role, projects and expertise), and why they matter to me. Leave this section out if nothing connects.",
      "## Say this next",
      "Two or three smart questions or contributions I could make right now, worded the way I'd say them.",
      "Keep it short enough to read in under a minute.",
    ].join("\n"),
  },
  {
    id: "bringMeUpToSpeed",
    label: "Bring me up to speed",
    prompt: [
      "Bring me up to speed: I've just joined this meeting or lost the thread. From the transcript so far, answer in Markdown with these sections:",
      "## What this is about",
      "One or two sentences.",
      "## What's happened",
      "The main points in the order they came up, as short bullets. Start a decision with **Decided:**.",
      "## Where it's heading",
      "Open questions and what's being discussed right now.",
      "## Anything for me",
      "Anything said to me, about me, or about my projects (from my profile). Leave this section out if there's nothing.",
      "Keep it short enough to read in under a minute.",
    ].join("\n"),
  },
  {
    id: "anyoneAskMe",
    label: "Did anyone ask me a question?",
    prompt: [
      "Did anyone ask me a question, or ask me to do something, in this meeting? Recognize me by my name and the other names I go by from my profile. Lines from my own microphone are mine, not questions to me.",
      "From the transcript so far, answer in Markdown: one bullet per question or request, with who asked (if known), roughly when, what they asked in a few words, and one sentence I could reply with.",
      "If nobody did, say so in one sentence.",
    ].join("\n"),
  },
];

const key = (s: string) =>
  s
    .trim()
    .replace(/^[\p{P}\s]+|[\p{P}\s]+$/gu, "")
    .toLowerCase();

/** The preset whose label (translated or English) `text` is, ignoring case, surrounding space and end punctuation. */
export function matchingPreset(text: string): AskPreset | null {
  const typed = key(text);
  if (!typed) return null;
  return ASK_PRESETS.find((p) => key(t(p.label)) === typed || key(p.label) === typed) ?? null;
}

/** The cited transcript lines, first cited first, once each: the chips "Transcript 1", "Transcript 2"… */
export function transcriptCitationIds(citations: ContextItemRef[]): number[] {
  const out: number[] = [];
  for (const c of citations) if (c.kind === "transcript" && !out.includes(c.id)) out.push(c.id);
  return out;
}

/** Why the bar can't take a question yet, or null (the Mac's `MeetingAskAvailability`). */
export function disabledReason(state: { recording: boolean; liveLines: number; processing: boolean; hasTranscript: boolean }): string | null {
  if (state.recording) return state.liveLines > 0 ? null : t("Listening. The transcript starts appearing after the first minute.");
  if (state.processing) return t("Transcribing — Ask will be available shortly.");
  if (!state.hasTranscript) return t("There is no transcript for this meeting to ask about.");
  return null;
}

/** One question on the meeting page and its answer. Never saved. */
export interface MeetingExchange {
  /** What the bar shows: the typed question, or a preset's label. */
  question: string;
  /** What the model was asked. */
  prompt: string;
  turn: Turn;
  narrowed: boolean;
}

export function newExchange(question: string): MeetingExchange {
  const preset = matchingPreset(question);
  return { question: preset ? t(preset.label) : question, prompt: preset ? preset.prompt : question, turn: newTurn(question), narrowed: false };
}

export function applyMeetingEvent(exchange: MeetingExchange, event: ChatEvent): MeetingExchange {
  return {
    ...exchange,
    turn: applyEvent(exchange.turn, event),
    narrowed: event.kind === "completed" ? event.narrowed : exchange.narrowed,
  };
}

/** The answered exchanges, as the model saw them, for a follow-up question. */
export function historyOf(exchanges: MeetingExchange[]): MeetingAskExchange[] {
  return exchanges.filter((e) => e.turn.status === "done" && e.turn.text.trim() !== "").map((e) => ({ query: e.prompt, answer: e.turn.text }));
}
