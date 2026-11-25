// The Conversations section (redesign flag only; screen 6, part 6): the list, or one conversation's page, or a new one. The list shows every
// conversation, starred first and then most recently updated, each with its title, its topic, when it was last updated and the first line of
// its latest answer. New conversation is at the top; a search over titles; star, rename and delete on each row.

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { chatDeleteConversation, chatRenameConversation, chatStarConversation, conversationsList } from "../api";
import { conversationActionLabels } from "../chat/chatModel";
import type { ConversationListRow } from "../generated/types";
import { t, useL10n } from "../l10n";
import { useAsk } from "../library/redesign/askStore";
import { WButton, WIcon, WIconButton, WNotice } from "../system";
import { ConversationPage } from "./ConversationPage";
import { filterRows, sortRows, updatedLabel } from "./model";
import { conversationsChanged, openConversation, showConversationList, startConversation, takeListReturn, useConversationNav } from "./nav";
import { messageOf } from "./runTurn";
import "./conversations.css";

export function ConversationsView() {
  const nav = useConversationNav();
  const { view } = nav;
  if (view.kind === "list") return <ConversationsList version={nav.version} />;
  if (view.kind === "page") return <ConversationPage key={`page-${view.id}`} conversationId={view.id} draft={null} onBack={() => showConversationList(view.id)} version={nav.version} />;
  return (
    <ConversationPage
      key={`draft-${view.key}`}
      conversationId={null}
      draft={{ scope: view.scope, name: view.name, question: view.question, shown: view.shown }}
      onBack={() => showConversationList(null)}
      version={nav.version}
    />
  );
}

type Read = { kind: "loading" } | { kind: "rows"; rows: ConversationListRow[] } | { kind: "failed" };

function ConversationsList({ version }: { version: number }) {
  const { locale } = useL10n();
  const [read, setRead] = useState<Read>({ kind: "loading" });
  const [query, setQuery] = useState("");
  const [renaming, setRenaming] = useState<number | null>(null);
  const [confirming, setConfirming] = useState<number | null>(null);
  const [note, setNote] = useState<string | null>(null);
  const root = useRef<HTMLDivElement>(null);
  const session = useAsk();
  // A conversation an answer is being written in (on the Find page) is not deleted under its stream.
  const answering = session.inFlight?.conversationId ?? null;

  const load = useCallback(() => {
    conversationsList().then(
      (rows) => setRead({ kind: "rows", rows }),
      () => setRead({ kind: "failed" }),
    );
  }, []);
  useEffect(load, [load, version]);
  // Back from a page (or a delete) puts focus where the person was: the conversation's row, or New conversation when it is gone.
  const returned = useRef(takeListReturn());
  useEffect(() => {
    if (read.kind !== "rows" || !returned.current) return;
    const id = returned.current.id;
    returned.current = null;
    requestAnimationFrame(() => {
      const target = (id !== null ? root.current?.querySelector<HTMLElement>(`[data-conv-row="${id}"]`) : null) ?? root.current?.querySelector<HTMLElement>("[data-conv-new]");
      target?.focus();
    });
  }, [read.kind]);
  const rows = read.kind === "rows" ? read.rows : [];
  const shown = useMemo(() => filterRows(sortRows(rows), query), [rows, query]);
  const now = Date.now();

  const focusRow = (id: number | null) =>
    requestAnimationFrame(() => {
      const target = (id !== null ? root.current?.querySelector<HTMLElement>(`[data-conv-row="${id}"]`) : null) ?? root.current?.querySelector<HTMLElement>("[data-conv-new]");
      target?.focus();
    });
  const act = (work: Promise<unknown>, then?: () => void) =>
    work.then(
      () => {
        setNote(null);
        then?.();
        conversationsChanged();
      },
      (e: unknown) => setNote(messageOf(e)),
    );
  const remove = (row: ConversationListRow) => {
    const index = shown.findIndex((r) => r.id === row.id);
    const next = shown[index + 1] ?? shown[index - 1] ?? null;
    void act(chatDeleteConversation(row.id), () => {
      setConfirming(null);
      focusRow(next ? next.id : null);
    });
  };

  return (
    <div className="th-cv-list" ref={root}>
      <div className="th-cv-list-head">
        <WButton variant="primary" data-conv-new="" onClick={() => startConversation({ kind: "everything" }, null)}>
          {t("New conversation")}
        </WButton>
        {rows.length > 0 ? <input type="search" className="th-cv-search" value={query} onChange={(e) => setQuery(e.target.value)} aria-label={t("Search")} placeholder={t("Search")} /> : null}
      </div>
      {note ? <WNotice tone="error">{note}</WNotice> : null}
      {read.kind === "failed" ? (
        <div className="th-cv-empty" role="alert">
          <p>{t("Couldn't load your conversations.")}</p>
          <WButton onClick={load}>{t("Try again")}</WButton>
        </div>
      ) : read.kind === "loading" ? (
        <p className="th-find-status" role="status">
          {t("Loading…")}
        </p>
      ) : rows.length === 0 ? (
        <p className="th-cv-empty">{t("Questions you continue from Today show up here.")}</p>
      ) : shown.length === 0 ? (
        <p className="th-cv-empty" role="status">
          {t("No matches")}
        </p>
      ) : (
        <ul className="th-cv-rows" aria-label={t("Conversations")}>
          {shown.map((row) => (
            <Row
              key={row.id}
              row={row}
              now={now}
              locale={locale}
              answering={row.id === answering}
              renaming={renaming === row.id}
              confirming={confirming === row.id}
              onRename={(on) => {
                setRenaming(on ? row.id : null);
                if (!on) focusRow(row.id);
              }}
              onSaveTitle={(title) => void act(chatRenameConversation(row.id, title), () => setRenaming(null)).then(() => focusRow(row.id))}
              onStar={() => void act(chatStarConversation(row.id, !row.starred))}
              onConfirm={(on) => {
                setConfirming(on ? row.id : null);
                if (!on) focusRow(row.id);
              }}
              onDelete={() => remove(row)}
            />
          ))}
        </ul>
      )}
    </div>
  );
}

interface RowProps {
  row: ConversationListRow;
  now: number;
  locale: string;
  answering: boolean;
  renaming: boolean;
  confirming: boolean;
  onRename: (on: boolean) => void;
  onSaveTitle: (title: string) => void;
  onStar: () => void;
  onConfirm: (on: boolean) => void;
  onDelete: () => void;
}

function Row({ row, now, locale, answering, renaming, confirming, onRename, onSaveTitle, onStar, onConfirm, onDelete }: RowProps) {
  const [title, setTitle] = useState(row.title);
  useEffect(() => {
    if (!renaming) setTitle(row.title);
  }, [row.title, renaming]);
  const names = conversationActionLabels(row);
  if (renaming) {
    return (
      <li className="th-cv-row">
        <form
          className="th-conv-edit"
          onSubmit={(e) => {
            e.preventDefault();
            const next = title.trim();
            if (next === "" || next === row.title) onRename(false);
            else onSaveTitle(next);
          }}
        >
          <input value={title} onChange={(e) => setTitle(e.target.value)} aria-label={t("Conversation title")} autoFocus onKeyDown={(e) => e.key === "Escape" && onRename(false)} />
          <WButton type="submit">{t("Save")}</WButton>
          <WButton onClick={() => onRename(false)}>{t("Cancel")}</WButton>
        </form>
      </li>
    );
  }
  if (confirming) {
    return (
      <li className="th-cv-row">
        <div className="th-conv-edit" role="group" aria-label={t("Delete this conversation?")}>
          <span>{t("Delete this conversation?")}</span>
          <WButton variant="destructive" disabled={answering} onClick={onDelete}>
            {t("Delete")}
          </WButton>
          <WButton onClick={() => onConfirm(false)}>{t("Cancel")}</WButton>
        </div>
      </li>
    );
  }
  return (
    <li className="th-cv-row">
      <button type="button" className="th-cv-open" data-conv-row={row.id} onClick={() => openConversation(row.id)}>
        <span className="th-cv-row-title">
          {row.starred ? <WIcon name="star" size={14} aria-label={t("Starred")} /> : null}
          <span>{row.title}</span>
        </span>
        {row.latestAnswerLine ? <span className="th-cv-row-line">{row.latestAnswerLine}</span> : null}
        <span className="th-cv-row-meta">
          {row.topic ? <span className="th-cv-topic"># {row.topic.name}</span> : null}
          <span>{updatedLabel(row.updatedAtUnixMs, now, locale)}</span>
        </span>
      </button>
      <span className="th-cv-actions-row">
        <WIconButton icon="star" label={names.star} aria-pressed={row.starred} onClick={onStar} />
        <WIconButton icon="pencil" label={names.rename} onClick={() => onRename(true)} />
        <WIconButton
          icon="close"
          label={names.delete}
          disabled={answering}
          title={answering ? t("An answer is being written in this conversation. Delete it when the answer is done.") : undefined}
          onClick={() => onConfirm(true)}
        />
      </span>
    </li>
  );
}
