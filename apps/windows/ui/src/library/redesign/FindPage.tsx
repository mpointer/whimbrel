// The Find page (redesign flag on): the words typed in the header, results from this PC on a time line, and an answer when asked for.
// Enter in the header shows results and sends nothing anywhere. The Ask button, named for where the answer comes from, writes the answer.
// (docs/ui-redesign/screens/ask.md)

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { chatSpend, chatStarterPrompts, conversationScope, getRoutingSettings } from "../../api";
import { openConversation as openConversationPage } from "../../conversations/nav";
import { citedEntries, spendLine } from "../../chat/chatModel";
import { LocalModelCard } from "../../chat/LocalModelCard";
import type { ChatSpend, ConversationRow, RoutePreviewView, StarterPromptView } from "../../generated/types";
import { t, useL10n } from "../../l10n";
import { BetterTranscriptsCard } from "../../settings/BetterTranscriptsCard";
import { WButton, WIcon, WNotice, WSegmented } from "../../system";
import { TopicPageOffer } from "../../topics/TopicChip";
import { hasFilter, NO_FILTER } from "../../today/redesign/homeModel";
import { useAskState } from "./askState";
import { AnswerCard } from "./AnswerCard";
import { FindCardView } from "./FindCard";
import { FindResultsView } from "./FindResultsView";
import { ConversationPanel } from "./ConversationPanel";
import {
  askLabel,
  askReadiness,
  centeredScrollTop,
  countLabel,
  effectiveQuery,
  evidenceOf,
  filterInput,
  filterKey,
  filtersOnLine,
  defaultOrder,
  followFormShown,
  hasFindFilter,
  placeRestored,
  requestHeld,
  resolveSend,
  routeLine,
  searchedAfterEdit,
  searchesAgain,
  singleFlight,
  type SendPress,
  withoutEvidence,
  type FindOpen,
} from "./findModel";
import { headerState, peekRequest, requestFind, setFindCount, setFindShown, setHeaderFilter, setHeaderText, setOrder, setSearched, takeRequest, useHeader } from "./findStore";
import { ask, askSnapshot, cancelQueued, continuedConversation, filtersChanged, loadConversations, newConversation, openConversation, setConversations, setError, stop, takeRestore, useAsk, useContinuedConversation } from "./askStore";
import { routePreviews, useRoutePreview } from "./routePreview";
import { useFindResults } from "./useFind";
import "./find.css";

export function FindPage({ hidden, onOpen, onOpenSettings }: { hidden: boolean; onOpen: (open: FindOpen) => void; onOpenSettings?: () => void }) {
  const { locale } = useL10n();
  const header = useHeader();
  const session = useAsk();
  const state = useAskState();
  // Until the AI settings have been read, nothing is known: the page neither offers set-up nor sends the person to Settings.
  const readiness = askReadiness(state);
  const available = readiness === "ready";
  // What was searched and in which order live in the store: the page is drawn again each time the tab is shown.
  const { searched, order } = header;
  const [selected, setSelected] = useState<{ turn: number; n: number } | null>(null);
  const [follow, setFollow] = useState("");
  const [panel, setPanel] = useState(false);
  const [note, setNote] = useState<string | null>(null);
  /** Words of a question that was waiting and could not go back where they came from (that field holds other words now). */
  const [returned, setReturned] = useState<{ text: string; to: "header" | "follow" } | null>(null);
  const [starters, setStarters] = useState<StarterPromptView[]>([]);
  const [spend, setSpend] = useState<ChatSpend | null>(null);
  const [localOnly, setLocalOnly] = useState(false);
  const [focusAsk, setFocusAsk] = useState(0);
  const answerRef = useRef<HTMLDivElement>(null);
  const askRow = useRef<HTMLDivElement>(null);

  const fi = useMemo(() => filterInput(header.filter, header.filterAt || Date.now()), [header.filter, header.filterAt]);
  const key = filterKey(fi);
  const query = effectiveQuery(searched, hasFindFilter(fi));
  const found = useFindResults(query, fi, order);
  const turns = session.turns;
  const last = turns[turns.length - 1];
  // The conversation a question asked now continues: the one on screen, once it has answers and when it was asked under these filters.
  const conversationId = useContinuedConversation(key);

  // The header shows the chips and the count while this page is on screen.
  useEffect(() => {
    setFindShown(!hidden);
    return () => setFindShown(false);
  }, [hidden]);
  const count = countLabel(found.page);
  useEffect(() => setFindCount(hidden ? null : count), [count, hidden]);

  useEffect(() => {
    loadConversations();
    chatStarterPrompts().then(setStarters, () => {});
    chatSpend().then(setSpend, () => {});
    getRoutingSettings().then((r) => setLocalOnly(r.localOnly), () => {});
  }, []);
  useEffect(() => {
    if (last?.status === "done") chatSpend().then(setSpend, () => {});
  }, [last?.status]);

  // Changing a filter after an answer starts a new conversation (the page says so). Only a change seen here counts: drawing the page again
  // when the tab is shown is not a change, and must not wipe an answer that is still arriving.
  const seenKey = useRef(key);
  useEffect(() => {
    if (seenKey.current === key) return;
    seenKey.current = key;
    filtersChanged(fi);
  }, [key]); // eslint-disable-line react-hooks/exhaustive-deps

  const followShown = followFormShown(last);
  const followNow = useRef(follow);
  followNow.current = follow;
  const preview = useRoutePreview(header.text, conversationId, fi, available);
  const followPreview = useRoutePreview(follow, conversationId, fi, available && followShown);

  /** What a press of Ask is about, read now: the words in the field it came from, the filters on the chips, the conversation it would continue. */
  const pressNow = (fromFollow: boolean): SendPress => {
    const h = headerState();
    const k = filterKey(filterInput(h.filter, h.filterAt || Date.now()));
    return { words: (fromFollow ? followNow.current : h.text).trim(), filterKey: k, conversationId: continuedConversation(askSnapshot(), k) };
  };

  /** Sends a question: the route is read again just before, so the button's promise is checked against the same decision the call makes,
   *  and what was pressed is checked again after, since the words, the chips or the conversation may have moved while it was read. */
  const sendNow = async (text: string, shown: RoutePreviewView | null, fromFollow: boolean) => {
    const words = text.trim();
    if (!words) return;
    if (readiness !== "ready") {
      if (readiness === "unavailable") onOpenSettings?.();
      return;
    }
    if (askSnapshot().loading) return;
    setNote(null);
    const pressed: SendPress = { words, filterKey: key, conversationId };
    const route = await resolveSend(shown, () => routePreviews.read(words, conversationId, fi, { fresh: true }), pressed, () => pressNow(fromFollow));
    if (!route.go) {
      if (route.why === "changed") setNote(t("The words, the filters or the conversation changed while this was being checked, so nothing was sent. Check them, then press Ask again."));
      else {
        setNote(t("Where this answer would come from has changed since the button was drawn. Check the button, then ask again."));
        // The label reads again, so what it names is true when the person looks.
        routePreviews.clear();
      }
      return;
    }
    const result = ask(words, fi, route.requireLocal, fromFollow ? "follow" : "header");
    if (result === "queue_full") {
      setNote(t("One question is already waiting for the answer in progress. Take it back to send a different one."));
      return;
    }
    if (result === "loading") return;
    if (fromFollow) setFollow("");
  };
  // A second press of the same control while the first is still reading the route does nothing; the one in flight is the question.
  // A press of the other control is its own question and is never dropped for it.
  const latestSendNow = useRef(sendNow);
  latestSendNow.current = sendNow;
  const send = useMemo(
    () => singleFlight((text: string, shown: RoutePreviewView | null, fromFollow: boolean) => latestSendNow.current(text, shown, fromFollow), (_text, _shown, fromFollow) => (fromFollow ? "follow" : "header")),
    [],
  );
  const sendHeader = () => void send(header.text, preview, false);

  // An emptied field (cleared, or every word deleted) leaves no results for words no longer there; Enter on it shows the empty state.
  useEffect(() => {
    const now = headerState();
    setSearched(searchedAfterEdit(now.text, now.searched));
  }, [header.text]);

  // A search or a question asked for from the header, from Home or from another tab. A search runs at once. A question waits only for
  // the AI settings to be read, and is not taken until then.
  const latestSend = useRef(sendHeader);
  latestSend.current = sendHeader;
  useEffect(() => {
    const request = peekRequest();
    if (!request) return;
    const words = header.text.trim();
    const next = words === "" && !hasFindFilter(fi) ? null : words;
    // The same words again are a new search too: what is stored may have changed since (new items, their meaning now read).
    const again = searchesAgain(next, headerState().searched);
    // New words open on the order their shape suggests (a question on best matches first); the same words again keep the picker's choice.
    if (next !== headerState().searched) setOrder(defaultOrder(next));
    setSearched(next);
    setNote(null);
    if (requestHeld(request.mode, readiness)) return;
    takeRequest();
    if (again && request.mode === "search") found.reload();
    if (request.mode === "ask") {
      if (words === "") return;
      if (readiness === "unavailable") onOpenSettings?.();
      else latestSend.current();
    }
    // Keyed on the request and on what is known about asking: the words and filters are read as they are now.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [header.request, readiness]);

  // Words go back into the field they came from, once, and never over what has been typed since. A question that was waiting is never
  // lost to that rule: when its field holds other words, or is not on screen, the page offers the words instead.
  useEffect(() => {
    if (!session.restore) return;
    const r = takeRestore();
    if (!r) return;
    const followField = r.to === "follow" && turns.length > 0 && followFormShown(last);
    const here = followField ? followNow.current : headerState().text;
    const where = placeRestored(here, r.text, r.queued);
    if (where === "put") {
      if (followField) setFollow(r.text);
      else setHeaderText(r.text);
    } else if (where === "offer") setReturned({ text: r.text, to: followField ? "follow" : "header" });
  }, [session.restore, turns.length]); // eslint-disable-line react-hooks/exhaustive-deps

  // A starter was chosen: the Ask button, with the question ready, is where focus goes.
  useEffect(() => {
    if (focusAsk > 0) askRow.current?.querySelector<HTMLButtonElement>("button:not(:disabled)")?.focus();
  }, [focusAsk]);

  const evidence = useMemo(() => (last ? evidenceOf(last.sources, last.cited) : []), [last?.sources, last?.cited]); // eslint-disable-line react-hooks/exhaustive-deps
  const results = useMemo(() => withoutEvidence(found.results, evidence), [found.results, evidence]);
  const mentions = useMemo(() => withoutEvidence(found.mentions, evidence), [found.mentions, evidence]);
  const onMark = useCallback((turn: number, n: number) => {
    setSelected({ turn, n });
    const id = turn === -1 ? `th-find-source-${n}` : `th-find-source-t${turn}-${n}`;
    requestAnimationFrame(() => {
      const card = document.getElementById(id);
      if (!card) return;
      const calm = window.matchMedia?.("(prefers-reduced-motion: reduce)").matches;
      // Only the page's own scroll container moves (scrollIntoView would scroll the window too, cutting off the header).
      const box = scrollParent(card);
      if (box) {
        const b = box.getBoundingClientRect();
        const c = card.getBoundingClientRect();
        const top = centeredScrollTop({ scrollTop: box.scrollTop, scrollHeight: box.scrollHeight, top: b.top, height: box.clientHeight }, { top: c.top, height: c.height });
        box.scrollTo({ top, behavior: calm ? "auto" : "smooth" });
      }
      card.focus({ preventScroll: true });
    });
  }, []);

  const clearFilters = () => setHeaderFilter(NO_FILTER);
  const empty = query === null && turns.length === 0;
  const searching = found.loading && found.results.length === 0 && found.mentions.length === 0;
  const noResults = query !== null && !found.loading && !found.error && found.results.length === 0 && found.mentions.length === 0;
  const askText = header.text.trim();
  const route = routeLine(preview);
  // Which answer a waiting question waits for: said by name when it is not the one on screen.
  const answeringElsewhere = session.inFlight && !session.inFlight.onScreen ? session.inFlight : null;
  const queuedLine = answeringElsewhere
    ? t("Your question is waiting for an answer still being written in another conversation: {title}.", {
        title: session.conversations.find((c) => c.id === answeringElsewhere.conversationId)?.title ?? answeringElsewhere.query,
      })
    : t("Your question is waiting for the answer in progress.");
  const semanticMissing = found.page != null && !found.page.semanticAvailable && !!query;

  // On a narrow window the conversations sit under the whole page: opening them brings them into view (the page's own scroll only).
  useEffect(() => {
    if (!panel) return;
    requestAnimationFrame(() => {
      const side = document.getElementById("th-find-convs");
      const box = side && scrollParent(side);
      if (!side || !box) return;
      const b = box.getBoundingClientRect();
      const c = side.getBoundingClientRect();
      if (c.top >= b.top && c.top < b.bottom - 48) return;
      const calm = window.matchMedia?.("(prefers-reduced-motion: reduce)").matches;
      const top = centeredScrollTop({ scrollTop: box.scrollTop, scrollHeight: box.scrollHeight, top: b.top, height: box.clientHeight }, { top: c.top, height: Math.max(c.height, box.clientHeight) });
      box.scrollTo({ top, behavior: calm ? "auto" : "smooth" });
    });
  }, [panel]);

  const openConv = (id: number) => {
    // A topic or meeting conversation opens on the Conversations page, which keeps its scope (Find asks over everything, filters and all);
    // one about everything opens here under the Dates and App it kept. A scope that cannot be read opens here too.
    conversationScope(id).then(
      (scope) => {
        if (scope.kind !== "everything") return openConversationPage(id);
        openConversation(id);
        requestAnimationFrame(() => answerRef.current?.focus());
      },
      () => {
        openConversation(id);
        requestAnimationFrame(() => answerRef.current?.focus());
      },
    );
  };
  const ranks = useMemo(() => [{ value: "time" as const, label: t("By day") }, { value: "rank" as const, label: t("Best matches first") }], [locale]); // eslint-disable-line react-hooks/exhaustive-deps

  return (
    <div className="th-find-wrap" hidden={hidden}>
      <div className="th-find-main">
        <BetterTranscriptsCard />
        {query ? <TopicPageOffer text={query} /> : null}

        <section className="th-find-ask" aria-label={t("Ask")}>
          <div className="th-find-ask-row" ref={askRow}>
            {readiness === "unavailable" ? (
              <WButton variant="primary" onClick={() => onOpenSettings?.()}>
                {t("Set up an AI provider")}
              </WButton>
            ) : (
              <WButton variant="primary" icon="spark" hint="Ctrl+Enter" disabled={askText === "" || !available || session.loading} onClick={sendHeader}>
                {askLabel(available ? preview : null)}
              </WButton>
            )}
            <span className="th-find-route">
              {route ?? (askText === "" ? t("Type in the field above. Enter searches on this PC and sends nothing to a model. Ask writes an answer.") : t("Nothing is sent until you press Ask."))}
            </span>
            <WButton icon="chat" aria-expanded={panel} aria-controls="th-find-convs" onClick={() => setPanel(!panel)}>
              {panel ? t("Hide conversations") : t("Conversations")}
            </WButton>
          </div>
          {note ? <WNotice tone="warning">{note}</WNotice> : null}
          {turns.length === 0 ? <LocalModelCard onOpenSettings={onOpenSettings} /> : null}
          {localOnly ? (
            <p className="th-find-small">
              {t("Local-only mode is on, so every answer comes from a local model on this PC. Turn it off in Settings → AI to use a hosted provider.")}
            </p>
          ) : null}
          {spend ? <p className="th-find-small">{spendLine(spend)}</p> : null}
        </section>

        {session.startedOver ? (
          <WNotice tone="info">{t("The filters changed, so this is a new conversation. Your earlier answers are in the conversation list.")}</WNotice>
        ) : null}
        {session.filtersCleared ? <WNotice tone="info">{t("Filters were cleared: a saved conversation does not keep its filters.")}</WNotice> : null}
        {session.peopleNotKept ? <WNotice tone="info">{t("The People filter was not kept with this conversation, so follow-ups are not limited to a person.")}</WNotice> : null}
        {session.error ? <WNotice tone="error">{session.error}</WNotice> : null}
        {returned ? (
          <div className="w-notice w-notice-info">
            <WIcon name="info" />
            <p>
              {t("Your waiting question was not sent: {question}", { question: returned.text })}{" "}
              <button
                type="button"
                className="th-find-link"
                onClick={() => {
                  if (returned.to === "follow") setFollow(returned.text);
                  else setHeaderText(returned.text);
                  setReturned(null);
                }}
              >
                {t("Put the words back")}
              </button>{" "}
              <button type="button" className="th-find-link" onClick={() => setReturned(null)}>
                {t("Dismiss")}
              </button>
            </p>
          </div>
        ) : null}
        {session.queued ? (
          // Said politely by the live region below; focus stays where it is.
          <div className="w-notice w-notice-info">
            <WIcon name="info" />
            <p>
              {queuedLine}{" "}
              <button type="button" className="th-find-link" onClick={cancelQueued}>
                {t("Take it back")}
              </button>
            </p>
          </div>
        ) : null}
        <div className="w-visually-hidden" role="status" aria-live="polite">
          {session.queued ? queuedLine : ""}
        </div>

        <div ref={answerRef} tabIndex={-1} className="th-find-answers">
          {turns.map((turn, i) => {
            const isLast = i === turns.length - 1;
            const turnEvidence = isLast ? [] : evidenceOf(turn.sources, turn.cited);
            return (
              <div key={i} className="th-find-turn">
                <AnswerCard
                  turn={turn}
                  filter={fi}
                  selected={selected && selected.turn === (isLast ? -1 : i) ? selected.n : null}
                  onMark={(n) => onMark(isLast ? -1 : i, n)}
                  onStop={stop}
                  last={isLast}
                  legacyChips={<LegacyChips turn={turn} onOpen={onOpen} />}
                />
                {turnEvidence.length > 0 ? (
                  <ul className="th-find-cards th-find-earlier" aria-label={t("Sources for this answer")}>
                    {turnEvidence.map((e) => (
                      <li key={e.n} className="th-find-item">
                        <FindCardView card={e.card} number={e.n} unreadable={e.unreadable} idPrefix={`th-find-source-t${i}`} selected={selected?.turn === i && selected.n === e.n} now={new Date()} locale={locale} onOpen={onOpen} />
                      </li>
                    ))}
                  </ul>
                ) : null}
              </div>
            );
          })}
          {followShown ? (
            <form
              className="th-find-follow"
              onSubmit={(e) => {
                e.preventDefault();
                void send(follow, followPreview, true);
              }}
            >
              <input type="text" value={follow} onChange={(e) => setFollow(e.target.value)} aria-label={t("Follow up")} placeholder={t("Follow up")} />
              <WButton type="submit" variant="primary" disabled={follow.trim() === "" || session.loading}>
                {askLabel(followPreview)}
              </WButton>
            </form>
          ) : null}
        </div>

        <div className="th-find-order">
          {query !== null ? (
            <WSegmented label={t("Order")} value={order} options={ranks} onChange={setOrder} />
          ) : null}
          {semanticMissing ? (
            <p className="th-find-small">{t("Meaning search needs the search model from Settings → Semantic search. Keyword results still show.")}</p>
          ) : null}
        </div>

        {found.error ? <WNotice tone="error">{found.error}</WNotice> : null}
        {searching ? (
          <p className="th-find-status" role="status">
            {t("Searching…")}
          </p>
        ) : null}
        {empty && !found.loading ? (
          <section className="th-find-empty" aria-label={t("Find")}>
            <p>{t("Ask about anything Whimbrel has captured, on this PC or your other devices.")}</p>
            <p className="th-find-small">{t("Find searches captures, meeting transcripts, slides, email and episodes in your work memory. Press Enter to see results.")}</p>
            {starters.length > 0 ? (
              <>
                <p className="th-find-small" id="th-find-starters-h">
                  {t("Questions you can ask. Picking one fills the field and searches; nothing is sent until you press Ask.")}
                </p>
                <ul className="th-find-starters" aria-labelledby="th-find-starters-h">
                  {starters.map((p) => (
                    <li key={p.id}>
                      <WButton
                        onClick={() => {
                          setHeaderText(p.query);
                          requestFind("search");
                          setFocusAsk((n) => n + 1);
                        }}
                      >
                        {p.title}
                      </WButton>
                    </li>
                  ))}
                </ul>
              </>
            ) : null}
          </section>
        ) : null}
        {noResults ? (
          <section className="th-find-empty" aria-label={t("No results")}>
            <p role="status">{query ? t("No results for these words in your work memory.") : t("Nothing in your work memory matches these filters.")}</p>
            {hasFilter(header.filter) ? (
              <p className="th-find-small">
                {t("Filters on: {filters}.", { filters: filtersOnLine(header.filter) })}{" "}
                <button type="button" className="th-find-link" onClick={clearFilters}>
                  {t("Clear filters")}
                </button>
              </p>
            ) : null}
          </section>
        ) : null}

        <FindResultsView evidence={evidence} selected={selected && selected.turn === -1 ? selected.n : null} results={results} mentions={mentions} person={header.filter.person} grouped={order === "time"} onOpen={onOpen} />
        {found.more ? (
          <div className="th-find-more-row">
            <WButton onClick={found.loadMore} disabled={found.loadingMore} aria-busy={found.loadingMore}>
              {found.loadingMore ? t("Loading…") : t("Load more")}
            </WButton>
          </div>
        ) : null}
      </div>
      {panel ? (
        <div id="th-find-convs" className="th-find-side">
          <ConversationPanel
            rows={session.conversations}
            activeId={session.activeId}
            canStartNew={session.activeId !== null || turns.length > 0}
            answeringId={session.inFlight?.conversationId ?? null}
            onNew={newConversation}
            onOpen={openConv}
            onChange={setConversations}
            onDeleted={(rows: ConversationRow[], id: number) => {
              setConversations(rows);
              if (id === session.activeId) newConversation();
            }}
            onError={setError}
          />
        </div>
      ) : null}
    </div>
  );
}

/** Today's chips for an answer saved before numbered sources: each opens what it cited, in the old words. */
export function LegacyChips({ turn, onOpen }: { turn: import("./askSession").SourcedTurn; onOpen: (open: FindOpen) => void }) {
  const cited = citedEntries(turn.citations, turn.context);
  if (cited.length === 0) return null;
  return (
    <div className="th-find-legacy">
      <span className="th-find-small">{t("Sources")}</span>
      {cited.map((entry) => {
        const target = entry.open;
        const key = `${entry.item.kind}-${entry.item.id}`;
        return target ? (
          <button key={key} type="button" className="chip citation" onClick={() => onOpen({ kind: target.kind, id: target.id })}>
            {entry.label}
          </button>
        ) : (
          <span key={key} className="chip">
            {entry.label}
          </span>
        );
      })}
    </div>
  );
}

/** The nearest ancestor that scrolls up and down (the page's content area), or null. */
function scrollParent(el: HTMLElement): HTMLElement | null {
  for (let p = el.parentElement; p; p = p.parentElement) {
    const y = getComputedStyle(p).overflowY;
    if ((y === "auto" || y === "scroll") && p.scrollHeight > p.clientHeight) return p;
  }
  return null;
}
