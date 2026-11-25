// The Memory card: the state row (words, On/Off, Pause), what Memory saves, "Show screen captures in the Library",
// the status line and the counters, and the "Never captured" statement. The commands are the old card's and the tray's.

import { useEffect, useState } from "react";
import { capturePauseUntil, captureResumeNow, captureStatus, launchState, librarySettings, setCaptureEnabled, setShowScreenCaptures } from "../../../../../api";
import { describeError, isCommandError } from "../../../../../errors";
import type { CaptureStatus } from "../../../../../generated/types";
import { useL10n } from "../../../../../l10n";
import { pauseChoiceLabel } from "../../../../../shell/redesign/captureChoices";
import { PAUSE_CHOICES, pauseDeadline, pauseEndLabel, type PauseChoice } from "../../../../../shell/redesign/captureModel";
import { WButton, WSettingsCard, WSettingsControlRow, WSettingsNote, WSettingsToggleRow, type WStatusTone } from "../../../../../system";
import { memoryDetailLine, memoryRow } from "./memoryTopModel";
import "./memtop.css";

const STATE_TONE: Record<string, WStatusTone> = { on: "good", off: "neutral", stopped: "problem", paused: "warning", pausedByApp: "warning" };

const message = (e: unknown): string => (isCommandError(e) ? describeError(e) : String(e));

export function MemoryCard() {
  const { t, locale } = useL10n();
  const [status, setStatus] = useState<CaptureStatus | null>(null);
  const [now, setNow] = useState(() => Date.now());
  const [showCaptures, setShowCaptures] = useState<boolean | null>(null);
  const [error, setError] = useState<string | null>(null);
  // A failed poll is its own line and goes at the next good poll; an action's error stays until the next action.
  const [pollFailed, setPollFailed] = useState(false);
  // Opened without a library: Memory cannot run, whatever the capture status says.
  const [noLibrary, setNoLibrary] = useState(false);

  useEffect(() => {
    launchState().then(
      (v) => setNoLibrary(v.stage.kind === "openedWithoutStore"),
      () => {},
    );
  }, []);
  useEffect(() => {
    const refresh = () =>
      captureStatus().then(
        (s) => {
          setStatus(s);
          setNow(Date.now());
          setPollFailed(false);
        },
        () => setPollFailed(true),
      );
    void refresh();
    const timer = setInterval(refresh, 5000);
    return () => clearInterval(timer);
  }, []);
  useEffect(() => {
    librarySettings().then(
      (s) => setShowCaptures(s.showScreenCaptures),
      () => {},
    );
  }, []);

  const apply = (call: Promise<CaptureStatus>) =>
    call.then(
      (s) => {
        setStatus(s);
        setNow(Date.now());
        setError(null);
      },
      (e: unknown) => setError(message(e)),
    );
  const toggleShow = (show: boolean) =>
    setShowScreenCaptures(show).then(
      (s) => setShowCaptures(s.showScreenCaptures),
      (e: unknown) => setError(message(e)),
    );

  const row = status ? memoryRow(status, now, locale, noLibrary) : null;
  const detail = status && !noLibrary ? memoryDetailLine(status, now) : null;
  return (
    <WSettingsCard
      id="capture.capture"
      headingLevel={3}
      title={t("Memory")}
      status={row ? { tone: STATE_TONE[row.memory?.state ?? "off"], children: row.words } : undefined}
      loading={status === null && error === null && !pollFailed}
      error={[pollFailed ? t("Could not read Memory's state. Trying again.") : null, error]}
    >
      {status && row ? (
        <>
          {/* Read out when the state changes, and only the state words: the line below changes with every check. */}
          <span role="status" className="w-visually-hidden">
            {row.words}
          </span>
          <WSettingsToggleRow
            label={t("Memory")}
            description={t("Memory saves the text of the window you are working in, and nothing from excluded apps.")}
            checked={status.enabled && !noLibrary}
            disabled={row.switchDisabled}
            disabledReason={row.reason ?? undefined}
            onToggle={(on) => void apply(setCaptureEnabled(on))}
          />
          {!row.switchDisabled && row.reason ? <p className="w-setting-note">{row.reason}</p> : null}
          {row.switchHint ? <p className="w-setting-note">{row.switchHint}</p> : null}
          {detail ? <p className="w-setting-note">{detail}</p> : null}
          <WSettingsControlRow
            label={t("Pause memory")}
            description={row.pause === "resume" && row.memory?.state === "paused" ? t("Paused until {time}", { time: pauseEndLabel(row.memory.resumeAt, now, locale) }) : undefined}
            disabled={row.pause === "none"}
            disabledReason={row.pauseReason ?? undefined}
          >
            <span className="rs-memtop-buttons">
              {row.pause === "resume" ? (
                <WButton onClick={() => void apply(captureResumeNow())}>{t("Resume now")}</WButton>
              ) : (
                PAUSE_CHOICES.map((c: PauseChoice) => (
                  <WButton key={c} disabled={row.pause === "none"} onClick={() => void apply(capturePauseUntil(pauseDeadline(c, Date.now())))}>
                    {pauseChoiceLabel(c)}
                  </WButton>
                ))
              )}
            </span>
          </WSettingsControlRow>
          {showCaptures !== null ? <WSettingsToggleRow label={t("Show screen captures in the Library")} checked={showCaptures} onToggle={(v) => void toggleShow(v)} /> : null}
          <WSettingsNote>
            {t("Never captured: the lock screen, sign-in and UAC prompts, password managers, InPrivate and Incognito windows, and Whimbrel itself. Screenshots are never saved.")}
          </WSettingsNote>
        </>
      ) : null}
    </WSettingsCard>
  );
}
