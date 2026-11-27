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
} from "../../../../api";
import { describeError, isCommandError } from "../../../../errors";
import type { MeetingSettings, SnapshotSettings, VoicePersonView } from "../../../../generated/types";
import { t } from "../../../../l10n";
import { noticeText, saveNoticeSettings, useRecordingNotice } from "../../../../meetings/recordingNotice";
import {
  WButton,
  breakAtSeparators,
  WSettingsCard,
  WSettingsControlRow,
  WSettingsList,
  WSettingsListRow,
  WSettingsNote,
  WSettingsNumberRow,
  WSettingsSelectRow,
  WSettingsTextRow,
  WSettingsToggleRow,
} from "../../../../system";
import { PROFILE_SAVED_EVENT } from "../../../ProfileCard";
import "./meetops.css";

/** "Ask if I'm still recording after": Off or 1-4 hours, as on the Mac. */
export const CHECK_IN_HOURS = [0, 1, 2, 3, 4] as const;

export function checkInLabel(hours: number): string {
  return hours === 0 ? t("Off") : hours === 1 ? t("1 hour") : t("{count} hours", { count: hours });
}

function message(e: unknown): string {
  return isCommandError(e) ? describeError(e) : String(e);
}

/** "Remembered voices": the people with a voice print, each forgettable. Shown on request, as in the old card. */
function RememberedVoices({ onError }: { onError: (message: string) => void }) {
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
      <WSettingsControlRow label={t("Remembered voices…")}>
        <WButton onClick={show}>{t("Show")}</WButton>
      </WSettingsControlRow>
    );
  }
  return (
    <WSettingsList
      label={t("Remembered voices…")}
      isEmpty={people.length === 0}
      empty={t("No remembered voices yet.")}
      footer={<WButton onClick={() => setPeople(null)}>{t("Hide")}</WButton>}
    >
      {people.map((p) => (
        <WSettingsListRow key={p.personKey} title={p.name} actions={<WButton onClick={() => forget(p.personKey)} aria-label={`${t("Remove")}, ${p.name}`}>{t("Remove")}</WButton>} />
      ))}
    </WSettingsList>
  );
}

/** A row of words, no control: the label, then the sentence at the same size and colour as any row's label. */
function PlainRow({ id, label, children }: { id: string; label: string; children: string }) {
  return (
    <div className="w-setting w-sr rs-meetops-plainrow" data-testid={id}>
      <div className="w-setting-text">
        <span className="w-setting-label">{label}</span>
        <p className="rs-meetops-plain">{children}</p>
      </div>
    </div>
  );
}

/** Meetings card: noticing calls, starting, the live transcript, voices, reminders, the recording notice and
 *  consent reminder (always drawn, full size, never behind a disclosure), and slide snapshots. Same commands as MeetingsCard. */
export function MeetingsCardRedesign() {
  const [settings, setSettings] = useState<MeetingSettings | null>(null);
  const [snapshots, setSnapshots] = useState<SnapshotSettings | null>(null);
  const [choosing, setChoosing] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const { settings: notice } = useRecordingNotice();

  const saveNotice = (enabled: boolean, text: string) => {
    setError(null);
    saveNoticeSettings({ enabled, text }).catch((e: unknown) => setError(message(e)));
  };

  const load = () => {
    setError(null);
    meetingSettings()
      .then(setSettings)
      .catch((e: unknown) => setError(message(e)));
    meetingSnapshotSettings()
      .then(setSnapshots)
      .catch((e: unknown) => setError(message(e)));
  };
  useEffect(() => {
    load();
    // A name typed in Profile unlocks question alerts without leaving the page; only `hasNames` is taken.
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

  return (
    <WSettingsCard id="meetings.meetings" title={t("Meetings")} headingLevel={3} loading={!settings && !error} error={error}>
      {!settings && error ? (
        <WSettingsControlRow label={t("Meetings settings")}>
          <WButton onClick={load}>{t("Try again")}</WButton>
        </WSettingsControlRow>
      ) : null}
      {settings ? (
        <>
          <WSettingsToggleRow
            label={t("Notice calls in Zoom, Teams, Slack, Webex and the browser")}
            description={t("Detection watches which app holds your microphone and speakers. Nothing is recorded until you accept.")}
            checked={settings.heuristicDetection}
            onToggle={(on) => save({ ...settings, heuristicDetection: on })}
          />
          <WSettingsToggleRow
            label={t("Start recording automatically after a 10-second countdown")}
            checked={settings.autoStart}
            onToggle={(on) => save({ ...settings, autoStart: on })}
          />
          <WSettingsToggleRow
            label={t("Live transcript during meetings")}
            description={t(
              "Shows what's being said while you record. The full transcript is still made after the meeting and replaces the live one. The live text stays on this PC, and turning this off deletes it.",
            )}
            checked={settings.liveTranscript === true}
            onToggle={(on) => save({ ...settings, liveTranscript: on })}
          />
          <WSettingsToggleRow
            label={t("Alert me when someone asks me something")}
            description={t("Lines that mention you, and the minutes before them, go to your Ask model.")}
            disabled={!settings.hasNames}
            disabledReason={t("Add your name in Settings → Profile first.")}
            checked={settings.hasNames === true && settings.liveQuestionAlerts === true}
            onToggle={(on) => save({ ...settings, liveQuestionAlerts: on })}
          />
          <WSettingsTextRow label={t("Other names people call you")} value={settings.otherNames ?? ""} onCommit={(text) => save({ ...settings, otherNames: text })} />
          <WSettingsToggleRow
            label={t("Remember people's voices")}
            description={t("Whimbrel learns a voice when you confirm or fix a name. Voices stay on your devices and in your encrypted sync.")}
            checked={settings.rememberVoices === true}
            onToggle={(on) => save({ ...settings, rememberVoices: on })}
          />
          <RememberedVoices onError={setError} />
          <WSettingsNumberRow
            label={t("Ask this many minutes before a calendar meeting")}
            value={settings.leadMinutes}
            min={1}
            max={10}
            onCommit={(n) => save({ ...settings, leadMinutes: n })}
          />
          <WSettingsNumberRow
            label={t("Prepare a Meeting Prep brief this many minutes before a meeting")}
            value={settings.prepLeadMinutes}
            min={5}
            max={120}
            onCommit={(n) => save({ ...settings, prepLeadMinutes: n })}
          />
          <WSettingsSelectRow
            label={t("Ask if I'm still recording after")}
            description={t(
              'Shows a "Still recording?" prompt. With no answer in 5 minutes, the recording stops and is saved up to the last words. Recordings always stop at 4 hours. Applies from the next recording.',
            )}
            value={String(settings.checkInHours ?? 1)}
            options={CHECK_IN_HOURS.map((h) => ({ value: String(h), label: checkInLabel(h) }))}
            onChange={(v) => save({ ...settings, checkInHours: Number(v) })}
          />
          {notice ? (
            <>
              <WSettingsToggleRow label={t("Remind me to tell people I'm recording")} checked={notice.enabled} onToggle={(on) => saveNotice(on, notice.text)} />
              {/* Plain rows at the primary text size, always drawn, on or off: the spec never lets these shrink or hide. */}
              <PlainRow id="meetings.recordingNotice" label={t("Recording notice")}>
                {noticeText(notice.text)}
              </PlainRow>
              <PlainRow id="meetings.consentReminder" label={t("Consent reminder")}>
                {t("Some places require everyone's consent before a meeting is recorded. Check the rules where you and the other people are.")}
              </PlainRow>
              {notice.enabled ? (
                <WSettingsTextRow
                  label={t("What to say")}
                  rows={2}
                  placeholder={noticeText("")}
                  value={notice.text}
                  onCommit={(text) => saveNotice(notice.enabled, text)}
                />
              ) : null}
            </>
          ) : null}
          {snapshots ? (
            <>
              <WSettingsToggleRow
                label={t("Save slides and shared screens")}
                description={t("Saves one image per slide shown in recorded meetings to a folder you can browse.")}
                checked={snapshots.enabled}
                onToggle={(enabled) => {
                  setSnapshots({ ...snapshots, enabled });
                  saveSnapshots(() => setMeetingSnapshotsEnabled(enabled));
                }}
              />
              <WSettingsControlRow label={t("Snapshots folder")} description={t("A new folder applies to meetings recorded from now on. Earlier meetings keep theirs.")} stacked>
                <div className="rs-meetops-line">
                  <code className="rs-meetops-path" title={snapshots.folder}>
                    {breakAtSeparators(snapshots.folder)}
                  </code>
                  <WButton disabled={choosing} onClick={chooseFolder}>
                    {t("Choose…")}
                  </WButton>
                  <WButton disabled={snapshots.isDefault} aria-describedby={snapshots.isDefault ? "rs-meetops-reset-why" : undefined} onClick={() => saveSnapshots(resetMeetingSnapshotsFolder)}>
                    {t("Reset")}
                  </WButton>
                </div>
                {snapshots.isDefault ? <WSettingsNote id="rs-meetops-reset-why">{t("This is already the default folder.")}</WSettingsNote> : null}
              </WSettingsControlRow>
            </>
          ) : null}
        </>
      ) : null}
    </WSettingsCard>
  );
}
