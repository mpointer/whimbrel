import { useCallback, useEffect, useRef, useState, type ReactNode } from "react";
import ReactMarkdown from "react-markdown";
import {
  chatAsk,
  chatConversation,
  chatConversations,
  chatDeleteConversation,
  chatRenameConversation,
  chatSpend,
  chatStarConversation,
  chatStarterPrompts,
  chatStop,
  getRoutingSettings,
} from "../api";
import { describeError, isCommandError } from "../errors";
import type { ChatSpend, ConversationRow, StarterPromptView } from "../generated/types";
import { Detail, type Selection } from "../library/Detail";
import {
  applyEvent,
  citedEntries,
  isBusy,
  newTurn,
  promptToSend,
  savedTurn,
  spendLine,
  statusLine,
  type AskRequest,
  type Turn,
} from "./chatModel";

function message(e: unknown): string {
  return isCommandError(e) ? describeError(e) : String(e);
}

/**
 * Answers are model output, so they are untrusted. react-markdown builds React elements
 * and never emits raw HTML. Links and images render as text, so nothing in an answer can
 * navigate the app window or load a remote resource.
 */
const markdownComponents = {
  a: ({ children, href }: { children?: ReactNode; href?: string }) => (
    <span className="md-link" title={href}>
      {children}
    </span>
  ),
  img: ({ alt }: { alt?: string }) => <span>{alt ?? ""}</span>,
};

/** Library → Ask: grounded chat with saved conversations (plan Task 8). */
export function AskView({ request, onRequestHandled }: { request?: AskRequest | null; onRequestHandled?: () => void }) {
  const [conversations, setConversations] = useState<ConversationRow[]>([]);
  const [activeId, setActiveId] = useState<number | null>(null);
  const [turns, setTurns] = useState<Turn[]>([]);
  const [composer, setComposer] = useState("");
  const [starters, setStarters] = useState<StarterPromptView[]>([]);
  const [spend, setSpend] = useState<ChatSpend | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [selected, setSelected] = useState<Selection | null>(null);
  // Local-only mode overrides the preferred provider, so Ask says so.
  const [localOnly, setLocalOnly] = useState(false);
  // Bumped whenever the view switches conversation. An answer still arriving for the
  // previous view is saved by the shell but no longer drawn here (the Mac's rule).
  const view = useRef(0);

  const refreshConversations = useCallback(() => {
    chatConversations()
      .then(setConversations)
      .catch((e: unknown) => setError(message(e)));
  }, []);
  const refreshSpend = useCallback(() => {
    chatSpend()
      .then(setSpend)
      .catch(() => {});
  }, []);

  useEffect(() => {
    refreshConversations();
    refreshSpend();
    chatStarterPrompts()
      .then(setStarters)
      .catch(() => {});
    getRoutingSettings()
      .then((r) => setLocalOnly(r.localOnly))
      .catch(() => {});
  }, [refreshConversations, refreshSpend]);

  // A question in flight, whichever conversation is on screen: switching views clears
  // `turns`, but the first answer still holds the store until it finishes (the Mac's
  // view-wide `isAsking`), so a second question would only queue behind it.
  const [asking, setAsking] = useState(false);
  const last = turns[turns.length - 1];
  const busy = asking || isBusy(last);

  const open = (id: number | null) => {
    view.current += 1;
    const token = view.current;
    setActiveId(id);
    setTurns([]);
    setError(null);
    if (id === null) return;
    chatConversation(id)
      .then((c) => {
        if (token === view.current && c) setTurns(c.exchanges.map(savedTurn));
      })
      .catch((e: unknown) => setError(message(e)));
  };

  const ask = (text: string) => {
    const query = text.trim();
    if (!query || busy) return;
    const token = view.current;
    const conversationId = activeId;
    const update = (f: (turn: Turn) => Turn) => setTurns((t) => (t.length ? [...t.slice(0, -1), f(t[t.length - 1])] : t));
    setComposer("");
    setError(null);
    setAsking(true);
    setTurns((t) => [...t, newTurn(query)]);
    chatAsk(query, conversationId, (event) => {
      if (event.kind === "completed" || event.kind === "cancelled" || event.kind === "failed") setAsking(false);
      if (event.kind === "completed" || event.kind === "titled") refreshConversations();
      if (event.kind === "completed") refreshSpend();
      if (token !== view.current) return;
      if (event.kind === "completed" && conversationId === null && event.conversationId != null) setActiveId(event.conversationId);
      if (event.kind === "cancelled") {
        // No bubble; the question goes back into the composer to edit or resend.
        setTurns((t) => t.slice(0, -1));
        setComposer(query);
        return;
      }
      if (event.kind === "failed") setComposer(query);
      update((turn) => applyEvent(turn, event));
    }).catch((e: unknown) => {
      setAsking(false);
      if (token !== view.current) return;
      update((turn) => ({ ...turn, status: "failed", error: message(e) }));
      setComposer(query);
    });
  };

  // A question from Today's composer: sent once, as a new conversation (this view opens on none).
  // This view mounts fresh, so nothing here is streaming; an answer still running from before holds
  // the store and this question waits behind it, as any second question does.
  const handled = useRef<number | null>(null);
  useEffect(() => {
    const prompt = promptToSend(request, handled.current);
    if (!request || prompt === null) return;
    handled.current = request.id;
    ask(prompt);
    onRequestHandled?.();
    // Keyed on the request: `ask` is recreated every render and must not retrigger this.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [request]);

  if (selected) return <Detail selection={selected} onBack={() => setSelected(null)} />;

  const runId = busy ? last?.runId : null;

  return (
    <div className="ask">
      <aside className="ask-sidebar" aria-label="Conversations">
        <button className="mode mode--active" onClick={() => open(null)} disabled={activeId === null && turns.length === 0}>
          New conversation
        </button>
        <ul className="conversation-list">
          {conversations.map((c) => (
            <ConversationItem
              key={c.id}
              row={c}
              active={c.id === activeId}
              onOpen={() => open(c.id)}
              onChange={setConversations}
              onDeleted={(rows) => {
                setConversations(rows);
                if (c.id === activeId) open(null);
              }}
              onError={setError}
            />
          ))}
        </ul>
      </aside>
      <section className="ask-main" aria-label="Ask">
        {turns.length === 0 && (
          <div className="starters">
            <p className="muted">Ask about anything Whimbrel has captured, on this PC or your other devices.</p>
            <div className="chips">
              {starters.map((p) => (
                <button key={p.id} className="mode starter" onClick={() => ask(p.query)} disabled={busy}>
                  {p.title}
                </button>
              ))}
            </div>
          </div>
        )}
        <ol className="turns">
          {turns.map((turn, i) => (
            <TurnView key={i} turn={turn} onOpen={setSelected} />
          ))}
        </ol>
        {error && <p role="alert">{error}</p>}
        <form
          className="composer"
          onSubmit={(e) => {
            e.preventDefault();
            ask(composer);
          }}
        >
          <textarea
            className="input grow"
            rows={2}
            value={composer}
            placeholder="Ask about your meetings, captures and episodes"
            aria-label="Question"
            onChange={(e) => setComposer(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter" && !e.shiftKey) {
                e.preventDefault();
                ask(composer);
              }
            }}
          />
          {runId ? (
            <button type="button" className="mode" onClick={() => chatStop(runId)}>
              Stop
            </button>
          ) : (
            <button type="submit" className="mode mode--active" disabled={busy || !composer.trim()}>
              Ask
            </button>
          )}
        </form>
        {localOnly && (
          <p className="muted small">
            Local-only mode is on, so every answer comes from Ollama on this PC. Turn it off in Settings → AI &amp; routing to use
            a hosted provider.
          </p>
        )}
        {spend && <p className="muted small">{spendLine(spend)}</p>}
      </section>
    </div>
  );
}

function TurnView({ turn, onOpen }: { turn: Turn; onOpen: (s: Selection) => void }) {
  const cited = citedEntries(turn.citations, turn.context);
  const status = statusLine(turn);
  return (
    <li className="turn">
      <p className="bubble bubble--question">{turn.query}</p>
      <div
        className={turn.status === "failed" ? "bubble bubble--answer bubble--error" : "bubble bubble--answer"}
        aria-live={turn.status === "streaming" ? "polite" : undefined}
      >
        {turn.status === "done" ? (
          <div className="markdown">
            <ReactMarkdown components={markdownComponents}>{turn.text}</ReactMarkdown>
          </div>
        ) : turn.text ? (
          // Plain text until it is complete: half-streamed markdown reflows badly.
          <p className="streaming-text">{turn.text}</p>
        ) : null}
        {status && (
          <p className="muted small" role="status">
            {status}
          </p>
        )}
        {turn.error && <p role="alert">{turn.error}</p>}
        {cited.length > 0 && (
          <div className="citations">
            <span className="muted small">Sources</span>
            {cited.map((entry) => {
              const target = entry.open;
              const key = `${entry.item.kind}-${entry.item.id}`;
              return target ? (
                <button key={key} className="chip citation" onClick={() => onOpen(target)}>
                  {entry.label}
                </button>
              ) : (
                <span key={key} className="chip">
                  {entry.label}
                </span>
              );
            })}
          </div>
        )}
        {turn.status === "done" && turn.model && <p className="muted small">{turn.model}</p>}
      </div>
    </li>
  );
}

function ConversationItem(props: {
  row: ConversationRow;
  active: boolean;
  onOpen: () => void;
  onChange: (rows: ConversationRow[]) => void;
  onDeleted: (rows: ConversationRow[]) => void;
  onError: (e: string | null) => void;
}) {
  const { row, active, onOpen, onChange, onDeleted, onError } = props;
  const [editing, setEditing] = useState(false);
  const [title, setTitle] = useState(row.title);
  const [confirmDelete, setConfirmDelete] = useState(false);

  // A generated title can land while the row is shown.
  useEffect(() => {
    if (!editing) setTitle(row.title);
  }, [row.title, editing]);

  const act = (p: Promise<ConversationRow[]>, then: (rows: ConversationRow[]) => void) =>
    p
      .then((rows) => {
        then(rows);
        onError(null);
      })
      .catch((e: unknown) => onError(message(e)));

  return (
    <li className={active ? "conversation conversation--active" : "conversation"}>
      {editing ? (
        <form
          className="field-row"
          onSubmit={(e) => {
            e.preventDefault();
            act(chatRenameConversation(row.id, title), (rows) => {
              onChange(rows);
              setEditing(false);
            });
          }}
        >
          <input className="input grow" value={title} onChange={(e) => setTitle(e.target.value)} aria-label="Conversation title" autoFocus />
          <button type="submit" className="mode">
            Save
          </button>
          <button type="button" className="mode" onClick={() => setEditing(false)}>
            Cancel
          </button>
        </form>
      ) : confirmDelete ? (
        <span className="field-row small">
          Delete this conversation?
          <button className="mode" onClick={() => act(chatDeleteConversation(row.id), onDeleted)}>
            Delete
          </button>
          <button className="mode" onClick={() => setConfirmDelete(false)}>
            Keep
          </button>
        </span>
      ) : (
        <>
          <button className="conversation-title" onClick={onOpen} title={row.title} aria-current={active ? "true" : undefined}>
            {row.title}
          </button>
          <span className="conversation-actions">
            <button
              className={row.starred ? "icon icon--on" : "icon"}
              aria-label={row.starred ? "Unstar" : "Star"}
              aria-pressed={row.starred}
              onClick={() => act(chatStarConversation(row.id, !row.starred), onChange)}
            >
              {row.starred ? "★" : "☆"}
            </button>
            <button className="icon" aria-label="Rename" onClick={() => setEditing(true)}>
              ✎
            </button>
            <button className="icon" aria-label="Delete" onClick={() => setConfirmDelete(true)}>
              ✕
            </button>
          </span>
        </>
      )}
    </li>
  );
}
