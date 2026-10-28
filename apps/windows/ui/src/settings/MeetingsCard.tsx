import { useEffect, useState } from "react";
import { meetingSettings, setMeetingSettings } from "../api";
import { describeError, isCommandError } from "../errors";
import type { MeetingSettings } from "../generated/types";

function message(e: unknown): string {
  return isCommandError(e) ? describeError(e) : String(e);
}

/** Settings → Meetings (M3.3 Task 6): how calls are noticed and started. */
export function MeetingsCard() {
  const [settings, setSettings] = useState<MeetingSettings | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    meetingSettings()
      .then(setSettings)
      .catch((e: unknown) => setError(message(e)));
  }, []);

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
      {error && <p role="alert">{error}</p>}
    </section>
  );
}
