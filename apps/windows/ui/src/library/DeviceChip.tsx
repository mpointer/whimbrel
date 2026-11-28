import type { DeviceTag } from "../generated/types";

/** Which device a library row, meeting or find result came from; the one on this PC has its own look. */
export function DeviceChip({ device }: { device: DeviceTag }) {
  return (
    <span className={device.thisDevice ? "chip chip--this" : "chip"} title={device.deviceId}>
      {device.label}
    </span>
  );
}
