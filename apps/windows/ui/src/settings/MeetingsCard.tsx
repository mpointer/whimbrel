import { useEffect, useState } from "react";
import {
  chooseMeetingSnapshotsFolder,
  meetingSettings,
  meetingSnapshotSettings,
  resetMeetingSnapshotsFolder,
  setMeetingSettings,
  setMeetingSnapshotsEnabled,
} from "../api";
import { describeError, isCommandError } from "../errors";
import type { MeetingSettings, SnapshotSettings } from "../generated/types";

function message(e: unknown): string {
  return isCommandError(e) ? describeError(e) : String(e);
}

/** Settings → Meetings (M3.3 Task 6): how calls are noticed and started. */
export function MeetingsCard() {
  const [settings, setSettings] = useState<MeetingSettings | null>(null);
  const [snapshots, setSnapshots] = useState<SnapshotSettings | null>(null);
  const [choosing, setChoosing] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    meetingSettings()
      .then(setSettings)
      .catch((e: unknown) => setError(message(e)));
    meetingSnapshotSettings()
      .then(setSnapshots)
      .catch((e: unknown) => setError(message(e)));
  }, []);

  const saveSnapshots = (change: () => Promise<SnapshotSettings>) => {
    setError(null);
    change()
      .then(setSnapshots)
      .catch((e: unknown) => setError(message(e)));
  };

  const chooseFolder = () => {
    setChoosing(true);
    saveSnapshots(() => chooseMeetingSnapshotsFolder().finally(() => setChoosing(false)));
  };

  const save = (next: MeetingSettings) => {
    setSettings(next);
    setMeetingSettings(next)
      .then(setSettings)
      .catch((e: unknown) => setError(message(e)));
  };

  if (!settings) return <section className="card about">{error ? <p role="alert">{error}</p> : "Loading…"}</section>;

  return (
    <section className="card about" aria-labelledby="meetings-heading">
      <h2 id="meetings-heading" className="section-label">
        Meetings
      </h2>
      <label className="toggle">
        <input
          type="checkbox"
          checked={settings.heuristicDetection}
          onChange={(e) => save({ ...settings, heuristicDetection: e.target.checked })}
        />
        Notice calls in Zoom, Teams, Slack, Webex and the browser
      </label>
      <label className="toggle">
        <input type="checkbox" checked={settings.autoStart} onChange={(e) => save({ ...settings, autoStart: e.target.checked })} />
        Start recording automatically after a 10-second countdown
      </label>
      <label className="field-row">
        <span>Ask this many minutes before a calendar meeting</span>
        <input
          className="input short"
          type="number"
          min={1}
          max={10}
          value={settings.leadMinutes}
          onChange={(e) => save({ ...settings, leadMinutes: Number(e.target.value) })}
        />
      </label>
      <label className="field-row">
        <span>Prepare a Meeting Prep brief this many minutes before a meeting</span>
        <input
          className="input short"
          type="number"
          min={5}
          max={120}
          value={settings.prepLeadMinutes}
          onChange={(e) => save({ ...settings, prepLeadMinutes: Number(e.target.value) })}
        />
      </label>
      <p className="muted small">Detection watches which app holds your microphone and speakers. Nothing is recorded until you accept.</p>
      {snapshots && (
        <>
          <label className="toggle">
            <input
              type="checkbox"
              checked={snapshots.enabled}
              onChange={(e) => {
                const enabled = e.target.checked;
                setSnapshots({ ...snapshots, enabled });
                saveSnapshots(() => setMeetingSnapshotsEnabled(enabled));
              }}
            />
            Save slides and shared screens
          </label>
          <p className="muted small">Saves one image per slide shown in recorded meetings to a folder you can browse.</p>
          <div className="field-row">
            <span>Snapshots folder</span>
            <code className="small grow" title={snapshots.folder}>
              {snapshots.folder}
            </code>
            <button className="mode" disabled={choosing} onClick={chooseFolder}>
              Choose…
            </button>
            <button className="mode" disabled={snapshots.isDefault} onClick={() => saveSnapshots(resetMeetingSnapshotsFolder)}>
              Reset
            </button>
          </div>
          <p className="muted small">A new folder applies to meetings recorded from now on. Earlier meetings keep theirs.</p>
        </>
      )}
      {error && <p role="alert">{error}</p>}
    </section>
  );
}
