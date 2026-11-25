import type { SectionProps } from "../../types";
import { MemoryCard } from "./top/MemoryCard";
import { NeverCapturedCard } from "./top/NeverCapturedCard";
import { YourDataCard } from "./top/YourDataCard";

/** The top half of Memory & Privacy, in the spec's order: Memory, Never captured, Your data (counts, Obsidian export, the
 *  Local-only line, then Delete everything). The recording notice and reminders stay in Meetings, where they are today. */
export function MemoryTopCards({ onSection }: SectionProps) {
  return (
    <>
      <MemoryCard />
      <NeverCapturedCard />
      <YourDataCard onOpenAi={() => onSection("ai")} />
    </>
  );
}
