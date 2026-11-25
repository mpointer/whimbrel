// Settings → Status, drawn on the design system's parts. The same data and the same 15 second refresh as the old
// page (settings/StatusPage.tsx); each row is a link that opens where the setting is changed.
import { useEffect, useState } from "react";
import { aiSettings, appStatus, retentionSettings, routineRuns, routinesHost, statusOverview, syncDevices, syncStatus, updateCheckStatus, updateReady, whimbrelAiStatus } from "../../../api";
import type { AiSettings, DeviceView, HostView, RetentionSettings, RunView, StatusView, SyncView, UpdateCheck, WhimbrelAiView } from "../../../generated/types";
import { t, useL10n } from "../../../l10n";
import { WSettingsCard, WSettingsLinkRow } from "../../../system";
import type { SectionProps } from "../types";
import { joinLines, statusRows, type StatusRowSpec } from "./StatusSectionModel";

/** How often the page looks again while it is open; sync and updates change in the background. */
const REFRESH_MS = 15_000;

/** The rows, drawn from lines already built. No state, so a test can call it and read what it returns. */
export function StatusRows(props: { rows: readonly StatusRowSpec[] } & Pick<SectionProps, "onSection" | "onOpenAutomations">) {
  const { rows, onSection, onOpenAutomations } = props;
  return (
    <WSettingsCard title={t("Overview")} headingLevel={3} description={t("Each row opens the place where it is changed.")}>
      {rows.map((row) => (
        <WSettingsLinkRow
          key={row.key}
          kind="section"
          label={row.title}
          description={joinLines(row.lines)}
          onPress={() => (row.target === "automations" ? onOpenAutomations() : onSection(row.target))}
        />
      ))}
    </WSettingsCard>
  );
}

export function StatusSection({ onSection, onOpenAutomations }: SectionProps) {
  const { locale } = useL10n();
  const [sync, setSync] = useState<SyncView | null>(null);
  const [devices, setDevices] = useState<DeviceView[]>([]);
  const [status, setStatus] = useState<StatusView | null>(null);
  const [ai, setAi] = useState<AiSettings | null>(null);
  const [retention, setRetention] = useState<RetentionSettings | null>(null);
  const [version, setVersion] = useState<string | null>(null);
  const [ready, setReady] = useState<string | null>(null);
  const [check, setCheck] = useState<UpdateCheck | null>(null);
  const [host, setHost] = useState<HostView | null>(null);
  const [lastRun, setLastRun] = useState<RunView | null>(null);
  const [whimbrel, setWhimbrel] = useState<WhimbrelAiView | null>(null);
  const [now, setNow] = useState(() => Date.now());

  useEffect(() => {
    // Each part fills in on its own: one that fails (no store, sync not set up) leaves its row short, not the page empty.
    const load = () => {
      setNow(Date.now());
      syncStatus().then(setSync, () => {});
      syncDevices().then(setDevices, () => {});
      whimbrelAiStatus().then(setWhimbrel, () => {});
      aiSettings().then(setAi, () => {});
      retentionSettings().then((r) => setRetention(r.settings), () => {});
      appStatus().then((s) => setVersion(s.version), () => {});
      updateReady().then(setReady, () => {});
      updateCheckStatus().then(setCheck, () => {});
      routinesHost().then(setHost, () => {});
      routineRuns(1).then((runs) => setLastRun(runs[0] ?? null), () => {});
    };
    load();
    // The backlog and media sizes look at every meeting's files, so they load once per visit, as on the Mac.
    statusOverview().then(setStatus, () => {});
    const timer = setInterval(load, REFRESH_MS);
    return () => clearInterval(timer);
  }, []);

  const rows = statusRows({ sync, devices, status, ai, retention, version, ready, check, host, lastRun, whimbrel, now, locale });
  return <StatusRows rows={rows} onSection={onSection} onOpenAutomations={onOpenAutomations} />;
}
