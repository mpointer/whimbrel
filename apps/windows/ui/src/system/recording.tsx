import { useEffect, useRef, useState, type ReactNode } from "react";
import { t } from "../l10n";
import { WButton } from "./controls";
import { formatClock, spokenDuration } from "./format";
import { WIcon } from "./WIcon";
import "./system.css";

export type WRecordingState = "recording" | "paused" | "processing" | "off" | "problem" | "starting" | "resuming";

/** The word shown for a state. Literals, so the l10n extractor finds each one. */
export function recordingWord(state: WRecordingState): string {
  switch (state) {
    case "recording":
      return t("Recording");
    case "paused":
      return t("Paused");
    case "processing":
      return t("Processing");
    case "off":
      return t("Not recording");
    case "problem":
      return t("Problem");
    case "starting":
      return t("Starting");
    case "resuming":
      return t("Resuming");
  }
}

/** The name assistive technology reads: the state, then the time or the reason. "Recording, 14 minutes 8 seconds". */
export function recordingName(state: WRecordingState, elapsedSeconds?: number, reason?: string, locale = "en"): string {
  const word = recordingWord(state);
  if ((state === "recording" || state === "paused" || state === "problem") && elapsedSeconds !== undefined) {
    return state === "problem" && reason ? `${word}, ${reason}, ${spokenDuration(elapsedSeconds, locale)}` : `${word}, ${spokenDuration(elapsedSeconds, locale)}`;
  }
  if (state === "problem") return reason ? `${word}, ${reason}` : word;
  return word;
}

/** A dot that is a drawn shape, so it stays visible when the system removes background colors. */
function Glyph({ state }: { state: WRecordingState }) {
  return (
    <svg className="w-rec-glyph" width="16" height="16" viewBox="0 0 16 16" aria-hidden="true" focusable="false">
      {state === "recording" ? (
        <>
          <circle cx="8" cy="8" r="7" fill="none" stroke="currentColor" strokeWidth="1.5" />
          <circle cx="8" cy="8" r="4" fill="currentColor" />
        </>
      ) : state === "paused" ? (
        <>
          <rect x="3.5" y="2.5" width="3" height="11" rx="1" fill="currentColor" />
          <rect x="9.5" y="2.5" width="3" height="11" rx="1" fill="currentColor" />
        </>
      ) : state === "processing" || state === "starting" || state === "resuming" ? (
        <circle cx="8" cy="8" r="6" fill="none" stroke="currentColor" strokeWidth="1.5" strokeDasharray="3 2.4" />
      ) : state === "problem" ? (
        <path d="M8 1.5l7 12.5H1zM8 6.5v3.5M8 12h.01" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round" />
      ) : (
        <circle cx="8" cy="8" r="6" fill="none" stroke="currentColor" strokeWidth="1.5" />
      )}
    </svg>
  );
}

export interface WRecordingIndicatorProps {
  state: WRecordingState;
  size?: "bar" | "pill" | "menu";
  /** Seconds since the recording started. Shown while recording or paused. */
  elapsedSeconds?: number;
  /** Problem only: a short reason. */
  reason?: string;
  /** Bar only: where Pause and Stop go. */
  actions?: ReactNode;
  /** Bar only: the level meter (`WLevelMeter`), drawn between the time and the buttons. */
  meter?: ReactNode;
}

/** State changes are announced: the word and time sit in a polite live region, named for the whole state. */
export function WRecordingIndicator({ state, size = "bar", elapsedSeconds, reason, actions, meter }: WRecordingIndicatorProps) {
  const showTime = (state === "recording" || state === "paused" || state === "problem") && elapsedSeconds !== undefined;
  return (
    <div className={`w-rec w-rec-${size} w-rec-${state}`} data-state={state}>
      <Glyph state={state} />
      <span className="w-rec-live" role="status" aria-label={recordingName(state, elapsedSeconds, reason)}>
        <span className="w-rec-word" aria-hidden="true">
          {recordingWord(state)}
        </span>
        {showTime ? (
          <span className="w-rec-time" aria-hidden="true">
            {" "}
            {formatClock(elapsedSeconds ?? 0)}
          </span>
        ) : null}
        {state === "problem" && reason ? (
          <span className="w-rec-reason" aria-hidden="true">
            {" "}
            {reason}
          </span>
        ) : null}
      </span>
      {size === "bar" && meter ? <span className="w-rec-meter">{meter}</span> : null}
      {size === "bar" && actions ? <span className="w-rec-slots">{actions}</span> : null}
    </div>
  );
}

/** Whether the system asks for less motion. False where there is no matchMedia (a test). */
function usePrefersReducedMotion(): boolean {
  const query = typeof window !== "undefined" && window.matchMedia ? window.matchMedia("(prefers-reduced-motion: reduce)") : null;
  const [reduced, setReduced] = useState(query?.matches ?? false);
  useEffect(() => {
    if (!query) return;
    const on = () => setReduced(query.matches);
    on();
    query.addEventListener("change", on);
    return () => query.removeEventListener("change", on);
  }, [query]);
  return reduced;
}

export interface WLevelMeterProps {
  /** 0 to 1, the latest reading. */
  level: number;
  /** Whether audio is arriving: the meter's name. Decided by the caller so the system part owns no threshold. */
  arriving: boolean;
  /** Draws nothing but a flat mark, for a paused recording. */
  idle?: boolean;
  /** How many readings the meter keeps; the bars drawn. */
  bars?: number;
}

const METER_BARS = 20;

/**
 * A short strip of bars that follows the recent level, drawn in the text color so it sits on the red bar and on a card.
 * It is decoration with a name ("Audio is arriving" or "No audio right now") and is never announced as it moves. With
 * Reduce Motion it is a still mark that shows only whether audio is arriving.
 */
export function WLevelMeter({ level, arriving, idle, bars = METER_BARS }: WLevelMeterProps) {
  const reduced = usePrefersReducedMotion();
  const history = useRef<number[]>([]);
  const [, redraw] = useState(0);
  useEffect(() => {
    const next = [...history.current, idle ? 0 : Math.min(1, Math.max(0, level))];
    history.current = next.slice(-bars);
    redraw((n) => n + 1);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [level, idle]);
  const name = arriving && !idle ? t("Audio is arriving") : t("No audio right now");
  const shown = reduced ? Array.from({ length: 5 }, () => (arriving && !idle ? 0.7 : 0.08)) : Array.from({ length: bars }, (_, i) => history.current[history.current.length - bars + i] ?? 0);
  return (
    <span className={reduced ? "w-meter w-meter-still" : "w-meter"} role="img" aria-label={name} data-arriving={arriving && !idle ? "true" : "false"}>
      {shown.map((v, i) => (
        <span key={i} className="w-meter-bar" style={{ height: `${Math.round(12 + v * 88)}%` }} />
      ))}
    </span>
  );
}

export interface WPauseChoice {
  id: string;
  label: string;
  /** "Back on 10:07 AM" */
  detail?: string;
}

export interface WCaptureStateProps {
  state: "on" | "paused" | "off";
  /** Paused only: the clock time memory comes back, for example "10:52 AM". */
  pausedUntil?: string;
  /** The line under the heading in the menu, for example "Saving context from your screen since 8:02 AM". */
  detail?: string;
  /** The timed choices. A slot: nothing is wired to them yet. */
  choices: WPauseChoice[];
  onChoose: (id: string) => void;
  onWhatIsCaptured: () => void;
  /** Set to control the menu from outside; otherwise it opens and closes itself. */
  open?: boolean;
  onOpenChange?: (open: boolean) => void;
}

export function captureLabel(state: "on" | "paused" | "off", pausedUntil?: string): string {
  if (state === "on") return t("Memory on");
  if (state === "off") return t("Memory off");
  return t("Paused until {time}", { time: pausedUntil ?? "" });
}

export function WCaptureState({ state, pausedUntil, detail, choices, onChoose, onWhatIsCaptured, open, onOpenChange }: WCaptureStateProps) {
  const [local, setLocal] = useState(false);
  const shown = open ?? local;
  const set = (next: boolean) => {
    setLocal(next);
    onOpenChange?.(next);
  };
  const label = captureLabel(state, pausedUntil);
  return (
    <div
      className={`w-capture w-capture-${state}`}
      onKeyDown={(e) => {
        if (e.key === "Escape" && shown) set(false);
      }}
    >
      <button type="button" className="w-capture-button" aria-haspopup="menu" aria-expanded={shown} onClick={() => set(!shown)}>
        <WIcon name={state === "paused" ? "pause" : "record"} size={16} />
        <span>{label}</span>
        <WIcon name="chevron" size={14} />
      </button>
      {shown ? (
        <div className="w-capture-menu" role="menu" aria-label={label}>
          <div className="w-capture-head">
            <strong>{state === "on" ? t("Memory is on") : state === "off" ? t("Memory is off") : t("Memory is paused")}</strong>
            {detail ? <span>{detail}</span> : null}
          </div>
          {choices.map((c) => (
            <button
              key={c.id}
              type="button"
              role="menuitem"
              className="w-menu-item"
              onClick={() => {
                onChoose(c.id);
                set(false);
              }}
            >
              <span>{c.label}</span>
              {c.detail ? <span className="w-menu-detail">{c.detail}</span> : null}
            </button>
          ))}
          <div className="w-menu-sep" role="separator" />
          <button
            type="button"
            role="menuitem"
            className="w-menu-item"
            onClick={() => {
              onWhatIsCaptured();
              set(false);
            }}
          >
            <span>{t("What is captured")}</span>
            <WIcon name="open" size={16} />
          </button>
        </div>
      ) : null}
    </div>
  );
}

export interface WConsentBannerProps {
  /** Called when Copy notice is pressed. The caller puts the notice text on the clipboard. */
  onCopy: () => void;
  onDone: () => void;
  /** Controls the "Copied" state from outside; otherwise the banner remembers the press itself. */
  copied?: boolean;
}

/** Says what to do. Never says anyone was told or agreed: the app cannot know that. */
export function WConsentBanner({ onCopy, onDone, copied }: WConsentBannerProps) {
  const [local, setLocal] = useState(false);
  const isCopied = copied ?? local;
  return (
    <section className="w-consent" aria-label={t("Recording notice")}>
      <p className="w-consent-text">
        <WIcon name="info" />
        <span>{t("You are recording. Tell the others before you go on.")}</span>
      </p>
      <div className="w-consent-actions">
        <span className="w-consent-copied" role="status">
          {isCopied ? t("Copied") : ""}
        </span>
        <WButton
          icon="copy"
          onClick={() => {
            setLocal(true);
            onCopy();
          }}
        >
          {t("Copy notice")}
        </WButton>
        <WButton variant="primary" onClick={onDone}>
          {t("Done")}
        </WButton>
      </div>
    </section>
  );
}
