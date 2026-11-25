// The conversations: new, rename, star, delete, as the Ask view has them, in a panel that can be hidden.

import { useEffect, useState } from "react";
import { chatDeleteConversation, chatRenameConversation, chatStarConversation } from "../../api";
import { describeError, isCommandError } from "../../errors";
import type { ConversationRow } from "../../generated/types";
import { t } from "../../l10n";
import { WButton, WIconButton } from "../../system";
import { conversationActionLabels } from "../../chat/chatModel";

const message = (e: unknown): string => (isCommandError(e) ? describeError(e) : String(e));

function Item({
  row,
  active,
  onOpen,
  onChange,
  onDeleted,
  onError,
  answering,
}: {
  row: ConversationRow;
  active: boolean;
  /** An answer is being written in this conversation: deleting it now would delete under the stream. */
  answering: boolean;
  onOpen: () => void;
  onChange: (rows: ConversationRow[]) => void;
  onDeleted: (rows: ConversationRow[]) => void;
  onError: (e: string | null) => void;
}) {
  const [editing, setEditing] = useState(false);
  const [title, setTitle] = useState(row.title);
  const [confirm, setConfirm] = useState(false);
  // A generated title can land while the row is shown.
  useEffect(() => {
    if (!editing) setTitle(row.title);
  }, [row.title, editing]);
  const act = (p: Promise<ConversationRow[]>, then: (rows: ConversationRow[]) => void) =>
    p.then(
      (rows) => {
        then(rows);
        onError(null);
      },
      (e: unknown) => onError(message(e)),
    );
  if (editing)
    return (
      <li className="th-conv">
        <form
          className="th-conv-edit"
          onSubmit={(e) => {
            e.preventDefault();
            act(chatRenameConversation(row.id, title), (rows) => {
              onChange(rows);
              setEditing(false);
            });
          }}
        >
          <input value={title} onChange={(e) => setTitle(e.target.value)} aria-label={t("Conversation title")} autoFocus />
          <WButton type="submit">{t("Save")}</WButton>
          <WButton onClick={() => setEditing(false)}>{t("Cancel")}</WButton>
        </form>
      </li>
    );
  if (confirm)
    return (
      <li className="th-conv">
        <div className="th-conv-edit" role="group" aria-label={t("Delete this conversation?")}>
          <span>{t("Delete this conversation?")}</span>
          <WButton variant="destructive" disabled={answering} onClick={() => act(chatDeleteConversation(row.id), onDeleted)}>
            {t("Delete")}
          </WButton>
          <WButton onClick={() => setConfirm(false)}>{t("Keep")}</WButton>
        </div>
      </li>
    );
  const names = conversationActionLabels(row);
  return (
    <li className={active ? "th-conv th-conv-active" : "th-conv"}>
      <button type="button" className="th-conv-title" onClick={onOpen} title={row.title} aria-current={active ? "true" : undefined}>
        {row.title}
      </button>
      <span className="th-conv-actions">
        <WIconButton icon="star" label={names.star} aria-pressed={row.starred} onClick={() => act(chatStarConversation(row.id, !row.starred), onChange)} />
        <WIconButton icon="pencil" label={names.rename} onClick={() => setEditing(true)} />
        <WIconButton
          icon="close"
          label={names.delete}
          disabled={answering}
          title={answering ? t("An answer is being written in this conversation. Delete it when the answer is done.") : undefined}
          onClick={() => setConfirm(true)}
        />
      </span>
    </li>
  );
}

export function ConversationPanel({
  rows,
  activeId,
  canStartNew,
  onNew,
  onOpen,
  onChange,
  onDeleted,
  onError,
  answeringId,
}: {
  rows: ConversationRow[];
  activeId: number | null;
  /** The conversation an answer is being written in, if it has an id yet. */
  answeringId: number | null;
  canStartNew: boolean;
  onNew: () => void;
  onOpen: (id: number) => void;
  onChange: (rows: ConversationRow[]) => void;
  onDeleted: (rows: ConversationRow[], id: number) => void;
  onError: (e: string | null) => void;
}) {
  return (
    <aside className="th-find-convs" aria-label={t("Conversations")}>
      <WButton variant="primary" onClick={onNew} disabled={!canStartNew}>
        {t("New conversation")}
      </WButton>
      {rows.length === 0 ? <p className="th-find-note">{t("Conversations you start here are listed.")}</p> : null}
      <ul className="th-convs">
        {rows.map((c) => (
          <Item key={c.id} row={c} active={c.id === activeId} onOpen={() => onOpen(c.id)} onChange={onChange} onDeleted={(next) => onDeleted(next, c.id)} onError={onError} answering={c.id === answeringId} />
        ))}
      </ul>
    </aside>
  );
}
