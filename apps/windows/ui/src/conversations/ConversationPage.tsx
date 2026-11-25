// A conversation's page (redesign flag only; screen 6, parts 3 to 5): the header (title, click to rename; star; the topic chip; a menu with
// Delete), the scope line, the exchanges oldest first in the same parts as Today's answer, and the composer. Provider and model are never
// picked here; the model that answered is named in small text under each answer. A stored conversation, or a new one stored with its first answer.

import { useEffect, useMemo, useRef, useState, useSyncExternalStore } from "react";
import { openAiSettings } from "../api";
import { t, useL10n } from "../l10n";
import { useSkin } from "../useSkin";
import { localizedVoice, voiceText } from "../today/layoutModel";
import { useAskState } from "../library/redesign/askState";
import { Detail, type Selection } from "../library/Detail";
import { askReadiness, type FindOpen } from "../library/redesign/findModel";
import { useRoutePreview } from "../library/redesign/routePreview";
import { menuKeyDown, WButton, WIcon, WIconButton, WMoreMenu, WNotice } from "../system";
import { ExchangeView } from "./ExchangeView";
import { composerLabel, menuTopics } from "./model";
import { createPageController, type PageController, type PageState } from "./pageController";
import type { ConversationScopeView, RoutePreviewView, TopicRefView } from "../generated/types";
import "./conversations.css";

export interface ConversationPageProps {
  /** A stored conversation, or a new one (`draft`). */
  conversationId: number | null;
  draft: { scope: ConversationScopeView; name: string | null; question: string | null; shown?: RoutePreviewView | null } | null;
  /** The way back to the list. */
  onBack: () => void;
  /** Bumped when stored data changed under the page (a generated title landed): it reads again when idle. */
  version: number;
}

function usePage(ctrl: PageController): PageState {
  return useSyncExternalStore(ctrl.subscribe, ctrl.state);
}

export function ConversationPage({ conversationId, draft, onBack, version }: ConversationPageProps) {
  const ctrl = useMemo(() => createPageController(), []);
  const page = usePage(ctrl);
  const [selected, setSelected] = useState<Selection | null>(null);
  const opener = useRef<string | null>(null);
  const root = useRef<HTMLDivElement>(null);
  const detail = useRef<HTMLDivElement>(null);
  const heading = useRef<HTMLHeadingElement>(null);

  useEffect(() => {
    if (draft) ctrl.startDraft(draft.scope, draft.name, draft.question, draft.shown ?? null);
    else if (conversationId !== null) void ctrl.open(conversationId);
    // A question in flight is stopped and late events dropped when the page goes.
    return () => ctrl.dispose();
  }, [ctrl]); // eslint-disable-line react-hooks/exhaustive-deps
  useEffect(() => {
    if (version > 0) void ctrl.refresh();
  }, [version]); // eslint-disable-line react-hooks/exhaustive-deps
  // Opened (from the list, Today's Continue or Find): focus goes to the title, where the page begins.
  const loaded = page.status === "ready";
  useEffect(() => {
    if (!loaded) return;
    // A new conversation with nothing asked yet starts at its composer; any other page starts at its title.
    const empty = page.id === null && page.turns.length === 0;
    const composer = empty ? root.current?.querySelector<HTMLElement>(".th-cv-composer input") : null;
    (composer ?? heading.current)?.focus();
  }, [loaded]); // eslint-disable-line react-hooks/exhaustive-deps

  const open = (target: FindOpen) => {
    opener.current = document.activeElement?.getAttribute("data-open-key") ?? null;
    setSelected({ kind: target.kind, id: target.id, line: target.line });
    // The button that was pressed is hidden now: focus moves to the detail, which is read from its start.
    requestAnimationFrame(() => detail.current?.focus());
  };
  const back = () => {
    const key = opener.current;
    setSelected(null);
    requestAnimationFrame(() => {
      const button = key ? document.querySelector<HTMLElement>(`[data-open-key="${key}"]`) : null;
      (button ?? heading.current)?.focus();
    });
  };

  if (page.status === "missing") {
    return (
      <div className="th-cv-page">
        <WNotice tone="error">{t("This item is no longer available.")}</WNotice>
        <div className="th-cv-actions">
          <WButton onClick={onBack}>{t("Conversations")}</WButton>
        </div>
      </div>
    );
  }
  return (
    <div className="th-cv-page" ref={root}>
      {selected ? (
        <div ref={detail} tabIndex={-1} className="th-find-detail">
          <Detail selection={selected} onBack={back} />
        </div>
      ) : null}
      <div className="th-cv-body" hidden={!!selected}>
        <div className="th-cv-back">
          <WButton variant="quiet" onClick={onBack}>
            {t("Conversations")}
          </WButton>
        </div>
        {page.status === "loading" ? (
          <p className="th-find-status" role="status">
            {t("Loading…")}
          </p>
        ) : (
          <>
            <Header ctrl={ctrl} page={page} headingRef={heading} onDeleted={onBack} />
            {page.scopeLine || page.filterNote ? (
              <div className="th-cv-scopes">
                {page.scopeLine ? <p className="th-cv-scope">{page.scopeLine}</p> : null}
                {page.filterNote ? <p className="th-cv-scope">{page.filterNote}</p> : null}
              </div>
            ) : null}
            <Transcript ctrl={ctrl} page={page} onOpen={open} />
            <Composer ctrl={ctrl} page={page} />
          </>
        )}
      </div>
    </div>
  );
}

function Header({ ctrl, page, headingRef, onDeleted }: { ctrl: PageController; page: PageState; headingRef: React.RefObject<HTMLHeadingElement>; onDeleted: () => void }) {
  const stored = page.id !== null;
  const [renaming, setRenaming] = useState(false);
  const [draftTitle, setDraftTitle] = useState(page.title);
  const [confirm, setConfirm] = useState(false);
  const field = useRef<HTMLInputElement>(null);
  const more = useRef<HTMLSpanElement>(null);
  const confirmBox = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (renaming) field.current?.select();
  }, [renaming]);
  // Delete asks first: focus goes to Cancel, the safe answer, and comes back to the menu's button when it is cancelled.
  useEffect(() => {
    if (confirm) confirmBox.current?.querySelector<HTMLElement>("[data-cancel]")?.focus();
  }, [confirm]);
  const commit = () => {
    setRenaming(false);
    void ctrl.rename(draftTitle);
    requestAnimationFrame(() => headingRef.current?.focus());
  };
  return (
    <header className="th-cv-header">
      <div className="th-cv-title-row">
        {renaming ? (
          <input
            ref={field}
            className="th-cv-title-field"
            value={draftTitle}
            aria-label={t("Conversation title")}
            onChange={(e) => setDraftTitle(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter") commit();
              if (e.key === "Escape") {
                setRenaming(false);
                requestAnimationFrame(() => headingRef.current?.focus());
              }
            }}
            onBlur={() => renaming && commit()}
          />
        ) : (
          <h2 className="th-cv-title" tabIndex={-1} ref={headingRef}>
            {stored ? (
              <button
                type="button"
                className="th-cv-title-button"
                title={t("Rename")}
                aria-label={t("Rename {title}", { title: page.title })}
                onClick={() => {
                  setDraftTitle(page.title);
                  setRenaming(true);
                }}
              >
                {page.title}
              </button>
            ) : (
              page.title
            )}
          </h2>
        )}
        {stored ? (
          <>
            <WIconButton icon="star" label={page.starred ? t("Unstar conversation") : t("Star conversation")} aria-pressed={page.starred} onClick={() => void ctrl.toggleStar()} />
            <TopicChip ctrl={ctrl} page={page} />
            <span ref={more} className="th-cv-more">
              <WMoreMenu disabled={page.busy} items={[{ key: "delete", label: t("Delete"), onPress: () => setConfirm(true) }]} />
            </span>
          </>
        ) : null}
      </div>
      {confirm ? (
        <div className="th-cv-confirm" ref={confirmBox} role="group" aria-label={t("Delete this conversation?")}>
          <span>{t("Delete this conversation?")}</span> <span className="th-find-small">{t("This can't be undone.")}</span>
          <WButton
            variant="destructive"
            disabled={page.busy}
            onClick={() => {
              void ctrl.remove().then((gone) => (gone ? onDeleted() : setConfirm(false)));
            }}
          >
            {t("Delete")}
          </WButton>
          <WButton
            data-cancel=""
            onClick={() => {
              setConfirm(false);
              requestAnimationFrame(() => more.current?.querySelector<HTMLElement>("button")?.focus());
            }}
          >
            {t("Cancel")}
          </WButton>
        </div>
      ) : null}
      {page.note ? <WNotice tone="warning">{page.note}</WNotice> : null}
    </header>
  );
}

/** The topic chip: the topic's name, or "Add to topic" when there is none. Its menu lists the topics the sources belong to, "Choose another
 *  topic…" and "Remove from topic"; each choice is the person's own and final. */
function TopicChip({ ctrl, page }: { ctrl: PageController; page: PageState }) {
  const [open, setOpen] = useState(false);
  const [all, setAll] = useState(false);
  const box = useRef<HTMLSpanElement>(null);
  const trigger = useRef<HTMLButtonElement>(null);
  const first = useRef<HTMLButtonElement>(null);
  useEffect(() => {
    if (open) first.current?.focus();
  }, [open, all]);
  const close = (refocus: boolean) => {
    setOpen(false);
    setAll(false);
    if (refocus) trigger.current?.focus();
  };
  const choose = (topic: TopicRefView | null) => {
    void ctrl.setTopic(topic ? topic.topicId : null);
    close(true);
  };
  const offered = menuTopics(page.citedTopics, page.topic);
  const shown = all ? page.topics : offered;
  const current = page.topic;
  return (
    <span
      className="w-more th-cv-chip"
      ref={box}
      onBlur={(e) => {
        if (open && !box.current?.contains(e.relatedTarget as Node | null)) close(false);
      }}
      onKeyDown={(e) => {
        if (e.key === "Escape" && open) {
          e.preventDefault();
          e.stopPropagation();
          close(true);
        }
      }}
    >
      <button
        type="button"
        ref={trigger}
        className={current ? "th-cv-topic" : "w-button w-button-quiet"}
        aria-haspopup="menu"
        aria-expanded={open}
        aria-label={current ? t("Topic: {name}. Change it", { name: current.name }) : t("Add to topic")}
        onClick={() => setOpen(!open)}
      >
        {current ? <span># {current.name}</span> : <span>{t("Add to topic")}</span>}
      </button>
      {open ? (
        <ul className="w-more-menu" role="menu" onKeyDown={menuKeyDown}>
          {all && shown.length === 0 ? (
            <li role="none">
              <p className="th-find-small th-cv-menu-note">{t("No topics yet")}</p>
            </li>
          ) : null}
          {shown.map((topic, i) => (
            <li key={topic.topicId} role="none">
              <button type="button" className="w-more-item th-cv-radio" role="menuitemradio" aria-checked={current?.topicId === topic.topicId} ref={i === 0 ? first : undefined} onClick={() => (current?.topicId === topic.topicId ? close(true) : choose(topic))}>
                {/* The check column only where the current topic is among the choices (every topic), so the short menu lines up. */}
                {all ? (
                  <span className="th-cv-check" aria-hidden="true">
                    {current?.topicId === topic.topicId ? <WIcon name="check" size={16} /> : null}
                  </span>
                ) : null}
                <span>{topic.name}</span>
              </button>
            </li>
          ))}
          {!all ? (
            <li role="none">
              <button type="button" className="w-more-item" role="menuitem" ref={shown.length === 0 ? first : undefined} onClick={() => setAll(true)}>
                {t("Choose another topic…")}
              </button>
            </li>
          ) : null}
          {!all && current ? (
            <li role="none">
              <button type="button" className="w-more-item" role="menuitem" onClick={() => choose(null)}>
                {t("Remove from topic")}
              </button>
            </li>
          ) : null}
        </ul>
      ) : null}
    </span>
  );
}

function Transcript({ ctrl, page, onOpen }: { ctrl: PageController; page: PageState; onOpen: (open: FindOpen) => void }) {
  const bottom = useRef<HTMLDivElement>(null);
  const last = page.turns[page.turns.length - 1];
  // The newest answer stays in view as it is written (the page's own scroll only, never the window's).
  useEffect(() => {
    let el: HTMLElement | null = bottom.current?.parentElement ?? null;
    while (el) {
      const y = getComputedStyle(el).overflowY;
      if ((y === "auto" || y === "scroll") && el.scrollHeight > el.clientHeight) break;
      el = el.parentElement;
    }
    if (el && page.busy) el.scrollTo({ top: el.scrollHeight });
  }, [page.turns.length, last?.text.length, page.busy]);
  return (
    <div className="th-cv-transcript">
      {page.turns.map((turn, i) => (
        <ExchangeView key={i} turn={turn} idPrefix={`th-cv-source-${i}`} last={i === page.turns.length - 1} onStop={ctrl.stop} onOpen={onOpen} />
      ))}
      {page.failed ? (
        <div className="th-cv-actions">
          <WButton variant="primary" onClick={ctrl.retry}>
            {t("Try again")}
          </WButton>
        </div>
      ) : null}
      {page.nothingInScope ? (
        <div className="th-cv-nothing">
          <p className="th-cv-asked">{t("You asked")}</p>
          <p className="th-cv-question">{page.nothingInScope}</p>
          <p role="status">{t("Nothing in this topic answers that question.")}</p>
          <WButton onClick={ctrl.askEverythingInstead}>{t("Ask about everything instead")}</WButton>
        </div>
      ) : null}
      <div ref={bottom} />
    </div>
  );
}

function Composer({ ctrl, page }: { ctrl: PageController; page: PageState }) {
  const readiness = askReadiness(useAskState());
  const { lang } = useL10n();
  const skin = useSkin();
  const [text, setText] = useState("");
  const input = useRef<HTMLInputElement>(null);
  const everything = page.scope.kind === "everything";
  const preview = useRoutePreview(text, page.id, page.filter, everything && readiness === "ready");
  // A stopped question goes back here, never over words typed since.
  useEffect(() => {
    if (page.restore) setText((now) => (now.trim() === "" ? page.restore!.text : now));
  }, [page.restore]);
  const label = composerLabel(page.scope, preview);
  const send = async () => {
    const words = text.trim();
    if (!words || page.busy) return;
    // The words leave the field only once the question has gone out. When it is refused (the route changed under the button's promise) they
    // stay, with the note saying why, so the question is not lost. Words typed meanwhile are never cleared.
    const started = await ctrl.ask(words, preview);
    if (started) setText((now) => (now.trim() === words ? "" : now));
  };
  if (readiness === "unavailable") {
    return (
      <div className="th-cv-composer">
        <WButton variant="primary" onClick={() => void openAiSettings()}>
          {t("Set up an AI provider")}
        </WButton>
      </div>
    );
  }
  // The skin's own words for the Ask field, as the Mac's composer takes them from the header's field.
  const placeholder = voiceText(localizedVoice(skin.skin, lang), "askPlaceholder");
  return (
    <form
      className="th-cv-composer"
      onSubmit={(e) => {
        e.preventDefault();
        void send();
      }}
    >
      <input ref={input} type="text" value={text} onChange={(e) => setText(e.target.value)} aria-label={placeholder} placeholder={placeholder} disabled={page.status !== "ready"} />
      <WButton type="submit" variant="primary" disabled={text.trim() === "" || page.busy || readiness !== "ready"}>
        {label}
      </WButton>
    </form>
  );
}
