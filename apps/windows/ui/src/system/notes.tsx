import type { ReactNode } from "react";
import { t } from "../l10n";
import { WSpeakerChip } from "./controls";
import { WIcon, type WIconName } from "./WIcon";
import "./system.css";

/* ---- Transcript ---- */

export interface WTranscriptLineProps {
  time: string;
  speaker: string;
  text: string;
  state?: "normal" | "cited" | "live";
  /** The line's id in the data, so a screen can scroll to it. */
  lineId?: number;
  /** Replaces the speaker chip, for a speaker the person can name. */
  speakerSlot?: ReactNode;
}

export function WTranscriptLine({ time, speaker, text, state = "normal", lineId, speakerSlot }: WTranscriptLineProps) {
  return (
    <li
      className={state === "normal" ? "w-transcript-line" : `w-transcript-line w-transcript-${state}`}
      aria-current={state === "cited" ? "true" : undefined}
      data-line-id={lineId}
      // A line a screen can scroll to can also take focus (not a tab stop), so a jump lands the reader on it.
      tabIndex={lineId != null ? -1 : undefined}
    >
      <span className="w-transcript-time">{time}</span>
      {speakerSlot ?? <WSpeakerChip name={speaker} size="small" />}
      <p className="w-transcript-text">{text}</p>
      {state === "live" ? <span className="w-listening">{t("Listening")}</span> : null}
      {state === "cited" ? <span className="w-visually-hidden">{t("Cited by a note")}</span> : null}
    </li>
  );
}

export function WTranscriptView({ label, children }: { label: string; children: ReactNode }) {
  return (
    <ol className="w-transcript" aria-label={label}>
      {children}
    </ol>
  );
}

/* ---- Source mark and card ---- */

export interface WSourceMarkProps {
  number: number;
  selected?: boolean;
  /** Called with the mark's number, so the caller selects the matching source. */
  onSelect: (number: number) => void;
  /** The mark's name for assistive tech when "Source 3" is not enough ("Source 3, Jo, 18:25"). */
  label?: string;
}

export function WSourceMark({ number, selected, onSelect, label }: WSourceMarkProps) {
  return (
    <button
      type="button"
      className="w-mark"
      aria-pressed={!!selected}
      aria-label={label ?? t("Source {number}", { number })}
      onClick={() => onSelect(number)}
    >
      {number}
    </button>
  );
}

export interface WSourceCardProps {
  number: number;
  appName: string;
  /** Left out in the compact margin card. */
  appIcon?: WIconName;
  /** "Transcript moment" or "Seen on screen"; left out in the compact margin card. */
  kind?: string;
  title?: string;
  /** "18:25" */
  time: string;
  quote?: string;
  size?: "margin" | "card";
  selected?: boolean;
  /** Words on the Open button. Defaults to "Open". */
  openLabel?: string;
  onOpen: () => void;
}

export function WSourceCard(p: WSourceCardProps) {
  const size = p.size ?? "margin";
  return (
    <article
      className={`w-source w-source-${size}${p.selected ? " w-source-selected" : ""}`}
      aria-label={t("Source {number}", { number: p.number })}
      aria-current={p.selected ? "true" : undefined}
      data-source={p.number}
    >
      <div className="w-source-head">
        <span className="w-source-num" aria-hidden="true">
          {p.number}
        </span>
        {p.appIcon ? <WIcon name={p.appIcon} size={16} /> : null}
        <span className="w-source-name">{p.appName}</span>
        <span className="w-source-time">{p.time}</span>
      </div>
      {p.kind ? <span className="w-source-kind">{p.kind}</span> : null}
      {p.title ? <span className="w-source-title">{p.title}</span> : null}
      {p.quote ? <q className="w-source-quote">{p.quote}</q> : null}
      <button type="button" className="w-source-open" onClick={p.onOpen}>
        <WIcon name="open" size={16} />
        <span>{p.openLabel ?? t("Open")}</span>
      </button>
    </article>
  );
}

/* ---- Note block ---- */

export interface WNoteBlockProps {
  kind: "heading" | "paragraph" | "bullet" | "check";
  author: "you" | "whimbrel";
  text: string;
  /** Whimbrel lines only: the numbers of the sources behind the line. */
  sources?: number[];
  selected?: boolean;
  /** Which source mark is selected, if any. */
  selectedSource?: number | null;
  onSelectSource?: (number: number) => void;
  /** Each mark's name for assistive tech, by number. */
  sourceLabels?: Record<number, string>;
  /** Check blocks only. */
  checked?: boolean;
  onCheck?: (next: boolean) => void;
  /** Whether a screen reader is told "Written by Whimbrel" with this block. Defaults to yes for text, no for a heading; a screen
   *  that says it once for a section turns it off on the lines and on in the section's heading. */
  announceAi?: boolean;
}

export function WNoteBlock(p: WNoteBlockProps) {
  const ai = p.author === "whimbrel";
  const cls = ["w-note-block", `w-note-${p.kind}`, ai ? "w-note-ai" : "", p.selected ? "w-note-selected" : ""].filter(Boolean).join(" ");
  const marks = ai
    ? (p.sources ?? []).map((n) => <WSourceMark key={n} number={n} label={p.sourceLabels?.[n]} selected={p.selectedSource === n} onSelect={(x) => p.onSelectSource?.(x)} />)
    : null;
  const say = ai && (p.announceAi ?? p.kind !== "heading");
  if (p.kind === "heading") {
    return (
      <h3 className={cls}>
        <span className="w-note-text">{p.text}</span>
        {say ? <span className="w-visually-hidden">{". "}{t("Written by Whimbrel")}</span> : null}
      </h3>
    );
  }
  return (
    <div className={cls} data-author={p.author}>
      {p.kind === "check" ? (
        <input type="checkbox" checked={!!p.checked} aria-label={p.text} onChange={(e) => p.onCheck?.(e.target.checked)} />
      ) : null}
      <p className="w-note-text">
        {say ? <span className="w-visually-hidden">{t("Written by Whimbrel")}{". "}</span> : null}
        {p.text}
        {marks}
      </p>
    </div>
  );
}

/* ---- Action item ---- */

export interface WActionItemRowProps {
  text: string;
  owner?: string;
  /** "Due Thu, Nov 19" */
  due?: string;
  /** "28:50": the moment in the transcript it came from. */
  sourceTime?: string;
  done: boolean;
  onToggle: (next: boolean) => void;
  /** The box cannot be changed here (another device's meeting). */
  disabled?: boolean;
  onOpenSource?: () => void;
  /** A line Whimbrel wrote: reading serif in the AI ink, with its numbered source marks. */
  sources?: number[];
  selectedSource?: number | null;
  onSelectSource?: (number: number) => void;
  sourceLabels?: Record<number, string>;
  /** The row's element: a list item (the default) or a div, for a screen that keeps the rows in one list of its own. */
  as?: "li" | "div";
  /** Tell a screen reader "Written by Whimbrel" with this row (a Whimbrel line only). Off by default. */
  announceAi?: boolean;
}

/** Every control is drawn at rest. Nothing waits for hover. */
export function WActionItemRow(p: WActionItemRowProps) {
  const Row = p.as ?? "li";
  return (
    <Row className={p.done ? "w-action w-action-done" : "w-action"}>
      <label className="w-action-check">
        <input type="checkbox" checked={p.done} disabled={p.disabled} aria-label={p.text} onChange={(e) => p.onToggle(e.target.checked)} />
      </label>
      <span className={p.sources ? "w-action-text w-action-ai" : "w-action-text"}>
        {p.sources && p.announceAi ? <span className="w-visually-hidden">{t("Written by Whimbrel")}{". "}</span> : null}
        {p.text}
        {(p.sources ?? []).map((n) => (
          <WSourceMark key={n} number={n} label={p.sourceLabels?.[n]} selected={p.selectedSource === n} onSelect={(x) => p.onSelectSource?.(x)} />
        ))}
      </span>
      <span className="w-action-meta">
        {p.owner ? <WSpeakerChip name={p.owner} size="small" /> : null}
        {p.due ? <span>{p.due}</span> : null}
        {p.sourceTime ? (
          <button type="button" className="w-time-chip" aria-label={t("Open source at {time}", { time: p.sourceTime })} onClick={p.onOpenSource}>
            {p.sourceTime}
          </button>
        ) : null}
      </span>
    </Row>
  );
}
