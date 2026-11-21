// The app shell in the Thread look, behind the redesign flag: the sidebar (with the Thread list on Home), the recording
// state above the header row, the header row (title, search and ask, capture state) on every section, and Ctrl+1 to
// Ctrl+6 and Ctrl+K. App.tsx decides (after all its own hooks) and hands over what it already holds. Nothing here
// loads the sections' data.

import { useEffect, useMemo, useRef, type ReactNode } from "react";
import { WCaptureState, WRoot, WSidebar, WSidebarItem, type WIconName } from "../../system";
import { ModelStatusLine } from "../../settings/ModelStatusLine";
import { SyncStoppedLine } from "../../settings/SyncStoppedLine";
import { waitingLabel } from "../../automations/automationsModel";
import { MARK_EVENT, RecordingContext } from "../../meetings/redesign/recordingContext";
import { RecordingBar } from "../../meetings/redesign/RecordingBar";
import { clockOffsetCs, pauseToggle } from "../../meetings/redesign/liveModel";
import { meetingMarkerAdd } from "../../api";
import { useLiveScreenShown, useLiveScreenTitle } from "./liveScreen";
import { useScreenTitle } from "./screenTitle";
import { t, useL10n } from "../../l10n";
import type { SearchRequest } from "../../library/searchRequest";
import { SkinHeader, TabIcon } from "../../SkinIcon";
import { timeLabel } from "../../library/rows";
import type { Tab, TabId } from "../../tabs";
import { elementsFromOptions } from "../../today/redesign/elements";
import { voiceText, localizedVoice } from "../../today/layoutModel";
import { useSkin } from "../../useSkin";
import { pauseChoiceLabel } from "./captureChoices";
import { PAUSE_CHOICES, pauseDeadline, pauseEndLabel, type PauseChoice } from "./captureModel";
import { HeaderRow } from "./HeaderRow";
import { scrollToThreadPart, useThreadParts } from "./outline";
import { liveShortcutOf, shortcutHint, shortcutOf } from "./shortcuts";
import { useCapture } from "./useCapture";
import { useRecording } from "./useRecording";
import "./shell.css";

const ICONS: Record<TabId, WIconName> = { today: "today", meetings: "meetings", library: "library", topics: "topics", automations: "automations", settings: "settings" };

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
  /** The search and ask bar: a question, a filtered search, and the "set up" action. */
  onAsk: (prompt: string) => void;
  onSearch: (request: Omit<SearchRequest, "id">) => void;
  onOpenAiSettings: () => void;
  /** The banners, the heading and the section, as the old shell lays them out inside its panel. */
  children: ReactNode;
}

export function RedesignedShell({ tabs, active, onChoose, waiting, onWhatIsCaptured, onOpenStatus, onAsk, onSearch, onOpenAiSettings, children }: RedesignedShellProps) {
  const { locale, lang } = useL10n();
  const skin = useSkin();
  const elements = elementsFromOptions(skin.skin.today.options, skin.skin.today.archetype);
  const capture = useCapture(true);
  const recording = useRecording(true);
  const liveShown = useLiveScreenShown();
  const liveTitle = useLiveScreenTitle();
  const screenTitle = useScreenTitle();
  // The key handler below reads the latest state without re-binding on every poll.
  const latest = useRef(recording);
  latest.current = recording;
  const parts = useThreadParts();
  const search = useRef<HTMLInputElement>(null);
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
      else search.current?.focus();
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
  const showBar = recording.state !== "idle" && !liveShown;
  return (
    <RecordingContext.Provider value={recording}>
    <div className="w-shell-box">
    <WRoot className="shell w-shell">
      <WSidebar
        label={t("Sections")}
        footerClass="sidebar-footer"
        items={
          <>
            {tabs.map((each, i) => (
              <WSidebarItem
                key={each.id}
                id={`tab-${each.id}`}
                icon={ICONS[each.id]}
                glyph={<TabIcon id={each.id} />}
                label={each.label}
                selected={each.id === active}
                shortcut={shortcutHint(i)}
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
        {showBar ? (
          <div className="w-shell-recording">
            <RecordingBar rec={recording} stage={recording.view?.processing?.label ?? null} />
            {recording.view?.degraded && recording.state !== "processing" ? (
              <p role="status" className="w-shell-recording-note">
                {recording.view.degraded}
              </p>
            ) : null}
          </div>
        ) : null}
        <HeaderRow
          title={liveShown && liveTitle ? liveTitle : (screenTitle ?? tab.label)}
          date={liveShown && liveTitle ? t("Now, {time}", { time: timeLabel(capture.now, locale) }) : active === "today" && !screenTitle ? date : undefined}
          capture={captureControl}
          ask={elements.ask}
          placeholder={voiceText(localizedVoice(skin.skin, lang), "askPlaceholder")}
          filters={active === "today" && !liveShown && !screenTitle}
          now={capture.now}
          inputRef={search}
          onAsk={onAsk}
          onSearch={onSearch}
          onOpenAiSettings={onOpenAiSettings}
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
