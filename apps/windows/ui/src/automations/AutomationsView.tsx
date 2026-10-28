import { useCallback, useEffect, useState } from "react";
import {
  approvalApprove,
  approvalReject,
  approvalsList,
  routineRuns,
  routinesHost,
  routinesSetHost,
  routineSetAutoApprove,
  routineSetEnabled,
  routineSetSchedule,
  routinesList,
} from "../api";
import { describeError, isCommandError } from "../errors";
import type { ApprovalView, HostView, RoutineView, RunView } from "../generated/types";
import {
  DESCRIPTIONS,
  HOST_ANY,
  NO_APPROVALS,
  NO_OTHER_DEVICES,
  NO_ROUTINES,
  NO_RUNS,
  accountLine,
  actionDetail,
  actionHeadline,
  autoApproveNote,
  canApprove,
  canChooseHost,
  canToggle,
  hasSchedule,
  lastRunLine,
  needsAccountPicker,
  outcomeLabel,
  runHeading,
  scheduleChanged,
  unavailableNote,
} from "./automationsModel";

function message(e: unknown): string {
  return isCommandError(e) ? describeError(e) : String(e);
}

/** How often the lists look again while the tab is open: the routines run in the background. */
const REFRESH_MS = 30_000;

/** The Automations tab (M3.4): what is waiting for approval, the routines, and what they have done. */
export function AutomationsView(props: { onWaiting?: (count: number) => void }) {
  const { onWaiting } = props;
  const [routines, setRoutines] = useState<RoutineView[] | null>(null);
  const [runs, setRuns] = useState<RunView[]>([]);
  const [approvals, setApprovals] = useState<ApprovalView[]>([]);
  const [host, setHost] = useState<HostView | null>(null);
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
    Promise.all([routinesList(), routineRuns(), approvalsList(), routinesHost()]).then(
      ([list, history, waiting, chosen]) => {
        setRoutines(list);
        setRuns(history);
        showApprovals(waiting);
        setHost(chosen);
      },
      (e: unknown) => setError(message(e)),
    );
  }, [showApprovals]);

  useEffect(() => {
    refresh();
    const timer = setInterval(refresh, REFRESH_MS);
    return () => clearInterval(timer);
  }, [refresh]);

  /** A change that returns the refreshed list; a refusal is shown and nothing changes. */
  const change = (request: Promise<RoutineView[]>) =>
    request.then(
      (list) => {
        setRoutines(list);
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

  return (
    <div className="stack">
      <section className="card about" aria-labelledby="approvals-heading">
        <h2 id="approvals-heading" className="section-label">
          Needs your approval
        </h2>
        {approvals.length === 0 ? (
          <p className="muted small">{NO_APPROVALS}</p>
        ) : (
          <ul className="approval-list">
            {approvals.map((a) => (
              <ApprovalRow key={a.runId} approval={a} busy={working === a.runId} onApprove={(account) => approve(a, account)} onReject={() => reject(a)} />
            ))}
          </ul>
        )}
        {notice && <p role="status">{notice}</p>}
      </section>

      <section className="card about" aria-labelledby="routines-heading">
        <h2 id="routines-heading" className="section-label">
          Routines
        </h2>
        <p className="muted small">
          Routines run on this PC while Whimbrel is running. A routine that would send or add something waits for your approval first, unless you
          turn on approving it automatically for that routine.
        </p>
        {host && (
          <div className="stack-sm">
            <label className="field">
              <span>Routines run on</span>
              <select className="input" value={host.selected} disabled={!canChooseHost(host)} onChange={(e) => void chooseHost(e.target.value)}>
                <option value="">{HOST_ANY}</option>
                {host.options.map((o) => (
                  <option key={o.id} value={o.id}>
                    {o.label}
                  </option>
                ))}
              </select>
            </label>
            {host.caption ? <p className="muted small">{host.caption}</p> : !canChooseHost(host) && <p className="muted small">{NO_OTHER_DEVICES}</p>}
          </div>
        )}
        {routines === null ? (
          <p className="muted small">{error ? "" : "Loading…"}</p>
        ) : routines.length === 0 ? (
          <p className="muted small">{NO_ROUTINES}</p>
        ) : (
          <ul className="routine-list">
            {routines.map((r) => (
              <RoutineRow
                key={r.id}
                routine={r}
                onToggle={(enabled) => void change(routineSetEnabled(r.id, enabled))}
                onSchedule={(schedule) => void change(routineSetSchedule(r.id, schedule))}
                onAutoApprove={(enabled) => void change(routineSetAutoApprove(r.id, enabled))}
              />
            ))}
          </ul>
        )}
        {error && <p role="alert">{error}</p>}
      </section>

      <section className="card about" aria-labelledby="history-heading">
        <h2 id="history-heading" className="section-label">
          History
        </h2>
        <p className="muted small">What your routines produced, most recent first.</p>
        {runs.length === 0 ? (
          <p className="muted small">{NO_RUNS}</p>
        ) : (
          <ul className="run-list">
            {runs.map((run) => (
              <li key={run.id} className="run-item">
                <button className="run-head" aria-expanded={open === run.id} onClick={() => setOpen(open === run.id ? null : run.id)}>
                  <span>{runHeading(run)}</span>
                  <span className={`run-outcome run-outcome--${run.outcome}`}>{outcomeLabel(run.outcome)}</span>
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

function ApprovalRow(props: { approval: ApprovalView; busy: boolean; onApprove: (account: string | undefined) => void; onReject: () => void }) {
  const { approval } = props;
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
            <p className="approval-detail">{actionDetail(approval.action)}</p>
          </>
        ) : (
          <p>{approval.summary}</p>
        )}
        {needsAccountPicker(approval) ? (
          <label className="field">
            <span>Use account</span>
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
        <button className="mode mode--active" disabled={!canApprove(approval) || props.busy} onClick={() => props.onApprove(account || undefined)}>
          {props.busy ? "Working…" : "Approve"}
        </button>
        <button className="mode" disabled={props.busy} onClick={props.onReject}>
          Reject
        </button>
      </div>
    </li>
  );
}

function RoutineRow(props: {
  routine: RoutineView;
  onToggle: (enabled: boolean) => void;
  onSchedule: (schedule: string) => void;
  onAutoApprove: (enabled: boolean) => void;
}) {
  const { routine } = props;
  const [time, setTime] = useState(routine.schedule);
  useEffect(() => setTime(routine.schedule), [routine.schedule]);
  const note = unavailableNote(routine);

  return (
    <li className="routine-row">
      <div className="routine-main">
        <label className="toggle">
          <input type="checkbox" checked={routine.enabled} disabled={!canToggle(routine)} onChange={(e) => props.onToggle(e.target.checked)} />
          <strong>{routine.name}</strong>
        </label>
        <p className="muted small">{DESCRIPTIONS[routine.kind]}</p>
        {note && <p className="small routine-unavailable">{note}</p>}
        {routine.writes && note === null && (
          <>
            <label className="toggle">
              <input type="checkbox" checked={routine.autoApprove} onChange={(e) => props.onAutoApprove(e.target.checked)} />
              Approve automatically
            </label>
            <p className="muted small">{autoApproveNote(routine.kind)}</p>
          </>
        )}
        <p className="muted small">{lastRunLine(routine)}</p>
      </div>
      {hasSchedule(routine) && (
        <label className="field routine-time">
          <span>Time</span>
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
