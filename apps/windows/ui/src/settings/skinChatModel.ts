// The skin chat's state (skins Task 11; the Mac's SkinChatSession). Every send and every Stop bumps
// `run`, and a step from a stale run changes nothing, so a late reply after Stop or a skin switch is
// dropped. The shell does the saving and keeps each skin's chat (src-tauri/src/skin_maker.rs).

import { isCommandError } from "../errors";
import type { ChatLine, ImageCandidateView, SkinLine, SkinSources, SkinTurnResult } from "../generated/types";
import { providerLabel } from "./aiModel";
import { isUserSkin } from "./skinGalleryModel";

/** At most this many found images are offered for one turn. */
export const SEARCH_LIMIT = 6;
export const SPEND_CAP_LINE = "Today's AI spending limit is reached.";
export const SET_UP_LINE = "Set up an AI provider in Settings to make skins.";
export const FAILED_LINE = "Whimbrel couldn't make that skin, so nothing changed. Try again or say it another way.";
/** The shell's answer to an approval in local-only mode (skin_maker.rs); the image stays in the strip. */
export const LOCAL_ONLY_LINE = "Local-only mode is on, so Whimbrel didn't download that image.";

export interface ChatState {
  /** The skin whose chat is shown, and the one `candidates` are for. */
  skinId: string;
  turns: ChatLine[];
  /** What the current (or last) turn did, step by step. */
  status: string[];
  candidates: ImageCandidateView[];
  running: boolean;
  run: number;
}

export type ChatEvent =
  /** The chat of `skinId`, as stored. Another skin's drops the strip, and stops a running turn. */
  | { type: "show"; skinId: string; turns: ChatLine[] }
  | { type: "send"; text: string; providerLine: string | null }
  | { type: "stop" }
  /** The turn's reply; `turns` is the saved skin's chat, read again when the turn stored it. */
  | { type: "replied"; run: number; result: SkinTurnResult; turns: ChatLine[] | null }
  | { type: "status"; run: number; line: string }
  | { type: "found"; run: number; candidates: ImageCandidateView[] }
  /** A Whimbrel line for `skinId`'s chat; ends `run` when given. */
  | { type: "line"; skinId: string; text: string; run?: number }
  | { type: "done"; run: number }
  | { type: "forget"; url: string };

export function initialChat(skinId: string, turns: ChatLine[] = []): ChatState {
  return { skinId, turns, status: [], candidates: [], running: false, run: 0 };
}

/** A line as the chat shows it; the date only orders the stored file. */
const line = (role: "user" | "whimbrel", text: string): ChatLine => ({ role, text, date: Date.now() / 1000 - 978_307_200 });

export function chatReducer(state: ChatState, event: ChatEvent): ChatState {
  const current = (run: number) => run === state.run && state.running;
  switch (event.type) {
    case "show":
      if (event.skinId === state.skinId) return { ...state, turns: event.turns };
      // Another skin: a running turn stops there, so its late reply changes nothing.
      if (state.running) return { ...state, skinId: event.skinId, turns: event.turns, candidates: [], running: false, run: state.run + 1, status: [...state.status, "Stopped."] };
      return { ...state, skinId: event.skinId, turns: event.turns, candidates: [], status: [] };
    case "send": {
      const status = ["Reading your request…", ...(event.providerLine ? [event.providerLine] : [])];
      return { ...state, run: state.run + 1, running: true, status, turns: [...state.turns, line("user", event.text)] };
    }
    case "stop":
      if (!state.running) return state;
      return { ...state, run: state.run + 1, running: false, status: [...state.status, "Stopped."] };
    case "replied": {
      if (!current(event.run)) return state;
      const { result } = event;
      const status = [...state.status, ...result.notes];
      const working = result.imageQueries.length > 0 || result.imagePrompts.length > 0;
      // Stored: the saved skin's chat as the shell wrote it. Otherwise only shown, after what was asked.
      const turns = event.turns ?? [...state.turns, ...result.lines.map((text) => line("whimbrel", text))];
      const moved = result.chatSkinId !== state.skinId;
      return { ...state, status, turns, skinId: result.chatSkinId, candidates: moved ? [] : state.candidates, running: working };
    }
    case "status":
      return current(event.run) ? { ...state, status: [...state.status, event.line] } : state;
    case "found": {
      if (!current(event.run)) return state;
      const fresh = event.candidates.filter((c) => !state.candidates.some((k) => k.url === c.url));
      return { ...state, candidates: [...state.candidates, ...fresh] };
    }
    case "line": {
      const ends = event.run !== undefined && current(event.run);
      if (event.run !== undefined && !ends) return state;
      const turns = event.skinId === state.skinId ? [...state.turns, line("whimbrel", event.text)] : state.turns;
      return { ...state, turns, running: ends ? false : state.running };
    }
    case "done":
      return current(event.run) ? { ...state, running: false } : state;
    case "forget":
      return { ...state, candidates: state.candidates.filter((c) => c.url !== event.url) };
  }
}

/** The status line naming where the request goes; none for Ollama, which stays on this PC. */
export function providerLine(sources: SkinSources): string | null {
  if (sources.provider === "ollama") return null;
  return `Sent to ${providerLabel(sources.provider)}: your request, this skin's chat, its design and the list of allowed values`;
}

export const placeholder = (skinId: string): string => (isUserSkin(skinId) ? "Ask for a change…" : "Describe a skin…");

/** What the chat offers below its turns: the composer, a way to set up a provider, or (until the shell has
 *  said which) the composer, disabled, so opening Settings doesn't flash the set-up button. */
export function composer(sources: SkinSources | null | undefined): "compose" | "set-up" | "loading" {
  if (sources === undefined) return "loading";
  return sources?.chat ? "compose" : "set-up";
}

const isSpendCap = (e: unknown): boolean => isCommandError(e) && e.kind === "spend_cap";

/** The shell calls a turn makes (api.ts in the app; fakes in tests). */
export interface SkinChatApi {
  turn(skinId: string, request: string, runId: string): Promise<SkinTurnResult>;
  stop(runId: string): Promise<boolean>;
  chat(skinId: string): Promise<ChatLine[]>;
  search(query: string): Promise<ImageCandidateView[]>;
  generate(prompt: string): Promise<ImageCandidateView>;
  post(skinId: string, text: string): Promise<SkinLine>;
}

export interface Turn {
  /** The run `send` started (the state's `run` after it). */
  run: number;
  runId: string;
  skinId: string;
  request: string;
}

/** Runs a turn after `send`: the reply, then the images it asked for, one call at a time so the status
 *  reads in order and Stop takes effect between calls. `live` says whether the run is still current. */
export async function runTurn(api: SkinChatApi, turn: Turn, dispatch: (e: ChatEvent) => void, live: () => boolean): Promise<SkinTurnResult | null> {
  const { run } = turn;
  let result: SkinTurnResult;
  try {
    result = await api.turn(turn.skinId, turn.request, turn.runId);
  } catch (e) {
    dispatch({ type: "line", skinId: turn.skinId, text: isSpendCap(e) ? SPEND_CAP_LINE : FAILED_LINE, run });
    return null;
  }
  if (!live() || result.intent === null) return null;
  const turns = result.stored ? await api.chat(result.chatSkinId).catch(() => null) : null;
  dispatch({ type: "replied", run, result, turns });
  const skinId = result.chatSkinId;
  const stopOnCap = async (e: unknown): Promise<boolean> => {
    if (!isSpendCap(e)) return false;
    const posted = await api.post(skinId, SPEND_CAP_LINE).catch(() => ({ text: SPEND_CAP_LINE, stored: false }));
    dispatch({ type: "line", skinId, text: posted.text, run });
    return true;
  };
  let found = 0;
  for (const query of result.imageQueries) {
    if (!live() || found >= SEARCH_LIMIT) break;
    dispatch({ type: "status", run, line: `Searching the web for '${query}'…` });
    try {
      const results = (await api.search(query)).slice(0, SEARCH_LIMIT - found);
      found += results.length;
      dispatch({ type: "found", run, candidates: results });
    } catch (e) {
      if (await stopOnCap(e)) return result;
    }
  }
  for (const prompt of result.imagePrompts) {
    if (!live()) break;
    dispatch({ type: "status", run, line: `Making an image: '${prompt}'…` });
    try {
      dispatch({ type: "found", run, candidates: [await api.generate(prompt)] });
    } catch (e) {
      if (await stopOnCap(e)) return result;
    }
  }
  dispatch({ type: "done", run });
  return result;
}
