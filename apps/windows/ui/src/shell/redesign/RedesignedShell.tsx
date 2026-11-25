// The app shell in the Thread look, behind the redesign flag: the sidebar (with the Thread list on Home), the recording
// state above the header row, the header row (title, search and ask, capture state) on every section, and Ctrl+1 to
// Ctrl+6 and Ctrl+K. App.tsx decides (after all its own hooks) and hands over what it already holds. Nothing here
// loads the sections' data.

import { useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { WCaptureState, WRoot, WSidebar, WSidebarItem, type WIconName } from "../../system";
import { ModelStatusLine } from "../../settings/ModelStatusLine";
import { SyncStoppedLine } from "../../settings/SyncStoppedLine";
import { waitingLabel } from "../../automations/automationsModel";
import { MARK_EVENT, RecordingContext } from "../../meetings/redesign/recordingContext";
import { RecordingBar } from "../../meetings/redesign/RecordingBar";
import { clockOffsetCs, degradedShown, pauseToggle } from "../../meetings/redesign/liveModel";
import { meetingMarkerAdd } from "../../api";
import { useLiveScreenShown, useLiveScreenTitle } from "./liveScreen";
import { useScreenTitle } from "./screenTitle";
import { t, useL10n } from "../../l10n";
import type { RoutePreviewView } from "../../generated/types";
import { SkinHeader, TabIcon } from "../../SkinIcon";
import { timeLabel } from "../../library/rows";
import type { Tab, TabId } from "../../tabs";
import { elementsFromOptions } from "../../today/redesign/elements";
import { useSkin } from "../../useSkin";
import { pauseChoiceLabel } from "./captureChoices";
import { PAUSE_CHOICES, pauseDeadline, pauseEndLabel, type PauseChoice } from "./captureModel";
import { HeaderRow } from "./HeaderRow";
import { scrollToThreadPart, useThreadParts } from "./outline";
import { liveShortcutOf, shortcutHint, shortcutOf } from "./shortcuts";
import { useCapture } from "./useCapture";
import { useRecording } from "./useRecording";
import { useShortViewport } from "./shortViewport";
import "../../system/restyle.css";
import "./shell.css";

const ICONS: Record<TabId, WIconName> = { today: "today", meetings: "meetings", library: "library", topics: "topics", conversations: "chat", automations: "automations", settings: "settings" };

export interface RedesignedShellProps {
  tabs: readonly Tab[];
  active: TabId;
  onChoose: (id: TabId) => void;
  /** Routine actions waiting for approval; the badge on Automations. */
  waiting: number;
  /** "What is captured": Settings, Capture. */
  onWhatIsCaptured: () => void;
  /** The sync line's Open: Settings, Status. */
  onOpenStatus: () => void;
  /** The header field's Enter and Ctrl+Enter: open the Library tab on Find (what to search or ask is in the Find store). */
  onFind: () => void;
  /** The header's Ask control (Ctrl+Enter), screen 6: true when the section answers it itself (Today, a new conversation); false leaves it to the Find page. */
  onAsk?: (query: string, shown: RoutePreviewView | null, filtered: boolean) => boolean;
  /** The banners, the heading and the section, as the old shell lays them out inside its panel. */
  children: ReactNode;
}

export function RedesignedShell({ tabs, active, onChoose, waiting, onWhatIsCaptured, onOpenStatus, onFind, onAsk, children }: RedesignedShellProps) {
  const { locale } = useL10n();
  const skin = useSkin();
  const elements = elementsFromOptions(skin.skin.today.options, skin.skin.today.archetype);
  const capture = useCapture(true);
  const recording = useRecording(true);
  const short = useShortViewport();
  const liveShown = useLiveScreenShown();
  const liveTitle = useLiveScreenTitle();
  const screenTitle = useScreenTitle();
  // The key handler below reads the latest state without re-binding on every poll.
  const latest = useRef(recording);
  latest.current = recording;
  const parts = useThreadParts();
  const search = useRef<HTMLInputElement>(null);
  // Ctrl+K: the header opens its field first (it is an icon at a middle width), then takes focus.
  const [focusTick, setFocusTick] = useState(0);
  const order = useMemo(() => tabs.map((x) => x.id), [tabs]);
  const tab = tabs.find((x) => x.id === active) ?? tabs[0];

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      // The live meeting's two keys: Ctrl+Shift+P pauses or resumes, Ctrl+Shift+M marks a moment.
      const live = liveShortcutOf(e);
      if (live) {
        e.preventDefault();
        if (live === "pause") {
          const next = pauseToggle(latest.current.state);
          if (next === "pause") latest.current.pause();
          else if (next === "resume") latest.current.resume();
        } else {
          // The live screen answers when it is showing; otherwise the shell marks with no note line to name it.
          const handled = !window.dispatchEvent(new CustomEvent(MARK_EVENT, { cancelable: true }));
          const now = latest.current;
          if (!handled && (now.state === "recording" || now.state === "problem") && now.view?.meetingId != null && now.view.startedUnixMs != null) {
            void meetingMarkerAdd(now.view.meetingId, clockOffsetCs(now.view.startedUnixMs, Date.now()), null).catch(() => {});
          }
        }
        return;
      }
      const action = shortcutOf(e, order);
      if (!action) return;
      e.preventDefault();
      if (action.kind === "section") onChoose(action.id);
      else setFocusTick((n) => n + 1);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [order, onChoose]);

  const state = capture.state;
  const choices = [
    ...(state?.kind === "paused" ? [{ id: "resume", label: t("Resume now") }] : []),
    ...(state && state.kind !== "off"
      ? PAUSE_CHOICES.map((c) => ({ id: c, label: pauseChoiceLabel(c), detail: t("Back on {time}", { time: pauseEndLabel(pauseDeadline(c, capture.now), capture.now, locale) }) }))
      : []),
  ];
  const captureControl = state ? (
    <WCaptureState
      state={state.kind}
      pausedUntil={state.kind === "paused" ? pauseEndLabel(state.resumeAt, capture.now, locale) : undefined}
      choices={choices}
      onChoose={(id) => (id === "resume" ? capture.resume() : capture.pause(id as PauseChoice))}
      onWhatIsCaptured={onWhatIsCaptured}
    />
  ) : null;

  const date = new Intl.DateTimeFormat(locale, { weekday: "long", month: "long", day: "numeric" }).format(new Date(capture.now));
  // The shell's recording bar sits above the header on every section except the live screen, which draws the same bar itself. In a
  // short window the bar is part of the pinned row everywhere, so it is drawn here on the live screen too, and the live screen's own
  // is hidden (CSS, `w-shell-box--live-pinned`): the recording state is then pinned on every screen.
  const recordingNow = recording.state !== "idle";
  const showBar = recordingNow && (short || !liveShown);
  const livePinned = short && liveShown && recordingNow;
  const recordingBar = showBar ? (
    <div className="w-shell-recording">
      <RecordingBar rec={recording} stage={recording.view?.processing?.label ?? null} />
      {degradedShown(recording.view) && recording.state !== "processing" && !liveShown ? (
        <p role="status" className="w-shell-recording-note">
          {degradedShown(recording.view)}
        </p>
      ) : null}
    </div>
  ) : null;
  return (
    <RecordingContext.Provider value={recording}>
    <div className={short ? (livePinned ? "w-shell-box w-shell-box--short w-shell-box--live-pinned" : "w-shell-box w-shell-box--short") : "w-shell-box"}>
    <WRoot className="shell w-shell">
      <WSidebar
        label={t("Sections")}
        footerClass="sidebar-footer"
        items={
          <>
            {tabs.map((each) => (
              <WSidebarItem
                key={each.id}
                id={`tab-${each.id}`}
                icon={ICONS[each.id]}
                glyph={<TabIcon id={each.id} />}
                label={each.label}
                selected={each.id === active}
                shortcut={shortcutHint(each.id)}
                badge={each.id === "automations" && waiting > 0 ? waiting : undefined}
                badgeLabel={each.id === "automations" && waiting > 0 ? waitingLabel(waiting) : undefined}
                onPress={() => onChoose(each.id)}
              />
            ))}
          </>
        }
        footer={
          <>
            <ModelStatusLine />
            <SyncStoppedLine onOpen={onOpenStatus} />
          </>
        }
        between={
          active === "today" && parts.length > 0 ? (
            <div className="w-thread" role="group" aria-label={t("Thread")}>
              <p className="w-thread-label" aria-hidden="true">
                {t("Thread")}
              </p>
              <ul>
                {parts.map((part) => (
                  <li key={part.id}>
                    <button type="button" className="w-thread-item" onClick={() => scrollToThreadPart(part.id)}>
                      <span>{part.label}</span>
                      {part.note ? <span className="w-thread-note">{part.note}</span> : null}
                    </button>
                  </li>
                ))}
              </ul>
            </div>
          ) : null
        }
      />
      <div className="stage">
        <SkinHeader />
        {short ? (
          // A short window (CSS height under 480, as the main window is at a large Windows text size): the recording state and the
          // Memory state are one compact pinned row; the title, search and filter rows scroll away with the content.
          <div className="w-shell-pinned">
            {recordingBar}
            {captureControl ? <div className="w-shell-pinned-capture">{captureControl}</div> : null}
          </div>
        ) : (
          recordingBar
        )}
        <HeaderRow
          title={liveShown && liveTitle ? liveTitle : (screenTitle ?? tab.label)}
          date={liveShown && liveTitle ? t("Now, {time}", { time: timeLabel(capture.now, locale) }) : active === "today" && !screenTitle ? date : undefined}
          capture={short ? null : captureControl}
          ask={elements.ask}
          placeholder={t("Search your work memory")}
          filters={active === "today" && !liveShown && !screenTitle}
          now={capture.now}
          inputRef={search}
          focusTick={focusTick}
          onFind={onFind}
          onAsk={onAsk}
        />
        <main id="panel" aria-label={tab.label} className="content w-content">
          {children}
        </main>
      </div>
    </WRoot>
    </div>
    </RecordingContext.Provider>
  );
}
