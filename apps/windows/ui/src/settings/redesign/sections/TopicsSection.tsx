import { TopicNamesCard } from "./topics/TopicNamesCard";
import { TopicsListCard } from "./topics/TopicsListCard";
import "./topics/topicsEmpty.css";
import type { SectionProps } from "../types";

/** Topics: the topics the person named and the ones suggested, then the names they go by. */
export function TopicsSection(_props: SectionProps) {
  return (
    <>
      <TopicsListCard />
      <TopicNamesCard />
    </>
  );
}
