import { useEffect, useState } from "react";
import { emit } from "@tauri-apps/api/event";
import { topicLookup, topicsNamedIn } from "../api";
import { useL10n } from "../l10n";
import type { TopicLookup, TopicRefView } from "../generated/types";

/** The event the app shell listens for to open the Topics tab on a topic. */
export const OPEN_TOPIC_EVENT = "open-topic";

/** Opens the Topics tab on `topicId`, from anywhere in the main window. */
export const openTopic = (topicId: number) => void emit(OPEN_TOPIC_EVENT, topicId);

/** A topic's name as a chip that opens its page (the Mac's `TopicChip`). */
export function TopicChip({ topic }: { topic: TopicRefView }) {
  return (
    <button type="button" className="chip citation topic-chip" onClick={() => openTopic(topic.topicId)}>
      {topic.name}
    </button>
  );
}

/** Each item's topic, looked up in one call whenever the items change (not on every render). */
export function useTopicLookup(items: TopicLookup[]): (TopicRefView | null)[] {
  const key = JSON.stringify(items);
  const [found, setFound] = useState<{ key: string; topics: (TopicRefView | null)[] }>({ key: "", topics: [] });
  useEffect(() => {
    const wanted: TopicLookup[] = JSON.parse(key);
    if (wanted.length === 0) return;
    let live = true;
    topicLookup(wanted).then(
      (topics) => {
        // No answer (an older shell, the test bridge) reads as no topics, never as a crash.
        if (live) setFound({ key, topics: Array.isArray(topics) ? topics : [] });
      },
      () => {},
    );
    return () => {
      live = false;
    };
  }, [key]);
  return found.key === key ? found.topics : [];
}

/** "Open topic page: Acme" rows for the topics `text` names: search and Ask offer the page first. */
export function TopicPageOffer({ text }: { text: string }) {
  const { t } = useL10n();
  const [named, setNamed] = useState<{ text: string; topics: TopicRefView[] }>({ text: "", topics: [] });
  useEffect(() => {
    if (!text.trim()) return;
    let live = true;
    topicsNamedIn(text).then(
      (topics) => {
        if (live) setNamed({ text, topics: Array.isArray(topics) ? topics : [] });
      },
      () => {},
    );
    return () => {
      live = false;
    };
  }, [text]);
  const topics = named.text === text ? named.topics : [];
  if (topics.length === 0) return null;
  return (
    <div className="toolbar topic-offer">
      {topics.map((topic) => (
        <button key={topic.topicId} type="button" className="mode" onClick={() => openTopic(topic.topicId)}>
          {t("Open topic page: {name}", { name: topic.name })}
        </button>
      ))}
    </div>
  );
}
