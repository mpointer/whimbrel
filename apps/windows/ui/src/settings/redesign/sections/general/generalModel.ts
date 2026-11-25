// What the redesigned General section says, apart from the components: the About values, the update row's state,
// and the skin card's colours. Each reads the same models the old General cards use.
import type { AppStatus, UpdateCheck } from "../../../../generated/types";
import { t } from "../../../../l10n";
import type { SkinPalette } from "../../../../skin";
import type { WSettingsValueItem } from "../../../../system";
import type { Mode } from "../../../../theme";
import { swatchColors } from "../../../skinGalleryModel";
import { isBusy, updateLine } from "../../../updateModel";

/** The About list: the same facts as the old About card. The database is "your library" in the redesign. */
export function aboutItems(status: Pick<AppStatus, "version" | "deviceId" | "dataDir" | "storeOpen" | "embeddingModelLoaded">): WSettingsValueItem[] {
  return [
    { key: "version", label: t("Version"), value: status.version },
    { key: "device", label: t("Device ID"), value: status.deviceId ?? t("not available"), mono: !!status.deviceId, copy: !!status.deviceId },
    { key: "data", label: t("Data folder"), value: status.dataDir, mono: true, copy: true },
    { key: "library", label: t("Your library"), value: status.storeOpen ? t("Open") : t("Not open") },
    { key: "search", label: t("Semantic search model"), value: status.embeddingModelLoaded ? t("Loaded") : t("Not downloaded yet") },
  ];
}

export interface UpdateRowState {
  /** The line under the row; none before the first check. */
  line: string | null;
  /** The check is still going: the button waits. */
  busy: boolean;
  /** An update is downloaded: Restart to Update is offered. */
  canRestart: boolean;
}

export function updateRowState(check: UpdateCheck, restarting: boolean): UpdateRowState {
  return { line: updateLine(check), busy: isBusy(check) || restarting, canRestart: check.state === "ready" };
}

/** One colour for a palette dot: the palette's main accent in the mode in force. */
export function dotColor(palette: SkinPalette, mode: Mode): string {
  const colors = swatchColors(palette, mode);
  return colors[2] ?? colors[0] ?? "transparent";
}

/** Said under the skin actions while a chat turn runs: Rename, Export, Delete and Import wait for it. */
export const actionsWaitNote = (): string => t("Rename, export, delete and import wait until the skin chat finishes.");
