import type { SectionProps } from "../../types";
import { FilesKeptAsideCard } from "./bottom/FilesKeptAsideCard";
import { HotkeyPanelCard } from "./bottom/HotkeyPanelCard";
import { LibraryEncryptionCard } from "./bottom/LibraryEncryptionCard";
import { RedesignedStorageCard } from "./bottom/RedesignedStorageCard";
import { RetentionKeepingCard } from "./bottom/RetentionKeepingCard";

/** The lower half of Memory & Privacy: Keeping data, Storage, Library encryption, Files kept aside (only while there
 *  are files), Hotkey panel. The retention card tells the frame when a period was saved. */
export function MemoryBottomCards({ onRetentionSaved }: SectionProps) {
  return (
    <>
      <RetentionKeepingCard onSaved={onRetentionSaved} />
      <RedesignedStorageCard />
      <LibraryEncryptionCard />
      <FilesKeptAsideCard />
      <HotkeyPanelCard />
    </>
  );
}
