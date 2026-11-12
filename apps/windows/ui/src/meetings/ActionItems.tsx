import { useState } from "react";
import { meetingAddActionItem, meetingDeleteActionItem, meetingReorderActionItems, meetingUpdateActionItem } from "../api";
import { describeError, isCommandError } from "../errors";
import type { ActionItemView } from "../generated/types";
import { useL10n } from "../l10n";
import { displayOrder, movedItem } from "./workspaceModel";

function message(e: unknown): string {
  return isCommandError(e) ? describeError(e) : String(e);
}

/** The meeting's action items: the summary's, plus whatever the user adds. */
export function ActionItems({
  meetingId,
  items,
  onItems,
  readOnly,
}: {
  meetingId: number;
  items: ActionItemView[];
  onItems: (items: ActionItemView[]) => void;
  readOnly: boolean;
}) {
  const [text, setText] = useState("");
  const [error, setError] = useState<string | null>(null);
  const { t } = useL10n();

  const run = (call: Promise<ActionItemView[]>) => {
    setError(null);
    call.then(onItems).catch((e: unknown) => setError(message(e)));
  };

  const move = (id: number, direction: -1 | 1) => {
    const next = movedItem(items, id, direction);
    if (next === items) return;
    onItems(next);
    run(meetingReorderActionItems(meetingId, next.map((i) => i.id)));
  };

  const add = (e: React.FormEvent) => {
    e.preventDefault();
    if (!text.trim()) return;
    run(meetingAddActionItem(meetingId, text));
    setText("");
  };

  return (
    <div className="field">
      <span>{t("Action items")}</span>
      {items.length === 0 ? (
        <p className="muted small">{t("None yet.")}</p>
      ) : (
        <ul className="action-items">
          {displayOrder(items).map((item) => (
            <li key={item.id}>
              <label className="toggle" title={readOnly ? t("Recorded on another device, so it can't be checked off here.") : undefined}>
                <input
                  type="checkbox"
                  checked={item.completed}
                  disabled={readOnly}
                  title={readOnly ? t("Recorded on another device, so it can't be checked off here.") : undefined}
                  onChange={() => run(meetingUpdateActionItem(meetingId, item.id, null, !item.completed))}
                />
                {/* A textarea, so a long item wraps and shows in full; Enter saves it as the old one-line field did. */}
                <textarea
                  className="input action-item-text"
                  rows={1}
                  value={item.text}
                  readOnly={readOnly}
                  onChange={(e) => onItems(items.map((i) => (i.id === item.id ? { ...i, text: e.target.value.replace(/\n/g, " ") } : i)))}
                  onKeyDown={(e) => {
                    if (e.key === "Enter") {
                      e.preventDefault();
                      e.currentTarget.blur();
                    }
                  }}
                  onBlur={(e) => run(meetingUpdateActionItem(meetingId, item.id, e.target.value, null))}
                />
              </label>
              {!readOnly && (
                <span className="action-item-buttons">
                  <button className="mode" aria-label={t("Move up")} onClick={() => move(item.id, -1)}>
                    ↑
                  </button>
                  <button className="mode" aria-label={t("Move down")} onClick={() => move(item.id, 1)}>
                    ↓
                  </button>
                  <button className="mode" aria-label={t("Delete")} onClick={() => run(meetingDeleteActionItem(meetingId, item.id))}>
                    ✕
                  </button>
                </span>
              )}
            </li>
          ))}
        </ul>
      )}
      {!readOnly && (
        <form className="field-row" onSubmit={add}>
          <input className="input" placeholder={t("Add an action item")} value={text} onChange={(e) => setText(e.target.value)} />
          <button className="mode" type="submit" disabled={!text.trim()}>
            {t("Add")}
          </button>
        </form>
      )}
      {error && <p role="alert">{error}</p>}
    </div>
  );
}
