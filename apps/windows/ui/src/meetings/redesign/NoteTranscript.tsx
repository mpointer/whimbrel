// The transcript view of the finished note (redesign flag on only): speaker lanes, then the lines. Every stored line is read once
// (a page at a time, they are small) because the lanes need them all; the rows are drawn in a window that grows as the person scrolls,
// so a long meeting never puts thousands of rows on the page. A line is opened by id (a source, a span, an Ask citation) or by time
// (a mark, a live alert): it scrolls into view and is highlighted. A jump from outside the view (a source, a mark, an alert, an Ask
// citation, Ctrl+T) also moves focus: onto the line, or onto the view's heading when there is no line. The line is then read by
// focus alone; a jump that leaves focus where it is (a lane span) is said once in a polite live region instead.

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { confirmMeetingSpeaker, meetingTranscriptLines, setMeetingSpeaker } from "../../api";
import { describeError, isCommandError } from "../../errors";
import type { InviteeView, MeetingMarkerView, TranscriptLineView, TranscriptTurn } from "../../generated/types";
import { useL10n } from "../../l10n";
import { WButton, WTranscriptLine, WTranscriptView } from "../../system";
import { SpeakerMenu } from "../SpeakerMenu";
import { buildLanes, growRange, rangeAround, rangeHolding, ROW_CHUNK, type RowRange } from "./lanesModel";
import { lineSpeaker, offsetClock, resolveTarget, type LineTarget } from "./noteModel";
import { SpeakerLanes } from "./SpeakerLanes";

/** Lines asked for at a time while reading the whole transcript. */
const PAGE = 1000;

const message = (e: unknown): string => (isCommandError(e) ? describeError(e) : String(e));

/** The speaker menu reads a turn; a line has the same facts. */
function turnOf(line: TranscriptLineView): TranscriptTurn {
  return { guess: line.guess, label: line.label, personKey: line.personKey, speaker: line.speaker, speakerId: line.speakerId, text: line.text, timestamp: offsetClock(line.startCs) };
}

export interface TranscriptOpen {
  /** Bumped for each request, so asking for the same line twice scrolls twice. */
  seq: number;
  /** The line or moment to open; null for the view as a whole. */
  target: LineTarget | null;
  /** Move focus to the line (or the heading when there is none). Off when the person pressed the view switch, which keeps focus. */
  focus: boolean;
}

export function NoteTranscript({
  meetingId,
  hasTranscript,
  marks,
  open,
  onNamesChanged,
  reloadKey,
}: {
  meetingId: number;
  hasTranscript: boolean;
  marks: readonly MeetingMarkerView[];
  open: TranscriptOpen | null;
  onNamesChanged: () => void;
  /** Changes when the transcript may have changed under the view (a run finished, a trim): the lines are read again. */
  reloadKey: number;
}) {
  const { t } = useL10n();
  const [lines, setLines] = useState<TranscriptLineView[] | null>(null);
  const [canName, setCanName] = useState(false);
  const [invitees, setInvitees] = useState<InviteeView[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [round, setRound] = useState(0);
  const [range, setRange] = useState<RowRange>({ from: 0, to: ROW_CHUNK });
  const [highlight, setHighlight] = useState<number | null>(null);
  const [announce, setAnnounce] = useState("");
  const [copied, setCopied] = useState(false);
  const seen = useRef(0);
  const flip = useRef(false);
  const heading = useRef<HTMLHeadingElement>(null);
  const top = useRef<HTMLLIElement>(null);
  const bottom = useRef<HTMLLIElement>(null);

  // Read every line, a page at a time. A rename reads them again, so the names change in every row and lane.
  useEffect(() => {
    let alive = true;
    (async () => {
      const all: TranscriptLineView[] = [];
      let first = true;
      for (let offset = 0; ; offset += PAGE) {
        const page = await meetingTranscriptLines(meetingId, offset, PAGE);
        if (!alive) return;
        all.push(...page.lines);
        if (first) {
          setCanName(page.canName);
          setInvitees(page.invitees);
          first = false;
        }
        if (page.lines.length === 0 || all.length >= page.total) break;
      }
      setLines(all);
    })().catch((e: unknown) => alive && setError(message(e)));
    return () => {
      alive = false;
    };
  }, [meetingId, round, reloadKey]);

  const loaded = useMemo(() => lines ?? [], [lines]);
  // The lanes are rebuilt only when the lines, the marks or the language change, not on every render of the page.
  const markFallback = t("Marked moment");
  const lanes = useMemo(() => buildLanes(loaded, marks, (m) => m.label ?? markFallback), [loaded, marks, markFallback]);

  // Highlights a line, makes sure its row is drawn and scrolls to it after this render has drawn the row. With `focus` the line
  // takes focus (a reader hears it from that); without, it is said once in the live region.
  const reveal = (id: number, all: TranscriptLineView[], focus: boolean) => {
    const index = all.findIndex((l) => l.id === id);
    if (index < 0) return;
    const line = all[index];
    setHighlight(id);
    setRange((r) => rangeHolding(r, index, all.length));
    if (!focus) {
      // The same words twice in a row are not announced again by most readers, so every other message ends in a no-break space.
      flip.current = !flip.current;
      setAnnounce(t("Line at {time}, {speaker}: {text}", { time: offsetClock(line.startCs), speaker: lineSpeaker(line), text: line.text }) + (flip.current ? "\u00a0" : ""));
    }
    requestAnimationFrame(() => {
      const calm = window.matchMedia?.("(prefers-reduced-motion: reduce)").matches;
      const row = document.querySelector<HTMLElement>(`[data-line-id="${id}"]`);
      row?.scrollIntoView({ block: "center", behavior: calm ? "auto" : "smooth" });
      if (focus) row?.focus({ preventScroll: true });
    });
  };

  // A request to open the view or a line, once the lines are read (a request with no line needs none).
  useEffect(() => {
    if (!open || open.seq === seen.current) return;
    if (open.target == null) {
      seen.current = open.seq;
      if (open.focus) heading.current?.focus();
      return;
    }
    if (!lines) return;
    seen.current = open.seq;
    const id = resolveTarget(lines, open.target);
    setHighlight(id);
    if (id != null) reveal(id, lines, open.focus);
    else if (open.focus) heading.current?.focus();
    // `reveal` only reads state setters and the language.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open, lines]);

  // The window grows when either end scrolls into view.
  const total = loaded.length;
  useEffect(() => {
    if (!("IntersectionObserver" in window)) return;
    const watch = new IntersectionObserver((entries) => {
      for (const entry of entries) {
        if (!entry.isIntersecting) continue;
        setRange((r) => growRange(r, total, entry.target === top.current ? "up" : "down"));
      }
    });
    if (top.current) watch.observe(top.current);
    if (bottom.current) watch.observe(bottom.current);
    return () => watch.disconnect();
  }, [total, range.from, range.to]);

  const rename = (change: Promise<unknown>) => {
    setError(null);
    change.then(
      () => {
        setRound((n) => n + 1);
        onNamesChanged();
      },
      (e: unknown) => setError(message(e)),
    );
  };

  const copy = useCallback(() => {
    const text = loaded.map((l) => `${offsetClock(l.startCs)} ${lineSpeaker(l)}: ${l.text}`).join("\n");
    navigator.clipboard.writeText(text).then(
      () => {
        setCopied(true);
        setTimeout(() => setCopied(false), 2000);
      },
      (e: unknown) => setError(message(e)),
    );
  }, [loaded]);

  const rows = loaded.slice(range.from, range.to);
  const reset = () => setRange(rangeAround(null, total));
  const head = (
    <div className="th-fin-transcript-head">
      <h3 className="th-fin-transcript-title" tabIndex={-1} ref={heading}>
        {t("Transcript")}
      </h3>
      {lines && lines.length > 0 ? (
        <>
          <WButton icon="copy" onClick={copy}>
            {copied ? t("Copied") : t("Copy transcript")}
          </WButton>
          {range.from > 0 || range.to < total ? (
            <WButton variant="quiet" onClick={reset}>
              {t("Back to the start")}
            </WButton>
          ) : null}
        </>
      ) : null}
    </div>
  );
  if (!lines || lines.length === 0) {
    return (
      <section className="th-fin-transcript" aria-label={t("Transcript")}>
        {head}
        {error && !lines ? (
          <p role="alert">{error}</p>
        ) : !lines ? (
          <p className="th-fin-quiet" role="status">
            {t("Loading…")}
          </p>
        ) : (
          <p className="th-fin-quiet">{hasTranscript ? t("The transcript file is empty.") : t("This meeting hasn't been transcribed.")}</p>
        )}
      </section>
    );
  }
  return (
    <section className="th-fin-transcript" aria-label={t("Transcript")}>
      {head}
      <SpeakerLanes
        model={lanes}
        highlightId={highlight}
        onOpen={(id) => reveal(id, lines, false)}
      />
      <p className="w-visually-hidden" role="status" aria-live="polite">
        {announce}
      </p>
      <WTranscriptView label={t("Transcript")}>
        {range.from > 0 ? <li ref={top} className="th-fin-more" aria-hidden="true" style={{ minHeight: "1px" }} /> : null}
        {rows.map((line) => (
          <WTranscriptLine
            key={line.id}
            lineId={line.id}
            time={offsetClock(line.startCs)}
            speaker={lineSpeaker(line)}
            text={line.text}
            state={line.id === highlight ? "cited" : "normal"}
            speakerSlot={
              canName && line.speakerId != null ? (
                <SpeakerMenu
                  turn={turnOf(line)}
                  invitees={invitees}
                  offered
                  onConfirm={(personKey) => line.speakerId != null && rename(confirmMeetingSpeaker(meetingId, line.speakerId, personKey))}
                  onPick={(name, email) => line.speakerId != null && rename(setMeetingSpeaker(meetingId, line.speakerId, name, email))}
                />
              ) : undefined
            }
          />
        ))}
        {range.to < total ? <li ref={bottom} className="th-fin-more" aria-hidden="true" style={{ minHeight: "1px" }} /> : null}
      </WTranscriptView>
      {error ? <p role="alert">{error}</p> : null}
    </section>
  );
}
