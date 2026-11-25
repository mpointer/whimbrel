// On a topic's page (redesign flag only; screen 6, parts 4 and 5): Ask within this topic, and the topic's conversations, newest first. A question
// asked here opens a new conversation about the topic's members only, attached to the topic from the start. The topic is also what the shell's
// Ask control asks about while this page shows.

import { useEffect, useState } from "react";
import { topicConversationsList } from "../api";
import type { ConversationRow } from "../generated/types";
import { t, useL10n } from "../l10n";
import { WButton, WIcon } from "../system";
import { updatedLabel } from "./model";
import { openConversation, setTopicShown, startConversation, useConversationNav } from "./nav";
import "./conversations.css";

export function TopicConversations({ topicId, name }: { topicId: number; name: string }) {
  const { locale } = useL10n();
  const { version } = useConversationNav();
  const [rows, setRows] = useState<ConversationRow[] | null>(null);
  const [question, setQuestion] = useState("");
  useEffect(() => {
    setTopicShown({ id: topicId, name });
    return () => setTopicShown(null);
  }, [topicId, name]);
  useEffect(() => {
    let live = true;
    topicConversationsList(topicId).then(
      (r) => live && setRows(r),
      () => live && setRows([]),
    );
    return () => {
      live = false;
    };
  }, [topicId, version]);
  const ask = () => {
    const words = question.trim();
    if (!words) return;
    setQuestion("");
    startConversation({ kind: "topic", id: topicId }, words, name);
  };
  const now = Date.now();
  return (
    <section className="field th-cv-topic-section" aria-label={t("Conversations")}>
      <h3 className="section-label">{t("Conversations")}</h3>
      <form
        className="th-cv-composer th-cv-topic-ask"
        onSubmit={(e) => {
          e.preventDefault();
          ask();
        }}
      >
        <input type="text" value={question} onChange={(e) => setQuestion(e.target.value)} aria-label={t("Ask about {name}", { name })} placeholder={t("Ask about {name}", { name })} />
        <WButton type="submit" variant="primary" disabled={question.trim() === ""}>
          {t("Ask")}
        </WButton>
      </form>
      {rows && rows.length > 0 ? (
        <ul className="th-cv-rows" aria-label={t("Conversations about {name}", { name })}>
          {rows.map((row) => (
            <li key={row.id} className="th-cv-row">
              <button type="button" className="th-cv-open" onClick={() => openConversation(row.id)}>
                <span className="th-cv-row-title">
                  {row.starred ? <WIcon name="star" size={14} aria-label={t("Starred")} /> : null}
                  <span>{row.title}</span>
                </span>
                <span className="th-cv-row-meta">{updatedLabel(row.updatedAtUnixMs, now, locale)}</span>
              </button>
            </li>
          ))}
        </ul>
      ) : rows ? (
        <p className="muted small">{t("Nothing yet.")}</p>
      ) : null}
    </section>
  );
}
