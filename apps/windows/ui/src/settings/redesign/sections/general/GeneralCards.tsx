// General → General (start at sign-in, updates), Language, and About this PC, drawn on the design system's parts.
// Each calls the same commands as the old GeneralCard and AboutCard.
import { useEffect, useState } from "react";
import {
  appStatus,
  checkForUpdates,
  generalSettings,
  installUpdate,
  languageSettings,
  openExternal,
  perfReportText,
  sendFeedback,
  setAutostart,
  setLanguage as saveLanguage,
  setWriteIn,
  updateCheckStatus,
} from "../../../../api";
import { describeError, isCommandError } from "../../../../errors";
import type { AppStatus, FeedbackResult, GeneralSettings, LanguageSettings, UpdateCheck } from "../../../../generated/types";
import { setLanguage, t } from "../../../../l10n";
import { WButton, WNoticeBox, WSettingsCard, WSettingsControlRow, WSettingsLinkRow, WSettingsNote, WSettingsSelectRow, WSettingsToggleRow, WSettingsValueList } from "../../../../system";
import { DISCORD_URL } from "../../../AboutCard";
import { languageOptions, localModelNote, writeInOptions } from "../../../languageModel";
import { FOLLOW_MS, isBusy } from "../../../updateModel";
import { aboutItems, updateRowState } from "./generalModel";
import "./general.css";

const message = (e: unknown): string => (isCommandError(e) ? describeError(e) : String(e));

/** Check for Updates, and Restart to Update once one is downloaded. Restarting stops and saves a recording first. */
function UpdateRows(props: { onError: (text: string | null) => void }) {
  const { onError } = props;
  const [check, setCheck] = useState<UpdateCheck>({ state: "idle" });
  const [restarting, setRestarting] = useState(false);

  useEffect(() => {
    updateCheckStatus().then(setCheck, () => {});
  }, []);
  useEffect(() => {
    if (!isBusy(check)) return;
    const id = setInterval(() => updateCheckStatus().then(setCheck, () => {}), FOLLOW_MS);
    return () => clearInterval(id);
  }, [check.state]);

  const state = updateRowState(check, restarting);
  return (
    <>
      <WSettingsControlRow label={t("Updates")} description={state.line ?? undefined}>
        <WButton
          disabled={state.busy}
          onClick={() => {
            onError(null);
            checkForUpdates().then(setCheck, (e: unknown) => onError(message(e)));
          }}
        >
          {t("Check for Updates")}
        </WButton>
      </WSettingsControlRow>
      {state.canRestart && (
        <WSettingsControlRow label={t("Restart to Update")} description={t("Restarting waits while a meeting is being recorded or processed.")}>
          <WButton
            variant="primary"
            disabled={restarting}
            onClick={() => {
              setRestarting(true);
              onError(null);
              installUpdate().catch((e: unknown) => {
                setRestarting(false);
                onError(message(e));
              });
            }}
          >
            {restarting ? t("Restarting…") : t("Restart to Update")}
          </WButton>
        </WSettingsControlRow>
      )}
    </>
  );
}

/** Start at sign-in, and Check for Updates. */
export function GeneralSettingsCard() {
  const [settings, setSettings] = useState<GeneralSettings | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    generalSettings()
      .then(setSettings)
      .catch((e: unknown) => setError(message(e)));
  }, []);

  const toggle = (enabled: boolean) =>
    setAutostart(enabled)
      .then((s) => {
        setSettings(s);
        setError(null);
      })
      .catch((e: unknown) => setError(message(e)));

  return (
    <WSettingsCard id="general.general" title={t("General")} headingLevel={3} error={error}>
      {settings && (
        <WSettingsToggleRow
          kind="checkbox"
          label={t("Start Whimbrel when you sign in to Windows")}
          checked={settings.autostart}
          onToggle={toggle}
          disabled={!settings.autostartAvailable}
          disabledReason={t("Available in release builds only.")}
        />
      )}
      <UpdateRows onError={setError} />
    </WSettingsCard>
  );
}

/** The app's language and the one notes are written in. A change applies at once. */
export function LanguageCard() {
  const [settings, setSettings] = useState<LanguageSettings | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    languageSettings().then(setSettings, (e: unknown) => setError(message(e)));
  }, []);

  const applied = (s: LanguageSettings) => {
    setSettings(s);
    setError(null);
    setLanguage(s.uiLanguage);
  };

  const note = settings ? localModelNote(settings) : null;
  return (
    <WSettingsCard title={t("Language")} headingLevel={3} loading={!settings && !error} error={error}>
      {settings && (
        <>
          <WSettingsSelectRow
            label={t("Language")}
            value={settings.choice}
            options={languageOptions(settings)}
            onChange={(value) => saveLanguage(value).then(applied, (err: unknown) => setError(message(err)))}
          />
          <WSettingsSelectRow
            label={t("Write notes and summaries in")}
            description={[t("Routines run on your primary device and write in that device's language."), note].filter(Boolean).join(" ")}
            value={settings.writeIn}
            options={writeInOptions(settings)}
            onChange={(value) => setWriteIn(value).then(applied, (err: unknown) => setError(message(err)))}
          />
        </>
      )}
    </WSettingsCard>
  );
}

/** About this PC: the facts, Send feedback, Copy performance report, the Discord, and startup problems. */
export function AboutThisPcCard() {
  const [status, setStatus] = useState<AppStatus | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [feedbackError, setFeedbackError] = useState<string | null>(null);
  const [preparing, setPreparing] = useState(false);
  const [feedback, setFeedback] = useState<FeedbackResult | null>(null);
  const [reportCopied, setReportCopied] = useState(false);

  useEffect(() => {
    appStatus()
      .then(setStatus)
      .catch((e: unknown) => setError(message(e)));
  }, []);

  return (
    <WSettingsCard
      id="general.about"
      title={t("About this PC")}
      headingLevel={3}
      loading={!status && !error}
      error={error ? t("Couldn't read the app status: {error}", { error }) : null}
    >
      {status && (
        <>
          <WSettingsValueList label={t("About this PC")} items={aboutItems(status)} />
          <WSettingsControlRow
            label={t("Send feedback")}
            description={t(
              "Opens an email to the Whimbrel team and saves a diagnostics file to attach. It has recent logs and settings status, with email addresses and meeting titles removed. No recordings, transcripts or keys.",
            )}
            error={feedbackError}
          >
            <WButton
              disabled={preparing}
              onClick={() => {
                setFeedbackError(null);
                setFeedback(null);
                setPreparing(true);
                sendFeedback()
                  .then(setFeedback, (e: unknown) => setFeedbackError(message(e)))
                  .finally(() => setPreparing(false));
              }}
            >
              {preparing ? t("Preparing…") : t("Send feedback…")}
            </WButton>
          </WSettingsControlRow>
          {feedback?.bundlePath && <WSettingsNote>{t("Saved {file}. Attach it to the email.", { file: feedback.bundlePath })}</WSettingsNote>}
          {feedback?.bundleError && <WSettingsNote>{t("Couldn't save the diagnostics file: {error}", { error: feedback.bundleError })}</WSettingsNote>}
          <WSettingsControlRow
            label={t("Performance report")}
            description={
              reportCopied
                ? t("Copied. Paste it into an email or message.")
                : t("The last hour of Whimbrel's CPU and memory use, as numbers only: no meeting titles, text or names.")
            }
          >
            <WButton
              onClick={() => {
                setReportCopied(false);
                perfReportText()
                  .then((text) => navigator.clipboard.writeText(text))
                  .then(() => setReportCopied(true), () => {});
              }}
            >
              {t("Copy performance report")}
            </WButton>
          </WSettingsControlRow>
          <WSettingsLinkRow kind="external" label={t("Discord")} href={DISCORD_URL} linkText={t("Join the Whimbrel Discord…")} onPress={() => void openExternal(DISCORD_URL).catch(() => {})} />
          {status.startupErrors.length > 0 && (
            <div className="rs-general-startup">
              <WNoticeBox tone="error" title={t("Startup problems")}>
                <ul>
                  {status.startupErrors.map((e) => (
                    <li key={e}>{e}</li>
                  ))}
                </ul>
              </WNoticeBox>
            </div>
          )}
        </>
      )}
    </WSettingsCard>
  );
}
