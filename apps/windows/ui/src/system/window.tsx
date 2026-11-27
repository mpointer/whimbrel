// The small windows: the tray popup, the alerts, the still-recording prompt. One compact frame (app mark and name, a title,
// one or two lines, an optional countdown, an action row with the main action first), a recording row and a memory control
// sized for a popup. Spec: docs/ui-redesign/screens/menu-bar.md. Colors come from the skin and the system tokens.

import { useEffect, useId, useRef, useState, type ReactNode, type RefObject } from "react";
import { t } from "../l10n";
import { WButton } from "./controls";
import { flowKeyAction, type FlowKeyEvent } from "./flow";
import { WRecordingIndicator, captureLabel, type WPauseChoice, type WRecordingState } from "./recording";
import { WIcon } from "./WIcon";
import "./system.css";

/* ---- Keys ---- */

/** What a key means in a small window: Return does the main action, Escape dismisses. It is the flow page's rule
 *  (`flowKeyAction`), so Return on a focused button, link, checkbox or select presses that control, never the main
 *  action as well, and nothing happens while an input method is open or another handler took the key. */
export function windowKeyAction(e: FlowKeyEvent): "primary" | "dismiss" | null {
  const action = flowKeyAction(e);
  return action === "back" ? "dismiss" : action;
}

/**
 * What Return does in each small window. The rule: Return never starts, resumes or stops a recording from a window that
 * appears by itself, where a stray key (typing into the app behind it) must not decide that. In those windows Return only
 * presses the control that has focus, as any button does, and Escape is the safe answer (Not now, Keep as now). Return may
 * run the main action only where that action is safe: opening, raising a budget, freeing space, keeping a recording going.
 * The views read this table (`mainActionOnReturn`), and `window.test.tsx` lists every row.
 */
export type SmallWindowKind = "meetingAlert" | "stillHere" | "stillHerePaused" | "question" | "budget" | "disk" | "popup";

export const WINDOW_KEYS: Record<SmallWindowKind, { returnRunsMainAction: boolean; mainAction: string; escape: string }> = {
  // Record starts a recording.
  meetingAlert: { returnRunsMainAction: false, mainAction: "Record", escape: "Not now" },
  // Keep recording changes nothing.
  stillHere: { returnRunsMainAction: true, mainAction: "Keep recording", escape: "Keep recording" },
  // Resume starts audio again.
  stillHerePaused: { returnRunsMainAction: false, mainAction: "Resume", escape: "Keep as now" },
  question: { returnRunsMainAction: true, mainAction: "Open meeting", escape: "Dismiss" },
  budget: { returnRunsMainAction: true, mainAction: "Raise budget", escape: "Dismiss" },
  disk: { returnRunsMainAction: true, mainAction: "Free up space…", escape: "Dismiss" },
  popup: { returnRunsMainAction: true, mainAction: "Open Whimbrel", escape: "Close" },
};

/** The handler Return runs in a window: the main action where the table allows it, else none. */
export function mainActionOnReturn(kind: SmallWindowKind, action: () => void): (() => void) | undefined {
  return WINDOW_KEYS[kind].returnRunsMainAction ? action : undefined;
}

/** Listens on the window, not on an element: these windows never take focus from the app in front when they appear, so
 *  nothing inside is focused until the person clicks, and the keys still have to work then. */
export function useWindowKeys(onPrimary?: () => void, onDismiss?: () => void) {
  const latest = useRef({ onPrimary, onDismiss });
  latest.current = { onPrimary, onDismiss };
  useEffect(() => {
    const onKey = (e: globalThis.KeyboardEvent) => {
      const el = e.target as HTMLElement | null;
      const action = windowKeyAction({
        key: e.key,
        ctrlKey: e.ctrlKey,
        metaKey: e.metaKey,
        defaultPrevented: e.defaultPrevented,
        composing: e.isComposing,
        target: { tag: el?.tagName ?? "body", type: (el as HTMLInputElement | null)?.type, role: el?.getAttribute?.("role") ?? undefined },
      });
      if (action === "primary" && latest.current.onPrimary) {
        e.preventDefault();
        latest.current.onPrimary();
      } else if (action === "dismiss" && latest.current.onDismiss) {
        e.preventDefault();
        latest.current.onDismiss();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);
}

/** A small window's design size in CSS px: the logical size the shell opens it at (src-tauri/src: alert.rs `SIZE_REDESIGN`,
 *  question_alert.rs, budget_alert.rs, disk_alert.rs, meetings/still_here.rs, hotkey_panel/window.rs, tray.rs
 *  `POPUP_SIZE_REDESIGN`). With the redesign on the shell multiplies the logical size by the Windows text scale when it opens the
 *  window, so the window has this much CSS space at any text size (WebView2 divides the CSS viewport by the text scale). */
export interface WindowDesign {
  width: number;
  height: number;
}

export const WINDOW_DESIGN = {
  popup: { width: 320, height: 560 },
  alert: { width: 360, height: 230 },
  question: { width: 380, height: 210 },
  budget: { width: 380, height: 170 },
  disk: { width: 380, height: 170 },
  stillHere: { width: 380, height: 170 },
  /** "Still recording?": one more line, its countdown (`still_here.rs` `HOURLY_SIZE`). */
  stillHereHourly: { width: 380, height: 196 },
  hotkeyPanel: { width: 560, height: 360 },
  /** The in-meeting window (`meetings/meeting_bar.rs` `SIZE`). */
  meetingBar: { width: 320, height: 260 },
} as const satisfies Record<string, WindowDesign>;

/** How much smaller than its design size the window's CSS space may be before it counts as compact: the shell rounds a size to
 *  whole physical pixels, which is a fraction of a CSS pixel, and nothing but a real cap is this big. */
export const COMPACT_SLACK_PX = 8;

/** Whether a window's CSS space is smaller than its design size, so the compact layout is needed. Pure, so it is testable. With no
 *  design size (a window shown outside the shell) it is never compact. */
export function isCompactSpace(viewport: { width: number; height: number }, design?: WindowDesign): boolean {
  if (!design) return false;
  return viewport.width < design.width - COMPACT_SLACK_PX || viewport.height < design.height - COMPACT_SLACK_PX;
}

/** Whether this window has less CSS space than it was designed for: the compact layout is the fallback for a window the shell had
 *  to cap to the screen (a small screen at a large Windows text size). It is read from the space the window really has (its own
 *  inner width and height in CSS px), never from the root font size, which does not change with the text size on a real PC.
 *  A small window uses it to move what can wait (a problem's explanation) out of the pinned part, and to draw its product-name
 *  row inline; nothing the person needs to act is hidden. */
export function useCompactWindow(design?: WindowDesign): boolean {
  const read = () => typeof window !== "undefined" && isCompactSpace({ width: window.innerWidth, height: window.innerHeight }, design);
  const [compact, setCompact] = useState(read);
  useEffect(() => {
    const update = () => setCompact(read());
    update();
    window.addEventListener("resize", update);
    return () => window.removeEventListener("resize", update);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [design?.width, design?.height]);
  return compact;
}

/** The next focus position in a column of controls for an arrow key, wrapping at both ends; null for a key that is not
 *  an arrow, Home or End. Left and Up go back, Right and Down go on. */
export function arrowFocusIndex(count: number, current: number, key: string): number | null {
  if (count <= 0) return null;
  switch (key) {
    case "ArrowDown":
    case "ArrowRight":
      return current < 0 ? 0 : (current + 1) % count;
    case "ArrowUp":
    case "ArrowLeft":
      return current < 0 ? count - 1 : (current - 1 + count) % count;
    case "Home":
      return 0;
    case "End":
      return count - 1;
    default:
      return null;
  }
}

/** What an arrow-type key does when a window's arrow-key handler gets it. `onScroller` is whether the focused element is the
 *  window's scrolling body itself (it takes focus when it is clipped, so the keys can scroll it): then Up, Down, Page Up,
 *  Page Down, Home and End are the browser's own scrolling, which the handler must leave alone ("scroll"); Tab leaves it as
 *  it always did (it is not an arrow key). Everywhere else the keys move focus ("move"), and any other key is not ours (null). */
export function arrowKeyAction(onScroller: boolean, key: string): "scroll" | "move" | null {
  if (arrowFocusIndex(1, 0, key) === null && key !== "PageUp" && key !== "PageDown") return null;
  if (onScroller && ["ArrowUp", "ArrowDown", "PageUp", "PageDown", "Home", "End"].includes(key)) return "scroll";
  return key === "PageUp" || key === "PageDown" ? null : "move";
}

/** Arrow keys move through the buttons and other controls of a window, as through a menu. A text field and a select keep
 *  their own arrows. Listens on the window, like `useWindowKeys`: nothing is focused when one of these windows appears,
 *  and the first arrow key focuses the first control. */
export function useArrowKeys(container: RefObject<HTMLElement>) {
  useEffect(() => {
    const onKey = (e: globalThis.KeyboardEvent) => {
      const root = container.current;
      const el = e.target as HTMLElement | null;
      if (!root || !el || e.defaultPrevented || e.altKey || e.ctrlKey || e.metaKey) return;
      const tag = el.tagName.toLowerCase();
      if (tag === "input" || tag === "textarea" || tag === "select") return;
      if (el !== document.body && !root.contains(el)) return;
      const items = Array.from(root.querySelectorAll<HTMLElement>("button:not(:disabled), a[href], [role='menuitem']:not([aria-disabled='true'])"));
      // A focused scrolling body scrolls with these keys: leave them to the browser rather than moving focus away.
      if (arrowKeyAction(el.classList.contains("w-win-scroll"), e.key) !== "move") return;
      const next = arrowFocusIndex(items.length, items.indexOf(el), e.key);
      if (next === null) return;
      e.preventDefault();
      items[next]?.focus();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [container]);
}

/* ---- The mark ---- */

/** The small Whimbrel mark, one color. The same shape as the tray glyph (design/tray-glyph.svg). The mask's white and black only
 *  say which part is kept and which is cut out (the eye); they are not drawn. */
export function WAppMark({ size = 20 }: { size?: number }) {
  const id = useId();
  return (
    <svg className="w-mark" viewBox="6 15.5 94 94" width={`${size / 16}rem`} height={`${size / 16}rem`} aria-hidden="true" focusable="false">
      <mask id={id}>
        <rect x="0" y="0" width="120" height="120" fill="white" />
        <circle cx="68" cy="36.5" r="4.6" fill="black" />
      </mask>
      <g fill="currentColor" mask={`url(#${id})`}>
        <path d="M52,44 C50,62 52,84 55,100 L100,100 L100,74 C88,68 80,60 77,49 Z" />
        <circle cx="65" cy="40" r="15" />
        <path d="M53,32.5 C38,34 22,46 10,78 C9.2,80.2 11.2,81.4 12.6,79.6 C25,58 38,48.5 54,46.5 Z" />
      </g>
    </svg>
  );
}

/* ---- The frame ---- */

export interface WWindowFrameProps {
  /** Heading above the lines, for example the meeting's title or "Still in a meeting?". */
  title: string;
  /** One or two lines under the title. */
  lines?: ReactNode[];
  /** A sentence the person must always see (the meeting alert's "Record it? Audio stays on this PC."): pinned under the title, never in the scroller, at every text size. */
  pinned?: ReactNode;
  /** A countdown line ("Recording starts in 8 s"); its way out is the action row's dismiss button (Don't record). Not a live region: the seconds are not announced. */
  countdown?: ReactNode;
  /** The action row, the main action first. */
  actions: ReactNode;
  /** The window's design size in CSS px (`WINDOW_DESIGN`): with less space than this the compact layout is used. Left out, never compact. */
  design?: WindowDesign;
  /** Return runs this, only where the window's main action is safe (`mainActionOnReturn`); Escape runs `onDismiss`. */
  onPrimary?: () => void;
  onDismiss?: () => void;
  /** More content between the lines and the actions, for example the question alert's queue. */
  children?: ReactNode;
}

/** The lines and any extra content: the one part of a small window that scrolls. When it is clipped a line and a chevron
 *  say so (in the picture as well as on screen), and it takes focus so the keys can scroll it. The popup uses it for everything between its pinned parts. */
export function WScrollArea({ className, children }: { className?: string; children: ReactNode }) {
  const box = useRef<HTMLDivElement>(null);
  const [more, setMore] = useState(false);
  useEffect(() => {
    const el = box.current;
    if (!el) return;
    const measure = () => setMore(el.scrollHeight - el.clientHeight - el.scrollTop > 1);
    measure();
    el.addEventListener("scroll", measure);
    const watch = typeof ResizeObserver !== "undefined" ? new ResizeObserver(measure) : null;
    watch?.observe(el);
    for (const child of Array.from(el.children)) watch?.observe(child);
    return () => {
      el.removeEventListener("scroll", measure);
      watch?.disconnect();
    };
  }, [children]);
  return (
    <div className={className ? `w-win-scrollwrap ${className}` : "w-win-scrollwrap"} data-more={more ? "true" : undefined}>
      <div className="w-win-scroll" ref={box} tabIndex={more ? 0 : undefined}>
        {children}
      </div>
      {more ? (
        <span className="w-win-more" aria-hidden="true">
          <WIcon name="chevron" size={16} />
        </span>
      ) : null}
    </div>
  );
}

/** The page's name for a small window: its title, then the product name. A screen reader says the page's name when the window
 *  is opened, and "Whimbrel" alone says nothing about what is being asked. */
export function windowDocumentTitle(title: string): string {
  const name = title.trim();
  return name ? `${name} - Whimbrel` : "Whimbrel";
}

/** The accessible name and description of a small window. The name is its title; the description is its pinned sentence and
 *  lines, each ended as a sentence so they are not run together. Empty parts are left out. The countdown is not part of it, so
 *  the seconds are not read. When the shell opens the window active (a screen reader is running) focus is put on the frame, and
 *  the reader says this name and description once, as the focus change. */
export function windowDescription(parts: ReactNode[] | undefined | null): string {
  return (parts ?? [])
    .filter((part): part is string | number => typeof part === "string" || typeof part === "number")
    .map((part) => String(part).replace(/\s+/g, " ").trim())
    .filter((part) => part.length > 0)
    .map((part) => (/[.!?]$/.test(part) ? part : `${part}.`))
    .join(" ");
}

/** Names the page after the window, and puts focus on the frame itself when the window is drawn. Focus is on the frame, not on
 *  a button: nothing is pressed by a stray key, and the reader says the title and text, then the person Tabs to the buttons.
 *  With no screen reader the shell does not activate the window, so this only sets which element has focus once the person
 *  clicks or switches to it; nothing is shown or moved. Runs once per window drawn (the countdown does not move focus). */
function useWindowFocus(frame: RefObject<HTMLElement | null>, title: string) {
  useEffect(() => {
    document.title = windowDocumentTitle(title);
  }, [title]);
  useEffect(() => {
    frame.current?.focus({ preventScroll: true });
  }, [frame]);
}

/** One compact frame for every small window. The title is a heading that names the window; the lines are a polite status
 *  so a change is read once. Return and Escape follow `WINDOW_KEYS`. The header, the countdown and the action row are
 *  pinned: they never scroll away, at any text size, because the actions must always be in view (a recording can
 *  start by itself). Only the lines scroll, with a visible cue when they are clipped. */
export function WWindowFrame({ title, lines, pinned, countdown, actions, design, onPrimary, onDismiss, children }: WWindowFrameProps) {
  const id = useId();
  const main = useRef<HTMLElement>(null);
  useWindowKeys(onPrimary, onDismiss);
  useArrowKeys(main);
  useWindowFocus(main, title);
  const descriptionId = `${id}-text`;
  const description = windowDescription([pinned, ...(lines ?? [])]);
  // The product name is drawn at every text size: these windows have no title bar, and one that asks to record must say whose it
  // is. In the compact layout (the window has less CSS space than its design size) the mark and name shrink to a compact inline
  // mark beside the title (same row, capped in px), so the name costs no extra row; with its full space it is the small row above the title.
  const large = useCompactWindow(design);
  const hasBody = (lines && lines.length > 0) || children;
  return (
    <div className="w-sys w-win-root">
      <main
        className={large ? "w-win w-win-frame w-win-frame--compact" : "w-win w-win-frame"}
        role="alertdialog"
        aria-labelledby={id}
        aria-describedby={description ? descriptionId : undefined}
        tabIndex={-1}
        ref={main}
      >
        <div className={large ? "w-win-top w-win-top--inline" : "w-win-top"}>
          <header className="w-win-head">
            <WAppMark />
            {/* The product name, the same in every language. */}
            <span className="w-win-name">{"Whimbrel"}</span>
          </header>
          {/* One line, whatever the text size (the sentence under it gets the space); the full text is in the tooltip and in the page. */}
          <h1 className="w-win-title" id={id} title={title}>
            {title}
          </h1>
        </div>
        {pinned ? (
          <p className="w-win-line w-win-pinned" role="status">
            {pinned}
          </p>
        ) : null}
        {hasBody ? (
          <WScrollArea>
            {lines && lines.length > 0 ? (
              <div className="w-win-lines" role="status">
                {lines.map((line, i) => (
                  <p key={i} className="w-win-line">
                    {line}
                  </p>
                ))}
              </div>
            ) : null}
            {children}
          </WScrollArea>
        ) : null}
        {countdown ? <div className="w-win-countdown">{countdown}</div> : null}
        <div className="w-win-actions">{actions}</div>
        {/* The text a screen reader says with the title when focus arrives (the same words as on screen, without the countdown). Hidden, so it is only ever the description and is not met a second time when moving through the page. */}
        {description ? (
          <span id={descriptionId} hidden>
            {description}
          </span>
        ) : null}
      </main>
    </div>
  );
}

/* ---- A section of a menu-like window ---- */

export function WMenuSection({ label, children, labelHidden }: { label: string; children: ReactNode; labelHidden?: boolean }) {
  const id = useId();
  return (
    <section className="w-menu-section" aria-labelledby={id}>
      <h2 className={labelHidden ? "w-visually-hidden" : "w-menu-label"} id={id}>
        {label}
      </h2>
      {children}
    </section>
  );
}

/* ---- The recording row ---- */

export interface WRecordingRowProps {
  state: WRecordingState;
  elapsedSeconds?: number;
  /** A problem, said in words under the state. */
  reason?: string;
  /** The recording has a problem and `state` is still its own state (recording or paused): the row says both. */
  problem?: boolean;
  /** The visible state word, when it should say more than the shared one ("Recording paused"). */
  word?: string;
  /** With `problem`: leave the explanation out of the row (the caller shows it elsewhere); the row still says "Recording problem". */
  reasonElsewhere?: boolean;
  /** Pause or Resume, and Stop. */
  actions?: ReactNode;
}

/** The menu size of the recording indicator with its controls: the state word, the recorded time, Pause or Resume, Stop. A
 *  problem is written out under it; the state is never only a color. */
export function WRecordingRow({ state, elapsedSeconds, reason, problem, word, reasonElsewhere, actions }: WRecordingRowProps) {
  return (
    <div className={`w-recrow w-recrow-${state}${problem ? " w-recrow-has-problem" : ""}`}>
      <div className="w-recrow-main">
        <WRecordingIndicator state={state} size="menu" elapsedSeconds={elapsedSeconds} reason={reason} word={word} />
        {actions ? <span className="w-recrow-actions">{actions}</span> : null}
      </div>
      {problem ? (
        <p className="w-recrow-reason w-recrow-problem">
          <WIcon name="warning" size={16} />
          <span>
            <strong>{t("Recording problem")}</strong>
            {/* The reason on its own line under the label, never run on after it. */}
            {reason && !reasonElsewhere ? (
              <span className="w-recrow-problem-reason">{reason}</span>
            ) : null}
          </span>
        </p>
      ) : state === "problem" && reason ? (
        <p className="w-recrow-reason">{reason}</p>
      ) : null}
    </div>
  );
}

/* ---- The memory control ---- */

export interface WMemoryControlProps {
  /** "stopped": memory is on but the service could not start. "paused" is a pause the person chose, until `pausedUntil`.
   *  "pausedByApp" is a pause with no time to name (the screen is locked, say). */
  state: "on" | "paused" | "pausedByApp" | "off" | "stopped";
  /** Paused only: the clock time memory comes back, for example "3:10 PM". */
  pausedUntil?: string;
  /** The words of the state, when the caller says them (the popup says what the tray says). */
  label?: string;
  /** Quiet lines under the state: why it stopped, what sync is doing. */
  notes?: ReactNode[];
  /** Stopped only: where to go about it. */
  onOpenSettings?: () => void;
  /** The timed choices, shown while memory is on. */
  choices: WPauseChoice[];
  onPause: (id: string) => void;
  onResumeNow: () => void;
  onTurnOff: () => void;
  onTurnOn: () => void;
  onWhatIsCaptured: () => void;
  busy?: boolean;
  /** The popup pins the state line and its buttons ("state") and lets the rest scroll ("details"); default is all of it. */
  part?: "all" | "state" | "details";
}

/** What the popup's memory control shows and offers in each state. Pure, so the table is testable. */
export function memoryControls(state: WMemoryControlProps["state"]): { pause: boolean; resumeNow: boolean; turnOff: boolean; turnOn: boolean; openSettings: boolean } {
  return {
    pause: state === "on",
    resumeNow: state === "paused",
    turnOff: state === "on" || state === "paused" || state === "pausedByApp",
    turnOn: state === "off",
    openSettings: state === "stopped",
  };
}

/** Memory's state in `WCaptureState`'s words, with the choices in the open: the popup is too small for a floating menu. */
export function WMemoryControl({ state, pausedUntil, label: given, notes, onOpenSettings, choices, onPause, onResumeNow, onTurnOff, onTurnOn, onWhatIsCaptured, busy, part = "all" }: WMemoryControlProps) {
  const shows = memoryControls(state);
  const label = given ?? (state === "stopped" ? t("Memory stopped") : state === "pausedByApp" ? t("Memory paused") : captureLabel(state, pausedUntil));
  const paused = state === "paused" || state === "pausedByApp";
  const top = part !== "details";
  const details = part !== "state";
  return (
    <div className={`w-mem w-mem-${state}`}>
      {top ? (
      <div className="w-mem-top">
        <p className="w-mem-state" role="status">
          <WIcon name={paused ? "pause" : "record"} size={16} />
          <strong>{label}</strong>
        </p>
        <span className="w-mem-switch">
          {shows.resumeNow ? (
            <WButton disabled={busy} onClick={onResumeNow}>
              {t("Resume now")}
            </WButton>
          ) : null}
          {/* A real button, not plain text: it is the only control that turns memory off. */}
          {shows.turnOff ? (
            <WButton disabled={busy} onClick={onTurnOff}>
              {t("Turn off memory")}
            </WButton>
          ) : null}
          {shows.turnOn ? (
            <WButton disabled={busy} onClick={onTurnOn}>
              {t("Turn on memory")}
            </WButton>
          ) : null}
          {shows.openSettings && onOpenSettings ? (
            <WButton disabled={busy} onClick={onOpenSettings}>
              {t("Open Settings")}
            </WButton>
          ) : null}
        </span>
      </div>
      ) : null}
      {details ? (
        <>
      {notes?.map((note, i) => (
        <p key={i} className="w-mem-note">
          {note}
        </p>
      ))}
      {shows.pause ? (
        <div className="w-mem-pause" role="group" aria-label={t("Pause memory")}>
          <span className="w-mem-pause-label" aria-hidden="true">
            {t("Pause memory")}
          </span>
          <span className="w-mem-choices">
            {choices.map((c) => (
              <WButton key={c.id} disabled={busy} onClick={() => onPause(c.id)}>
                {c.label}
              </WButton>
            ))}
          </span>
        </div>
      ) : null}
      <div>
        <WButton variant="quiet" link onClick={onWhatIsCaptured}>
          {t("What is captured")}
        </WButton>
      </div>
        </>
      ) : null}
    </div>
  );
}
