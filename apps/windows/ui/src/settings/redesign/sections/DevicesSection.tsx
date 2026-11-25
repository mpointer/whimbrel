import type { SectionProps } from "../types";
import { DevicesListCard } from "./devices/DevicesListCard";
import { PhoneCard } from "./devices/PhoneCard";
import { SyncStatusCard } from "./devices/SyncStatusCard";
import { useSyncStatus } from "./devices/useSyncStatus";

/** Sync & Devices: sync (turn on, approve, the way past the device limit), the devices on the account, and the phone beta. Same commands as the old Sync card. */
export function DevicesSection(_props: SectionProps) {
  const sync = useSyncStatus();
  const ready = sync.view?.state === "ready";
  return (
    <>
      <SyncStatusCard sync={sync} />
      {ready && <DevicesListCard />}
      <PhoneCard ready={ready} />
    </>
  );
}
