// What the redesigned Status section says. The lines come from the same builders the old Status page uses
// (../../statusModel, ../../planModel); this file builds its own fixed sentences in the redesign wording ("Keep
// forever" for no time limit) and turns each area into one row that opens where it is changed. Lines that come from
// elsewhere (a routine's name, a server's message) are shown as they are, never rewritten.
import type { AiSettings, DeviceView, HostView, RetentionSettings, RunView, StatusView, SyncView, UpdateCheck, WhimbrelAiView } from "../../../generated/types";
import { t } from "../../../l10n";
import { planStatusLines } from "../../planModel";
import type { SettingsSectionId } from "../../settingsSections";
import { aiLines, diskSize, routineLines, syncLines, updateLines } from "../../statusModel";

/** Where a Status row opens: a Settings section, or the Automations tab (routines live there on Windows). */
export type StatusTarget = SettingsSectionId | "automations";

export interface StatusRowSpec {
  key: "sync" | "plan" | "ai" | "storage" | "updates" | "routines";
  title: string;
  lines: string[];
  target: StatusTarget;
}

export interface StatusInput {
  sync: SyncView | null;
  devices: Pick<DeviceView, "status">[];
  status: StatusView | null;
  ai: AiSettings | null;
  retention: RetentionSettings | null;
  version: string | null;
  ready: string | null;
  check: UpdateCheck | null;
  host: HostView | null;
  lastRun: RunView | null;
  whimbrel: WhimbrelAiView | null;
  now: number;
  locale: string;
}

/** The storage lines in the redesign's words: the same audio and image sizes, and retention under its row name. */
export function storageRedesignLines(status: Pick<StatusView, "audioBytes" | "imageBytes"> | null, retention: Pick<RetentionSettings, "meetingDays"> | null): string[] {
  const lines: string[] = [];
  if (status) lines.push(t("Audio: {audio}. Images: {images}.", { audio: diskSize(status.audioBytes), images: diskSize(status.imageBytes) }));
  if (retention) {
    lines.push(
      retention.meetingDays == null
        ? t("Meetings and recordings: Keep forever.")
        : t("Meetings and recordings are kept for {count} days.", { count: retention.meetingDays }),
    );
  }
  return lines;
}

/** One description from a row's lines: each line ends like a sentence, so they read as one paragraph. */
export function joinLines(lines: readonly string[]): string | undefined {
  const text = lines
    .map((l) => l.trim())
    .filter(Boolean)
    .map((l) => (/[.!?…]$/.test(l) ? l : `${l}.`))
    .join(" ");
  return text || undefined;
}

/** The rows in the old page's order. The Plan row is left out while there is nothing to say about the plan. */
export function statusRows(i: StatusInput): StatusRowSpec[] {
  const plan = planStatusLines(i.sync, i.whimbrel, i.locale);
  const rows: StatusRowSpec[] = [
    { key: "sync", title: t("Sync"), lines: syncLines(i.sync, i.devices, i.status, i.now, i.locale), target: "devices" },
  ];
  if (plan.length > 0) rows.push({ key: "plan", title: t("Plan"), lines: plan, target: "plan" });
  rows.push(
    { key: "ai", title: t("AI"), lines: aiLines(i.ai), target: "ai" },
    { key: "storage", title: t("Storage"), lines: storageRedesignLines(i.status, i.retention), target: "capture" },
    { key: "updates", title: t("Updates"), lines: updateLines(i.version, i.ready, i.check), target: "general" },
    { key: "routines", title: t("Routines"), lines: routineLines(i.host, i.lastRun, i.now, i.locale), target: "automations" },
  );
  return rows;
}
