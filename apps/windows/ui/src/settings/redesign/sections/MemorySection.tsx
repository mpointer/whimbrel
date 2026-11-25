import type { SectionProps } from "../types";
import { MemoryBottomCards } from "./memory/MemoryBottomCards";
import { MemoryTopCards } from "./memory/MemoryTopCards";

/** Memory & Privacy, in the spec's order: Memory, Never captured, (recording notice and reminders stay where they are
 *  today, in Meetings), Your data (counts, export, the Local-only line, delete everything), Keeping data, Library
 *  encryption, Hotkey panel. The two halves are drawn in ./memory so each can be tested on its own. */
export function MemorySection(props: SectionProps) {
  return (
    <>
      <MemoryTopCards {...props} />
      <MemoryBottomCards {...props} />
    </>
  );
}
