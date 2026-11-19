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
}

export function WTranscriptLine({ time, speaker, text, state = "normal" }: WTranscriptLineProps) {
  return (
    <li
      className={state === "normal" ? "w-transcript-line" : `w-transcript-line w-transcript-${state}`}
      aria-current={state === "cited" ? "true" : undefined}
    >
      <span className="w-transcript-time">{time}</span>
      <WSpeakerChip name={speaker} size="small" />
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
}

export function WSourceMark({ number, selected, onSelect }: WSourceMarkProps) {
  return (
    <button
      type="button"
      className="w-mark"
      aria-pressed={!!selected}
      aria-label={t("Source {number}", { number })}
      onClick={() => onSelect(number)}
    >
      {number}
    </button>
  );
}

export interface WSourceCardProps {
  number: number;
  appName: string;
  appIcon: WIconName;
  /** "Transcript moment" or "Seen on screen" */
  kind: string;
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
        <WIcon name={p.appIcon} size={16} />
        <span className="w-source-name">{p.appName}</span>
        <span className="w-source-time">{p.time}</span>
      </div>
      <span className="w-source-kind">{p.kind}</span>
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
  /** Check blocks only. */
  checked?: boolean;
  onCheck?: (next: boolean) => void;
}

export function WNoteBlock(p: WNoteBlockProps) {
  const ai = p.author === "whimbrel";
  const cls = ["w-note-block", `w-note-${p.kind}`, ai ? "w-note-ai" : "", p.selected ? "w-note-selected" : ""].filter(Boolean).join(" ");
  const marks = ai
    ? (p.sources ?? []).map((n) => <WSourceMark key={n} number={n} selected={p.selectedSource === n} onSelect={(x) => p.onSelectSource?.(x)} />)
    : null;
  if (p.kind === "heading") {
    return (
      <h3 className={cls}>
        <span className="w-note-text">{p.text}</span>
      </h3>
    );
  }
  return (
    <div className={cls} data-author={p.author}>
      {p.kind === "check" ? (
        <input type="checkbox" checked={!!p.checked} aria-label={p.text} onChange={(e) => p.onCheck?.(e.target.checked)} />
      ) : null}
      <p className="w-note-text">
        {ai ? <span className="w-visually-hidden">{t("Written by Whimbrel")}{". "}</span> : null}
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
  onOpenSource?: () => void;
  /** A line Whimbrel wrote: reading serif in the AI ink, with its numbered source marks. */
  sources?: number[];
  selectedSource?: number | null;
  onSelectSource?: (number: number) => void;
}

/** Every control is drawn at rest. Nothing waits for hover. */
export function WActionItemRow(p: WActionItemRowProps) {
  return (
    <li className={p.done ? "w-action w-action-done" : "w-action"}>
      <label className="w-action-check">
        <input type="checkbox" checked={p.done} aria-label={p.text} onChange={(e) => p.onToggle(e.target.checked)} />
      </label>
      <span className={p.sources ? "w-action-text w-action-ai" : "w-action-text"}>
        {p.text}
        {(p.sources ?? []).map((n) => (
          <WSourceMark key={n} number={n} selected={p.selectedSource === n} onSelect={(x) => p.onSelectSource?.(x)} />
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
    </li>
  );
}
