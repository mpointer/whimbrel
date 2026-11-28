import { useCallback, useEffect, useState } from "react";
import {
  approvalApprove,
  approvalReject,
  approvalsList,
  routineModelOptions,
  routineRuns,
  routinesHost,
  routinesSetHost,
  routineSetAutoApprove,
  routineSetEnabled,
  routineSetModelPref,
  routineSetSchedule,
  routinesList,
  routinesCloud,
  routineSetCloud,
  routineSetReadyBy,
} from "../api";
import { describeError, isCommandError } from "../errors";
import type { ApprovalView, HostView, ModelOptionView, RoutineView, RunView } from "../generated/types";
import { useL10n } from "../l10n";
import { WButton, WLoadingState, WNoticeBox, WStatusChip, type WStatusTone } from "../system";
import {
  accountLine,
  actionDetail,
  actionHeadline,
  autoApproveNote,
  canApprove,
  canChooseHost,
  canToggle,
  description,
  fastHostPoll,
  HOST_FAST_POLL_MS,
  hasModelPicker,
  hasSchedule,
  hostAny,
  lastRunLine,
  modelPickerNote,
  modelPrefValue,
  needsAccountPicker,
  noApprovals,
  noOtherDevices,
  noRoutines,
  noRuns,
  outcomeLabel,
  routinesIntro,
  runHeading,
  scheduleChanged,
  unavailableNote,
} from "./automationsModel";
import {
  blockedReason,
  disclosure,
  infoFor,
  readyByLabel,
  readyByNote,
  routineFreshnessLabel,
  runnerMissing,
  showsCloudSection,
  showsToggle,
  shownReadyBy,
  statusNote,
  toggleCopy,
  toggleDisabled,
  toggleLabel,
  canBeLate,
  type CloudRoutineInfo,
  type CloudRoutinesView,
} from "./cloudRoutineModel";

function message(e: unknown): string {
  return isCommandError(e) ? describeError(e) : String(e);
}

/** How often the lists look again while the tab is open: the routines run in the background. */
const REFRESH_MS = 30_000;

/** The Automations tab (M3.4): what is waiting for approval, the routines, and what they have done. */
export function AutomationsView(props: { onWaiting?: (count: number) => void }) {
  const { onWaiting } = props;
  const { t, locale } = useL10n();
  const [routines, setRoutines] = useState<RoutineView[] | null>(null);
  const [runs, setRuns] = useState<RunView[]>([]);
  const [approvals, setApprovals] = useState<ApprovalView[]>([]);
  const [host, setHost] = useState<HostView | null>(null);
  const [modelOptions, setModelOptions] = useState<ModelOptionView[]>([]);
  // "Run in the cloud": null until read, and while it can't be (the toggles then stay hidden).
  const [cloud, setCloud] = useState<CloudRoutinesView | null>(null);
  // The runs the Updated/Late lines read: more than the history shows, as the Mac reads 200, so a busy day
  // does not push today's result out and show a false "Late".
  const [cloudRuns, setCloudRuns] = useState<RunView[]>([]);
  const [notice, setNotice] = useState<string | null>(null);
  const [working, setWorking] = useState<number | null>(null);
  const [open, setOpen] = useState<number | null>(null);
  const [error, setError] = useState<string | null>(null);

  const showApprovals = useCallback(
    (list: ApprovalView[]) => {
      setApprovals(list);
      onWaiting?.(list.length);
    },
    [onWaiting],
  );

  const refresh = useCallback(() => {
    Promise.all([routinesList(), routineRuns(), approvalsList(), routinesHost(), routineModelOptions()]).then(
      ([list, history, waiting, chosen, options]) => {
        setRoutines(list);
        setRuns(history);
        showApprovals(waiting);
        setHost(chosen);
        setModelOptions(options);
      },
      (e: unknown) => setError(message(e)),
    );
    routinesCloud().then(setCloud, () => {});
    routineRuns(200).then(setCloudRuns, () => {});
  }, [showApprovals]);

  useEffect(() => {
    refresh();
    const timer = setInterval(refresh, REFRESH_MS);
    return () => clearInterval(timer);
  }, [refresh]);

  // The device list can still be arriving when the page opens (after launch or sign-in), so the
  // picker re-checks every few seconds for its first minute until there is something to choose.
  const [openedAt] = useState(() => Date.now());
  useEffect(() => {
    if (!fastHostPoll(host, Date.now() - openedAt)) return;
    const timer = setTimeout(() => {
      routinesHost().then(setHost, () => {});
    }, HOST_FAST_POLL_MS);
    return () => clearTimeout(timer);
  }, [host, openedAt]);

  /** A change that returns the refreshed list; a refusal is shown and nothing changes. */
  const change = (request: Promise<RoutineView[]>) =>
    request.then(
      (list) => {
        setRoutines(list);
        setError(null);
      },
      (e: unknown) => setError(message(e)),
    );

  /** A cloud change that returns the refreshed cloud view; a refusal is shown and nothing changes. */
  const changeCloud = (request: Promise<CloudRoutinesView>) =>
    request.then(
      (view) => {
        setCloud(view);
        setError(null);
      },
      (e: unknown) => setError(message(e)),
    );

  const chooseHost = (id: string) =>
    routinesSetHost(id).then(
      (view) => {
        setHost(view);
        setError(null);
      },
      (e: unknown) => setError(message(e)),
    );

  const approve = (a: ApprovalView, account: string | undefined) => {
    setWorking(a.runId);
    setNotice(null);
    approvalApprove(a.runId, account).then(
      (result) => {
        showApprovals(result.approvals);
        setNotice(result.message ?? null);
        setError(null);
        setWorking(null);
        refresh();
      },
      (e: unknown) => {
        setError(message(e));
        setWorking(null);
      },
    );
  };

  const reject = (a: ApprovalView) => {
    setNotice(null);
    approvalReject(a.runId).then(
      (list) => {
        showApprovals(list);
        setError(null);
      },
      (e: unknown) => setError(message(e)),
    );
  };

  const alertLine = (text: string) => <WNoticeBox tone="error">{text}</WNoticeBox>;
  return (
    <div className="stack rt">
      <section className="card about" aria-labelledby="approvals-heading">
        <h2 id="approvals-heading" className="section-label">
          {t("Needs your approval")}
        </h2>
        {approvals.length === 0 ? (
          <p className="muted small">{noApprovals()}</p>
        ) : (
          <ul className="approval-list">
            {approvals.map((a) => (
              <ApprovalRow key={a.runId} approval={a} busy={working === a.runId} onApprove={(account) => approve(a, account)} onReject={() => reject(a)} />
            ))}
          </ul>
        )}
        {notice && <WNoticeBox tone="info" live>{notice}</WNoticeBox>}
      </section>

      <section className="card about" aria-labelledby="routines-heading">
        <h2 id="routines-heading" className="section-label">
          {t("Routines")}
        </h2>
        <p className="muted small">
          {routinesIntro(host)}
        </p>
        {host && (
          <div className="stack-sm">
            <label className="field">
              <span>{t("Routines run on")}</span>
              <select className="input" value={host.selected} disabled={!canChooseHost(host)} onChange={(e) => void chooseHost(e.target.value)}>
                <option value="">{hostAny()}</option>
                {host.options.map((o) => (
                  <option key={o.id} value={o.id}>
                    {o.label}
                  </option>
                ))}
              </select>
            </label>
            {host.caption ? <p className="muted small">{host.caption}</p> : !canChooseHost(host) && <p className="muted small">{noOtherDevices()}</p>}
          </div>
        )}
        {routines === null ? (
          error ? null : <WLoadingState title={t("Loading…")} />
        ) : routines.length === 0 ? (
          <p className="muted small">{noRoutines()}</p>
        ) : (
          <ul className="routine-list">
            {routines.map((r) => (
              <RoutineRow
                key={r.id}
                routine={r}
                modelOptions={modelOptions}
                cloud={cloud}
                runs={cloudRuns}
                onCloud={(template, on) => void changeCloud(routineSetCloud(template, on))}
                onReadyBy={(template, readyBy) => void changeCloud(routineSetReadyBy(template, readyBy))}
                onToggle={(enabled) => void change(routineSetEnabled(r.id, enabled))}
                onSchedule={(schedule) => void change(routineSetSchedule(r.id, schedule))}
                onAutoApprove={(enabled) => void change(routineSetAutoApprove(r.id, enabled))}
                onModelPref={(pref) => void change(routineSetModelPref(r.id, pref))}
              />
            ))}
          </ul>
        )}
        {error && alertLine(error)}
      </section>

      <section className="card about" aria-labelledby="history-heading">
        <h2 id="history-heading" className="section-label">
          {t("History")}
        </h2>
        <p className="muted small">{t("What your routines produced, most recent first.")}</p>
        {runs.length === 0 ? (
          <p className="muted small">{noRuns()}</p>
        ) : (
          <ul className="run-list">
            {runs.map((run) => (
              <li key={run.id} className="run-item">
                <button className="run-head" aria-expanded={open === run.id} onClick={() => setOpen(open === run.id ? null : run.id)}>
                  <span>{runHeading(run, locale)}</span>
                  <span className={`run-outcome run-outcome--${run.outcome}`}>
                    <WStatusChip tone={outcomeTone(run.outcome)}>{outcomeLabel(run.outcome)}</WStatusChip>
                  </span>
                </button>
                <p className={open === run.id ? "run-summary" : "run-summary run-summary--clamped"}>{run.summary}</p>
              </li>
            ))}
          </ul>
        )}
      </section>
    </div>
  );
}

/** The tone of a run's outcome chip: the glyph and the words carry it, the color only repeats it. */
function outcomeTone(outcome: string): WStatusTone {
  return outcome === "success" ? "good" : outcome === "failed" ? "problem" : outcome === "pending" ? "warning" : "neutral";
}

function ApprovalRow(props: { approval: ApprovalView; busy: boolean; onApprove: (account: string | undefined) => void; onReject: () => void }) {
  const { approval } = props;
  const { t, locale } = useL10n();
  const [account, setAccount] = useState(approval.account ?? "");
  useEffect(() => setAccount(approval.account ?? ""), [approval.account]);
  const line = accountLine(approval);

  return (
    <li className="approval-row">
      <div className="approval-main">
        <p className="muted small">{approval.routineName}</p>
        {approval.action ? (
          <>
            <p>
              <strong>{actionHeadline(approval.action)}</strong>
            </p>
            <p className="approval-detail">{actionDetail(approval.action, locale)}</p>
          </>
        ) : (
          <p>{approval.summary}</p>
        )}
        {needsAccountPicker(approval) ? (
          <label className="field">
            <span>{t("Use account")}</span>
            <select className="input" value={account} onChange={(e) => setAccount(e.target.value)}>
              {approval.accounts.map((email) => (
                <option key={email} value={email}>
                  {email}
                </option>
              ))}
            </select>
          </label>
        ) : (
          line && <p className="muted small">{line}</p>
        )}
      </div>
      <div className="approval-actions">
        <WButton variant="primary" disabled={!canApprove(approval) || props.busy} onClick={() => props.onApprove(account || undefined)}>
          {props.busy ? t("Working…") : t("Approve")}
        </WButton>
        <WButton disabled={props.busy} onClick={props.onReject}>
          {t("Reject")}
        </WButton>
      </div>
    </li>
  );
}

function RoutineRow(props: {
  routine: RoutineView;
  modelOptions: ModelOptionView[];
  cloud: CloudRoutinesView | null;
  runs: RunView[];
  onCloud: (template: string, on: boolean) => void;
  onReadyBy: (template: string, readyBy: string) => void;
  onToggle: (enabled: boolean) => void;
  onSchedule: (schedule: string) => void;
  onAutoApprove: (enabled: boolean) => void;
  onModelPref: (pref: string) => void;
}) {
  const { routine, modelOptions } = props;
  const { t, locale } = useL10n();
  const [time, setTime] = useState(routine.schedule);
  useEffect(() => setTime(routine.schedule), [routine.schedule]);
  const note = unavailableNote(routine);
  const info = infoFor(props.cloud, routine.kind);

  return (
    <li className="routine-row">
      <div className="routine-main">
        <label className="toggle">
          <input type="checkbox" checked={routine.enabled} disabled={!canToggle(routine)} onChange={(e) => props.onToggle(e.target.checked)} />
          <strong>{routine.name}</strong>
        </label>
        <p className="muted small">{description(routine.kind)}</p>
        {note && <p className="small routine-unavailable">{note}</p>}
        {routine.writes && note === null && (
          <>
            <label className="toggle">
              <input type="checkbox" checked={routine.autoApprove} onChange={(e) => props.onAutoApprove(e.target.checked)} />
              {t("Approve automatically")}
            </label>
            <p className="muted small">{autoApproveNote(routine.kind)}</p>
          </>
        )}
        <p className="muted small">{lastRunLine(routine, locale)}</p>
        {hasModelPicker(routine) && modelOptions.length > 0 && (
          <label className="field">
            <span>{t("Model")}</span>
            <select
              className="input"
              aria-label={t("{name} model", { name: routine.name })}
              value={modelPrefValue(routine, modelOptions)}
              onChange={(e) => props.onModelPref(e.target.value)}
            >
              {modelOptions.map((o) => (
                <option key={o.value} value={o.value}>
                  {o.label}
                </option>
              ))}
            </select>
            <span className="muted small">{modelPickerNote()}</span>
          </label>
        )}
        {props.cloud && info && showsToggle(info) && showsCloudSection(props.cloud, info.cloud) && (
          <CloudSection routine={routine} info={info} view={props.cloud} runs={props.runs} onCloud={props.onCloud} onReadyBy={props.onReadyBy} />
        )}
      </div>
      {hasSchedule(routine) && (
        <label className="field routine-time">
          <span>{t("Time")}</span>
          <input
            className="input"
            type="time"
            value={time}
            onChange={(e) => setTime(e.target.value)}
            onBlur={() => scheduleChanged(routine.schedule, time) && props.onSchedule(time)}
          />
        </label>
      )}
    </li>
  );
}

/** "Run in the cloud" for one routine core lets run there: the toggle with its grant and limits, the ready-by time,
 *  and how fresh the last result is. */
function CloudSection(props: {
  routine: RoutineView;
  info: CloudRoutineInfo;
  view: CloudRoutinesView;
  runs: RunView[];
  onCloud: (template: string, on: boolean) => void;
  onReadyBy: (template: string, readyBy: string) => void;
}) {
  const { routine, info, view } = props;
  const { t, locale } = useL10n();
  const on = info.cloud;
  const blocked = blockedReason(view.sync_ready, routine.enabled, view.local_only, view.has_synced_key);
  const note = statusNote(on, blocked, runnerMissing(view.devices), info.template);
  const readyBy = shownReadyBy(routine.schedule, info.ready_by);
  const [time, setTime] = useState(readyBy);
  useEffect(() => setTime(readyBy), [readyBy]);
  const now = new Date();
  const fresh = on ? routineFreshnessLabel(routine, info, props.runs, now, locale) : null;

  return (
    <div className="stack-sm">
      <label className="toggle">
        <input type="checkbox" checked={on} disabled={toggleDisabled(on, blocked)} onChange={(e) => props.onCloud(info.template, e.target.checked)} />
        {toggleLabel()}
      </label>
      <p className="muted small">{toggleCopy(info)}</p>
      <p className="muted small">{disclosure()}</p>
      {note && <p className="small routine-unavailable">{note}</p>}
      {on && canBeLate(info.template, true) && (
        <label className="field routine-time">
          <span>{readyByLabel()}</span>
          <input
            className="input"
            type="time"
            aria-label={t("{name} ready by", { name: routine.name })}
            value={time}
            onChange={(e) => setTime(e.target.value)}
            onBlur={() => scheduleChanged(readyBy, time) && props.onReadyBy(info.template, time)}
          />
          <span className="muted small">{readyByNote()}</span>
        </label>
      )}
      {fresh && <p className="muted small">{fresh}</p>}
    </div>
  );
}
