import { describeError } from "../errors";
import type { ChatEvent, ChatSpend, ContextEntry, ContextItemRef, ExchangeView } from "../generated/types";

export type TurnStatus = "starting" | "streaming" | "blocking" | "done" | "failed" | "cancelled";

/** One question and its answer, as the Ask view draws it. */
export interface Turn {
  query: string;
  status: TurnStatus;
  /** Set while streaming; Stop cancels this run. */
  runId: string | null;
  provider: string | null;
  model: string | null;
  /** What has streamed so far, then the final answer. */
  text: string;
  progress: string | null;
  /** The items a citation can resolve against. */
  context: ContextEntry[];
  citations: ContextItemRef[];
  error: string | null;
}

export function newTurn(query: string): Turn {
  return { query, status: "starting", runId: null, provider: null, model: null, text: "", progress: null, context: [], citations: [], error: null };
}

/** A saved exchange, with the entries it cited and the model that answered. */
export function savedTurn(e: ExchangeView): Turn {
  const citations = e.citations.map((c) => c.item);
  return { ...newTurn(e.query), status: "done", text: e.answer, model: e.model ?? null, context: e.citations, citations };
}

/** How one event from the shell changes the turn it belongs to. */
export function applyEvent(turn: Turn, event: ChatEvent): Turn {
  switch (event.kind) {
    case "started":
      return {
        ...turn,
        status: event.runId ? "streaming" : "blocking",
        runId: event.runId ?? null,
        provider: event.provider,
        model: event.model,
        context: event.context,
      };
    case "delta":
      return { ...turn, status: "streaming", text: turn.text + event.text, progress: null };
    case "progress":
      return { ...turn, progress: event.label };
    case "fell_back":
      // The stream never produced output; the blocking path starts over. Its chain can
      // fall through to any provider with a key and doesn't say which answered, so no
      // model is named from here on.
      return { ...turn, status: "blocking", runId: null, provider: null, model: null, text: "", progress: null };
    case "completed":
      return { ...turn, status: "done", runId: null, text: event.answer, citations: event.citations, progress: null };
    case "cancelled":
      return { ...turn, status: "cancelled", runId: null, progress: null };
    case "failed":
      return {
        ...turn,
        status: "failed",
        runId: null,
        progress: null,
        text: event.partial ? turn.text : "",
        error: describeError(event.error),
      };
    case "titled":
      return turn;
  }
}

export const isBusy = (turn: Turn | undefined): boolean =>
  !!turn && (turn.status === "starting" || turn.status === "streaming" || turn.status === "blocking");

/**
 * The items an answer cites, in citation order, once each. A citation whose item isn't
 * among the candidates is dropped, as on the Mac.
 */
export function citedEntries(citations: ContextItemRef[], context: ContextEntry[]): ContextEntry[] {
  const out: ContextEntry[] = [];
  for (const c of citations) {
    const hit = context.find((e) => e.item.kind === c.kind && e.item.id === c.id);
    if (hit && !out.includes(hit)) out.push(hit);
  }
  return out;
}

export function spendLine(spend: ChatSpend): string {
  return spend.capCents == null ? `${spend.userCents}¢ today · no cap` : `${spend.userCents}¢ / ${spend.capCents}¢ today`;
}

/** The small line under an answer that isn't finished yet. */
export function statusLine(turn: Turn): string | null {
  const model = turn.model ?? "the model";
  switch (turn.status) {
    case "starting":
      return "Finding context…";
    case "blocking":
      return `Asking ${model}…`;
    case "streaming":
      return turn.text ? turn.progress : (turn.progress ?? `Connecting to ${model}…`);
    default:
      return null;
  }
}

/** A question handed to Ask from elsewhere (Today's composer). `id` makes each hand-off distinct. */
export interface AskRequest {
  id: number;
  prompt: string;
}

/**
 * The prompt Ask should send for this request, or null: none pending, a blank one, or one already
 * sent (`handledId`). Keeps a re-render or a return to the tab from sending the same question twice.
 */
export function promptToSend(request: AskRequest | null | undefined, handledId: number | null): string | null {
  if (!request || request.id === handledId) return null;
  const prompt = request.prompt.trim();
  return prompt === "" ? null : prompt;
}
