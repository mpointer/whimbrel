import { useEffect, useState } from "react";
import {
  chooseMeetingSnapshotsFolder,
  forgetVoice,
  meetingSettings,
  meetingSnapshotSettings,
  resetMeetingSnapshotsFolder,
  setMeetingSettings,
  setMeetingSnapshotsEnabled,
  voicePeople,
} from "../api";
import { describeError, isCommandError } from "../errors";
import type { MeetingSettings, SnapshotSettings, VoicePersonView } from "../generated/types";
import { t } from "../l10n";
import { noticeText, saveNoticeSettings, useRecordingNotice } from "../meetings/recordingNotice";
import { PROFILE_SAVED_EVENT } from "./ProfileCard";

function message(e: unknown): string {
  return isCommandError(e) ? describeError(e) : String(e);
}

/** "Remembered voices…": the people with a voice print, each forgettable (speaker names). */
export function RememberedVoices({ onError }: { onError: (message: string) => void }) {
  const [people, setPeople] = useState<VoicePersonView[] | null>(null);
  const show = () =>
    voicePeople()
      .then(setPeople)
      .catch((e: unknown) => onError(message(e)));
  const forget = (personKey: string) =>
    forgetVoice(personKey)
      .then(setPeople)
      .catch((e: unknown) => onError(message(e)));

  if (people == null) {
    return (
      <div className="toolbar">
        <button className="mode" onClick={show}>
          {t("Remembered voices…")}
        </button>
      </div>
    );
  }
  return (
    <div className="field" aria-label={t("Remembered voices…")}>
      {people.length === 0 ? (
        <p className="muted small">{t("No remembered voices yet.")}</p>
      ) : (
        <ul className="bullets">
          {people.map((p) => (
            <li key={p.personKey} className="field-row">
              <span className="grow">{p.name}</span>
              <button className="mode" onClick={() => forget(p.personKey)}>
                {t("Remove")}
              </button>
            </li>
          ))}
        </ul>
      )}
      <div className="toolbar">
        <button className="mode" onClick={() => setPeople(null)}>
          {t("Hide")}
        </button>
      </div>
    </div>
  );
}

/** Settings → Meetings (M3.3 Task 6): how calls are noticed and started. */
export function MeetingsCard() {
  const [settings, setSettings] = useState<MeetingSettings | null>(null);
  const [snapshots, setSnapshots] = useState<SnapshotSettings | null>(null);
  const [choosing, setChoosing] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const { settings: notice } = useRecordingNotice();
  // The wording being typed; saved when the box loses focus.
  const [draft, setDraft] = useState<string | null>(null);
  // "Other names people call you" while being typed; saved when the box loses focus.
  const [namesDraft, setNamesDraft] = useState<string | null>(null);
  const saveNotice = (enabled: boolean, text: string) => {
    setError(null);
    saveNoticeSettings({ enabled, text })
      .then(() => setDraft(null))
      .catch((e: unknown) => setError(message(e)));
  };

  useEffect(() => {
    meetingSettings()
      .then(setSettings)
      .catch((e: unknown) => setError(message(e)));
    meetingSnapshotSettings()
      .then(setSnapshots)
      .catch((e: unknown) => setError(message(e)));
    // A name typed in the Profile card above unlocks question alerts without leaving the page.
    // Only `hasNames` is taken, so an edit in progress here isn't overwritten.
    const recheck = () =>
      meetingSettings().then(
        (fresh) => setSettings((s) => s && { ...s, hasNames: fresh.hasNames }),
        () => {},
      );
    window.addEventListener(PROFILE_SAVED_EVENT, recheck);
    return () => window.removeEventListener(PROFILE_SAVED_EVENT, recheck);
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

  if (!settings) return <section className="card about">{error ? <p role="alert">{error}</p> : t("Loading…")}</section>;

  return (
    <section className="card about" aria-labelledby="meetings-heading">
      <h2 id="meetings-heading" className="section-label">
        {t("Meetings")}
      </h2>
      <label className="toggle">
        <input
          type="checkbox"
          checked={settings.heuristicDetection}
          onChange={(e) => save({ ...settings, heuristicDetection: e.target.checked })}
        />
        {t("Notice calls in Zoom, Teams, Slack, Webex and the browser")}
      </label>
      <label className="toggle">
        <input type="checkbox" checked={settings.autoStart} onChange={(e) => save({ ...settings, autoStart: e.target.checked })} />
        {t("Start recording automatically after a 10-second countdown")}
      </label>
      {/* "Forward me a live question": shown off, and locked, until there is a name to listen for. */}
      <label className="toggle">
        <input
          type="checkbox"
          disabled={!settings.hasNames}
          checked={settings.hasNames && settings.liveQuestionAlerts}
          onChange={(e) => save({ ...settings, liveQuestionAlerts: e.target.checked })}
        />
        {t("Alert me when someone asks me something")}
      </label>
      {!settings.hasNames && <p className="muted small">{t("Add your name in Settings → Profile first.")}</p>}
      <label className="field-row">
        <span>{t("Other names people call you")}</span>
        <input
          className="input"
          value={namesDraft ?? settings.otherNames}
          onChange={(e) => setNamesDraft(e.target.value)}
          onBlur={() => {
            if (namesDraft !== null) save({ ...settings, otherNames: namesDraft });
            setNamesDraft(null);
          }}
        />
      </label>
      <p className="muted small">{t("Lines that mention you, and the minutes before them, go to your Ask model.")}</p>
      <label className="toggle">
        <input
          type="checkbox"
          checked={settings.rememberVoices}
          onChange={(e) => save({ ...settings, rememberVoices: e.target.checked })}
        />
        {t("Remember people's voices")}
      </label>
      <p className="muted small">
        {t("Whimbrel learns a voice when you confirm or fix a name. Voices stay on your devices and in your encrypted sync.")}
      </p>
      <RememberedVoices onError={setError} />
      <label className="field-row">
        <span>{t("Ask this many minutes before a calendar meeting")}</span>
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
        <span>{t("Prepare a Meeting Prep brief this many minutes before a meeting")}</span>
        <input
          className="input short"
          type="number"
          min={5}
          max={120}
          value={settings.prepLeadMinutes}
          onChange={(e) => save({ ...settings, prepLeadMinutes: Number(e.target.value) })}
        />
      </label>
      <p className="muted small">
        {t("Detection watches which app holds your microphone and speakers. Nothing is recorded until you accept.")}
      </p>
      {notice && (
        <>
          <label className="toggle">
            <input type="checkbox" checked={notice.enabled} onChange={(e) => saveNotice(e.target.checked, notice.text)} />
            {t("Remind me to tell people I'm recording")}
          </label>
          {notice.enabled && (
            <textarea
              className="input"
              rows={2}
              aria-label={t("Remind me to tell people I'm recording")}
              placeholder={noticeText("")}
              value={draft ?? notice.text}
              onChange={(e) => setDraft(e.target.value)}
              onBlur={() => draft !== null && saveNotice(notice.enabled, draft)}
            />
          )}
          <p className="muted small">
            {t("Some places require everyone's consent before a meeting is recorded. Check the rules where you and the other people are.")}
          </p>
        </>
      )}
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
            {t("Save slides and shared screens")}
          </label>
          <p className="muted small">{t("Saves one image per slide shown in recorded meetings to a folder you can browse.")}</p>
          <div className="field-row">
            <span>{t("Snapshots folder")}</span>
            <code className="small grow" title={snapshots.folder}>
              {snapshots.folder}
            </code>
            <button className="mode" disabled={choosing} onClick={chooseFolder}>
              {t("Choose…")}
            </button>
            <button className="mode" disabled={snapshots.isDefault} onClick={() => saveSnapshots(resetMeetingSnapshotsFolder)}>
              {t("Reset")}
            </button>
          </div>
          <p className="muted small">{t("A new folder applies to meetings recorded from now on. Earlier meetings keep theirs.")}</p>
        </>
      )}
      {error && <p role="alert">{error}</p>}
    </section>
  );
}
