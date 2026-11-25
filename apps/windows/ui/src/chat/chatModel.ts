import { describeError } from "../errors";
import { t } from "../l10n";
import type { ChatEvent, ChatSpend, ContextEntry, ContextItemRef, ConversationRow, ExchangeView } from "../generated/types";

export type TurnStatus = "starting" | "waiting" | "streaming" | "blocking" | "done" | "failed" | "cancelled";

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
  /** The error was a spent monthly budget: the turn offers "Open AI settings". */
  budgetSpent?: boolean;
  /** Retrieved items a hosted model answered without, because they involve an excluded client (core's
   *  `filtered_hosted`; items, not clients). Shown under the answer only, never saved. */
  leftOut?: number | null;
  /** The question involves a client excluded from hosted models (its name, an alias or a known contact's address): a model on this PC answers it (PR 874). */
  routedLocally?: boolean;
  /** The answer was made on this PC for that reason: a note under it says so, never naming the client. */
  answeredLocally?: boolean;
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
        routedLocally: false,
      };
    case "waiting":
      // Only before the answer has started: a late one changes nothing.
      return turn.status === "starting" ? { ...turn, status: "waiting" } : turn;
    case "routed_locally":
      return { ...turn, routedLocally: true };
    case "answered_by":
      // The model that really answered (the chain may have moved past the picked one).
      return { ...turn, model: event.model || null, answeredLocally: event.excludedClient };
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
      return { ...turn, status: "done", runId: null, text: event.answer, citations: event.citations, progress: null, leftOut: event.leftOut ?? null };
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
        budgetSpent: event.error.kind === "budget_spent",
      };
    case "titled":
    // The sourced ask's two events belong to the redesigned Find page (library/redesign/askSession.ts); this model draws none of it.
    case "sourced":
    case "cited":
    // A question asked inside one topic that found nothing: the Conversations page handles it before the turn sees it.
    case "scope_empty":
      return turn;
  }
}

export const isBusy = (turn: Turn | undefined): boolean =>
  !!turn && (turn.status === "starting" || turn.status === "waiting" || turn.status === "streaming" || turn.status === "blocking");

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
  return spend.capCents == null
    ? t("{used}¢ today · no cap", { used: String(spend.userCents) })
    : t("{used}¢ / {cap}¢ today", { used: String(spend.userCents), cap: String(spend.capCents) });
}

/** The spinner while a model on this PC answers a question that involves an excluded client (PR 874). */
export const answeringLocallyLine = () => t("Answering on this PC: this question involves a client excluded from hosted models…");

/** Under an answer made on this PC for that reason. Never names the client. */
export const answeredLocallyLine = () => t("Answered locally: this question involves a client excluded from hosted models.");

/** The small line under an answer that isn't finished yet. */
export function statusLine(turn: Turn): string | null {
  const model = turn.model;
  switch (turn.status) {
    case "starting":
      return t("Finding context…");
    case "waiting":
      return t("Waiting for background work…");
    case "blocking":
      if (turn.routedLocally) return answeringLocallyLine();
      return model ? t("Asking {model}…", { model }) : t("Asking the model…");
    case "streaming":
      if (turn.text || turn.progress) return turn.progress;
      return model ? t("Connecting to {model}…", { model }) : t("Connecting to the model…");
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

/** The names of a conversation row's Star, Rename and Delete buttons. Each holds the conversation's title, so moving through the list
 *  with Tab reads "Star Bake sale prices", not a bare "Star" with no word of which conversation it acts on. */
export function conversationActionLabels(row: Pick<ConversationRow, "title" | "starred">): { star: string; rename: string; delete: string } {
  const title = row.title;
  return {
    star: row.starred ? t("Unstar {title}", { title }) : t("Star {title}", { title }),
    rename: t("Rename {title}", { title }),
    delete: t("Delete {title}", { title }),
  };
}
