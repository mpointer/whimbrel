// The tray popup with the redesign switch on (docs/ui-redesign/screens/menu-bar.md). Order: Recording, Memory, Needs
// attention, Next meeting, then Open Whimbrel and Quit. Every control calls what the old popup's controls call. What each
// row shows in each state is `popupRows` (popupModel.ts). Return opens Whimbrel, Escape closes, arrow keys move between controls.
//
// Layout: the header, the recording row with Stop, the memory state line and the footer are pinned, at any text size; the rest
// (the pause choices, notes, Needs attention, Next meeting) scrolls between them, with a line and a chevron when it is clipped.
//
// Reads on open: the recording status (polled), the tray status (once: it also carries the recording's start and the sync
// facts and whether a Google sign-in expired: the tray already keeps that in memory), the capture status (which also says whether
// screen-text reading is failing), the launch stage and the next meeting. Nothing here blocks the main thread: the Google
// sign-in and the sync status were read with sync commands on every open and are not read here.

import { useEffect, useRef, useState } from "react";
import { emit } from "@tauri-apps/api/event";
import {
  capturePauseUntil,
  captureResumeNow,
  captureStatus,
  launchState,
  meetingBarShow,
  nextMeetingPrep,
  popupClose,
  popupOpenMain,
  popupOpenTopic,
  popupQuit,
  setCaptureEnabled,
  trayStatus,
} from "../api";
import { NoStoreBanner } from "../launch/NoStoreBanner";
import { useTopicLookup } from "../topics/TopicChip";
import { timeLabel } from "../library/rows";
import { recordedSeconds } from "../meetings/redesign/liveModel";
import type { CaptureStatus, NextMeetingView, TrayStatus } from "../generated/types";
import { useL10n } from "../l10n";
import { useRecording } from "../shell/redesign/useRecording";
import { PAUSE_CHOICES, pauseDeadline, type PauseChoice } from "../shell/redesign/captureModel";
import { WAppMark, WButton, WMemoryControl, WMenuSection, WNotice, WRecordingRow, WRoot, WScrollArea, mainActionOnReturn, useArrowKeys, useCompactWindow, useWindowKeys, WINDOW_DESIGN } from "../system";
import { memoryWords, popupRows, readProblemLine, stoppedLine, syncLine } from "./popupModel";
import { shortPauseLabel } from "./popupChoices";

/** Opens the main window, then Settings at a section (the same two calls the no-store notice makes). */
const openSettings = (section: string) => void popupOpenMain().then(() => emit("open-settings", section), () => {});

export function RedesignedPopup() {
  const { t, locale } = useL10n();
  const rec = useRecording(true);
  // When the popup has less CSS space than its design size (the shell capped it to a small screen at a large text size) the
  // explanation of a recording problem moves into the scrolling part, so the pinned part keeps Stop and Memory in view.
  const large = useCompactWindow(WINDOW_DESIGN.popup);
  const [next, setNext] = useState<NextMeetingView | null>(null);
  const [topic] = useTopicLookup(next ? [{ kind: "event", id: next.eventId, title: next.title }] : []);
  const [capture, setCapture] = useState<CaptureStatus | null>(null);
  const [tray, setTray] = useState<TrayStatus | null>(null);
  const [trayFailed, setTrayFailed] = useState(false);
  const [busy, setBusy] = useState(false);
  const [noStore, setNoStore] = useState(false);

  useEffect(() => {
    captureStatus().then(setCapture, () => {});
    // The tray status is the second source for a running recording, and the source of the sync facts. It also refreshes the tray.
    trayStatus().then(setTray, () => setTrayFailed(true));
    nextMeetingPrep().then(setNext, () => setNext(null));
    launchState().then((v) => setNoStore(v.stage.kind === "openedWithoutStore"), () => {});
  }, []);

  const rows = popupRows({
    recordingView: rec.view,
    viewFailed: rec.failed,
    tray,
    trayFailed,
    busy: rec.busy,
    capture,
    now: rec.now,
    noStore,
    hasNext: next != null,
  });

  // "Try again" when neither read worked: both are asked again now, and a good answer clears the failure.
  const retryRead = () => {
    rec.reload();
    trayStatus().then(
      (status) => {
        setTray(status);
        setTrayFailed(false);
      },
      () => setTrayFailed(true),
    );
  };

  // A memory change: the answer is the new status, and the tray icon and tooltip are refreshed at once (`tray_status`).
  const changeMemory = (call: () => Promise<CaptureStatus>) => {
    setBusy(true);
    call()
      .then((status) => {
        setCapture(status);
        void trayStatus().then(setTray, () => {});
      })
      .catch(() => {})
      .finally(() => setBusy(false));
  };

  const main = useRef<HTMLElement>(null);
  // Return opens Whimbrel (a safe action); Escape closes.
  useWindowKeys(mainActionOnReturn("popup", () => void popupOpenMain()), () => void popupClose().catch(() => {}));
  useArrowKeys(main);

  const block = rows.recording;
  const row = block?.kind === "row" ? block.row : null;
  // The time shows only from the detailed status: the tray's start time cannot say how long the recording was paused.
  const elapsed = row?.source === "detail" && rec.view && rec.view.startedUnixMs != null ? recordedSeconds(rec.view, rec.now, rec.readAt) : undefined;
  const memory = rows.memory;
  const sync = syncLine(tray);
  const memoryProps = memory
    ? {
        state: memory.state,
        label: memoryWords(memory, rec.now, locale),
        choices: PAUSE_CHOICES.map((c) => ({ id: c, label: shortPauseLabel(c) })),
        busy,
        onPause: (id: string) => changeMemory(() => capturePauseUntil(pauseDeadline(id as PauseChoice, Date.now()))),
        onResumeNow: () => changeMemory(captureResumeNow),
        onTurnOff: () => changeMemory(() => setCaptureEnabled(false)),
        onTurnOn: () => changeMemory(() => setCaptureEnabled(true)),
        onWhatIsCaptured: () => openSettings("capture"),
        // Stopped is not a dead end: why, in plain words, and where to go.
        onOpenSettings: () => openSettings("capture"),
        notes: [
          stoppedLine(memory),
          readProblemLine(memory),
          // A way to the OCR languages, under the reason.
          readProblemLine(memory) ? (
            <WButton key="ocr-settings" variant="quiet" link onClick={() => openSettings("capture")}>
              {t("Open Settings")}
            </WButton>
          ) : null,
          sync,
        ].filter((n) => !!n),
      }
    : null;

  // "Show in-meeting window" belongs to the recording row; at a large text size it waits in the scrolling part to leave room.
  const showWindowLink = (
    <div>
      <WButton variant="quiet" link onClick={() => void meetingBarShow().catch(() => {})}>
        {t("Show in-meeting window")}
      </WButton>
    </div>
  );

  // Compact layout with a recording problem: the reason leaves the pinned row (it would push Stop out of view), so it is the FIRST thing
  // in the scrolling part under its own "Recording problem" label, and the Memory controls follow it there. It never sits below them:
  // pinned Memory above a scrolling reason would put the reason under "Turn on memory".
  const memorySection = memoryProps ? (
    <WMenuSection label={t("Memory")}>
      <WMemoryControl {...memoryProps} part="state" />
    </WMenuSection>
  ) : null;
  const reasonAside = large && !!row?.problem && !!row.reason;

  return (
    <WRoot className="w-win-root">
      <main className="w-win w-pop" aria-label="Whimbrel" ref={main}>
        <header className="w-win-head">
          <WAppMark />
          {/* The product name, the same in every language. */}
          <span className="w-win-name">{"Whimbrel"}</span>
        </header>
        <div className="w-pop-pinned">
          {block?.kind === "unreadable" ? (
            <WMenuSection label={t("Recording")} labelHidden>
              <WNotice tone="warning">{t("Could not read the recording state.")}</WNotice>
              {/* Whether a recording runs is not known: Stop is still here (it does nothing when nothing records), and so is a
                  way to read again. */}
              <div className="w-pop-unreadable-actions">
                <WButton variant="primary" icon="stop" onClick={rec.stop}>
                  {t("Stop")}
                </WButton>
                <WButton onClick={retryRead}>{t("Try again")}</WButton>
              </div>
            </WMenuSection>
          ) : null}
          {row ? (
            <WMenuSection label={t("Recording")} labelHidden>
              <WRecordingRow
                state={row.state}
                // "Recording" and "Recording paused", as the tray's tooltip and menu say them; a problem is said as well, never instead.
                word={row.state === "paused" ? t("Recording paused") : row.state === "recording" ? t("Recording") : undefined}
                elapsedSeconds={elapsed}
                problem={row.problem}
                reason={row.reason}
                reasonElsewhere={large}
                actions={
                  <>
                    {row.controls.pause ? (
                      <WButton icon={row.controls.pause === "pause" ? "pause" : "play"} disabled={row.controls.pauseDisabled} onClick={row.controls.pause === "pause" ? rec.pause : rec.resume}>
                        {row.controls.pause === "pause" ? t("Pause") : t("Resume")}
                      </WButton>
                    ) : null}
                    {row.controls.stop ? (
                      <WButton icon="stop" variant="primary" disabled={row.controls.stopDisabled} onClick={rec.stop}>
                        {t("Stop")}
                      </WButton>
                    ) : null}
                  </>
                }
              />
              {large ? null : showWindowLink}
            </WMenuSection>
          ) : null}
          {memoryProps && !reasonAside ? memorySection : null}
        </div>
        <WScrollArea className="w-pop-body">
          {reasonAside ? (
            <WMenuSection label={t("Recording problem")} labelHidden>
              <p className="w-recrow-reason w-pop-aside-reason">{row!.reason}</p>
            </WMenuSection>
          ) : null}
          {row && large ? showWindowLink : null}
          {memoryProps && reasonAside ? memorySection : null}
          {memoryProps ? <WMemoryControl {...memoryProps} part="details" /> : null}
          {rows.attention.length > 0 ? (
            <WMenuSection label={t("Needs attention")}>
              {rows.attention.map((item) =>
                item === "noLibrary" ? (
                  <NoStoreBanner key={item} onOpenSettings={() => openSettings("capture")} />
                ) : item === "googleSignIn" ? (
                  <div key={item} className="w-menu-notice">
                    {/* The main window's banner says "Google sign-in expired"; its Sign in again button is on each account's row in Settings, Connectors, which is where this opens. */}
                    <WNotice tone="warning">{t("Google sign-in expired")}</WNotice>
                    <div>
                      <WButton variant="quiet" link onClick={() => openSettings("connectors")}>
                        {t("Open Settings")}
                      </WButton>
                    </div>
                  </div>
                ) : (
                  <div key={item} className="w-menu-notice">
                    <WNotice tone="warning">{t("Sync is not working.")}</WNotice>
                    <div>
                      <WButton variant="quiet" link onClick={() => openSettings("devices")}>
                        {t("Open Settings")}
                      </WButton>
                    </div>
                  </div>
                ),
              )}
            </WMenuSection>
          ) : null}
          {next ? (
            <WMenuSection label={t("Next meeting")}>
              <p className="w-menu-next">
                <span className="w-menu-next-title">{next.title}</span>
                <span className="w-menu-next-time">{timeLabel(next.startUnixMs, locale)}</span>
              </p>
              {topic ? (
                <div>
                  <WButton variant="quiet" link onClick={() => void popupOpenTopic(topic.topicId)}>
                    {topic.name}
                  </WButton>
                </div>
              ) : null}
            </WMenuSection>
          ) : null}
        </WScrollArea>
        <div className="w-win-actions w-pop-foot">
          <WButton variant="primary" onClick={() => void popupOpenMain()}>
            {t("Open Whimbrel")}
          </WButton>
          <WButton onClick={() => void popupQuit()}>{t("Quit")}</WButton>
        </div>
      </main>
    </WRoot>
  );
}
