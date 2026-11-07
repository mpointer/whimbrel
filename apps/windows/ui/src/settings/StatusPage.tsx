import { useEffect, useState } from "react";
import { aiSettings, appStatus, retentionSettings, routineRuns, routinesHost, statusOverview, syncDevices, syncStatus, updateCheckStatus, updateReady } from "../api";
import type { AiSettings, DeviceView, HostView, RetentionSettings, RunView, StatusView, SyncView, UpdateCheck } from "../generated/types";
import { t, useL10n } from "../l10n";
import type { SettingsSectionId } from "./settingsSections";
import { aiLines, routineLines, storageLines, syncLines, updateLines } from "./statusModel";

/** How often the page looks again while it is open; sync and updates change in the background. */
const REFRESH_MS = 15_000;

/** Settings → Status (issue 595): one row per area, each opening where it is changed. Read-only. */
export function StatusPage(props: { onSection: (id: SettingsSectionId) => void; onOpenAutomations: () => void }) {
  const { onSection, onOpenAutomations } = props;
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
  const [now, setNow] = useState(() => Date.now());

  useEffect(() => {
    // Each part fills in on its own: one that fails (no store, sync not set up) leaves its row short, not the page empty.
    const load = () => {
      setNow(Date.now());
      syncStatus().then(setSync, () => {});
      syncDevices().then(setDevices, () => {});
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

  return (
    <div className="stack status-page">
      <StatusRow title={t("Sync")} lines={syncLines(sync, devices, status, now, locale)} onOpen={() => onSection("devices")} />
      <StatusRow title={t("AI")} lines={aiLines(ai)} onOpen={() => onSection("ai")} />
      <StatusRow title={t("Storage")} lines={storageLines(status, retention)} onOpen={() => onSection("capture")} />
      <StatusRow title={t("Updates")} lines={updateLines(version, ready, check)} onOpen={() => onSection("general")} />
      <StatusRow title={t("Routines")} lines={routineLines(host, lastRun, now, locale)} onOpen={onOpenAutomations} />
    </div>
  );
}

function StatusRow(props: { title: string; lines: string[]; onOpen: () => void }) {
  return (
    <button type="button" className="card status-row" onClick={props.onOpen}>
      <span className="status-row-text">
        <strong className="status-row-title">{props.title}</strong>
        {props.lines.map((line, i) => (
          <span key={i} className="muted small">
            {line}
          </span>
        ))}
      </span>
      <span className="status-row-chevron" aria-hidden="true">
        ›
      </span>
    </button>
  );
}
