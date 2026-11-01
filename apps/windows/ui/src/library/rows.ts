import type { EpisodeRow, LibraryRow } from "../generated/types";
import { t } from "../l10n";
import { meetingTitleLabel, speakerLabel } from "../meetings/sentinels";

/**
 * An episode's summary, or why it has none. Episode summaries come only from Ollama on
 * this PC (docs/09's local batch tier), whichever provider Ask and the recap use.
 */
export function episodeSummaryText(e: Pick<EpisodeRow, "summary" | "status">): string {
  if (e.summary) return e.summary;
  if (e.status === "summarize_failed")
    return t(
      "Not summarized. Episode summaries are written by Ollama on this PC, whichever AI provider you chose for Ask. Ollama isn't running, or it doesn't have the model set in Settings → AI & routing.",
    );
  return t("Not summarized yet. Episode summaries are written by Ollama on this PC, whichever AI provider you chose for Ask.");
}

export function rowKey(row: LibraryRow): string {
  return `${row.kind}-${row.id}`;
}

/** When the row happened; transcript and slide hits have no time of their own. */
export function rowTs(row: LibraryRow): number | null {
  return row.kind === "transcript" || row.kind === "snapshot" ? null : row.tsUnixMs;
}

function startOfDay(d: Date): number {
  return new Date(d.getFullYear(), d.getMonth(), d.getDate()).getTime();
}

export function dayLabel(ms: number, now: Date = new Date(), locale?: string): string {
  const day = new Date(ms);
  const days = Math.round((startOfDay(now) - startOfDay(day)) / 86_400_000);
  if (days === 0) return t("Today");
  if (days === 1) return t("Yesterday");
  return day.toLocaleDateString(locale, {
    weekday: "short",
    month: "short",
    day: "numeric",
    year: day.getFullYear() === now.getFullYear() ? undefined : "numeric",
  });
}

/** Windows' ICU puts U+202F (narrow no-break) before AM/PM and U+2009 around range
 *  dashes; plain spaces and a bare dash read the same and keep labels predictable. */
function tidy(s: string): string {
  return s.replace(/[  ]/g, " ").replace(/\s*–\s*/g, "–");
}

/** "2:05 PM" in the viewer's locale. */
export function timeLabel(ms: number, locale?: string): string {
  return tidy(new Date(ms).toLocaleTimeString(locale, { hour: "numeric", minute: "2-digit" }));
}

/** "Thu, Sep 11", with the year when it isn't the current one. */
export function shortDate(ms: number, now: Date = new Date(), locale?: string): string {
  const d = new Date(ms);
  return tidy(
    d.toLocaleDateString(locale, {
      weekday: "short",
      month: "short",
      day: "numeric",
      year: d.getFullYear() === now.getFullYear() ? undefined : "numeric",
    }),
  );
}

/** "Thursday, September 11, 2026". */
export function longDate(ms: number, locale?: string): string {
  return tidy(new Date(ms).toLocaleDateString(locale, { weekday: "long", month: "long", day: "numeric", year: "numeric" }));
}

/** "Thu, Sep 11 · 2:05 PM": a row's date and time. */
export function dateTimeLabel(ms: number, now: Date = new Date(), locale?: string): string {
  return `${shortDate(ms, now, locale)} · ${timeLabel(ms, locale)}`;
}

/** "Thursday, September 11, 2026 · 2:05 PM": a detail page's date and time. */
export function fullDateTimeLabel(ms: number, locale?: string): string {
  return `${longDate(ms, locale)} · ${timeLabel(ms, locale)}`;
}

/** "45 min", "1 h", "1 h 5 min". */
export function durationLabel(ms: number): string {
  const minutes = Math.max(0, Math.round(ms / 60_000));
  if (minutes < 60) return t("{minutes} min", { minutes });
  const hours = Math.floor(minutes / 60);
  const rest = minutes % 60;
  return rest ? t("{hours} h {minutes} min", { hours, minutes: rest }) : t("{hours} h", { hours });
}

/**
 * A span with its date: "Thu, Sep 11 · 2:05–2:50 PM (45 min)". A span across midnight
 * dates both ends; one with no end (still recording) is just its start. `full` uses the
 * detail page's long date.
 */
export function spanLabel(
  start: number,
  end: number | null | undefined,
  opts: { full?: boolean; now?: Date; locale?: string } = {},
): string {
  const { full = false, now = new Date(), locale } = opts;
  const date = (ms: number) => (full ? longDate(ms, locale) : shortDate(ms, now, locale));
  if (end == null || end <= start) return `${date(start)} · ${timeLabel(start, locale)}`;
  const sameDay = new Date(start).toDateString() === new Date(end).toDateString();
  const range = sameDay
    ? `${date(start)} · ${tidy(new Intl.DateTimeFormat(locale, { hour: "numeric", minute: "2-digit" }).formatRange(start, end))}`
    : `${date(start)} · ${timeLabel(start, locale)} – ${date(end)} · ${timeLabel(end, locale)}`;
  return `${range} (${durationLabel(end - start)})`;
}

/** When a Library row happened, dated; transcript and slide hits have no time of their own. */
export function rowWhen(row: LibraryRow, now: Date = new Date(), locale?: string): string | null {
  switch (row.kind) {
    case "capture":
      return dateTimeLabel(row.tsUnixMs, now, locale);
    case "meeting":
      return spanLabel(row.tsUnixMs, row.endUnixMs, { now, locale });
    case "episode":
      return spanLabel(row.tsUnixMs, row.endTsUnixMs, { now, locale });
    case "transcript":
    case "snapshot":
      return null;
  }
}

/** A transcript offset in centiseconds as m:ss or h:mm:ss. */
export function formatOffset(cs: number): string {
  const total = Math.max(0, Math.floor(cs / 100));
  const h = Math.floor(total / 3600);
  const m = Math.floor((total % 3600) / 60);
  const s = String(total % 60).padStart(2, "0");
  return h > 0 ? `${h}:${String(m).padStart(2, "0")}:${s}` : `${m}:${s}`;
}

export interface Headline {
  title: string;
  meta: string;
}

export function headline(row: LibraryRow): Headline {
  switch (row.kind) {
    case "capture":
      return { title: row.title || row.app, meta: row.source === "ocr" ? t("{app} · OCR", { app: row.app }) : row.app };
    case "meeting":
      // The status is core's value ("completed", "recording"), shown as stored.
      return {
        title: row.title ? meetingTitleLabel(row.title) : t("Untitled meeting"),
        meta: row.hasSummary
          ? t("Meeting · {status} · summary", { status: row.status })
          : t("Meeting · {status}", { status: row.status }),
      };
    case "episode":
      return {
        title: row.title || row.app,
        meta:
          row.hidden || row.appHidden
            ? t("{app} · {count} captures · hidden", { app: row.app, count: row.itemCount })
            : t("{app} · {count} captures", { app: row.app, count: row.itemCount }),
      };
    case "transcript":
      return {
        title: row.meetingTitle ? meetingTitleLabel(row.meetingTitle) : t("Meeting transcript"),
        meta: `${speakerLabel(row.speaker)} · ${formatOffset(row.startCs)}`,
      };
    case "snapshot":
      return { title: row.meetingTitle ? meetingTitleLabel(row.meetingTitle) : t("Meeting slide"), meta: t("Slide") };
  }
}

export interface DayGroup {
  day: string;
  rows: LibraryRow[];
}

/** Consecutive rows grouped under a day heading, keeping their order. Transcript and
 *  slide hits, which have no time, go under "Transcript matches" and "Slide matches". */
export function groupByDay(rows: LibraryRow[], now: Date = new Date(), locale?: string): DayGroup[] {
  const groups: DayGroup[] = [];
  for (const row of rows) {
    const ts = rowTs(row);
    const day = ts !== null ? dayLabel(ts, now, locale) : row.kind === "snapshot" ? t("Slide matches") : t("Transcript matches");
    const last = groups[groups.length - 1];
    if (last && last.day === day) last.rows.push(row);
    else groups.push({ day, rows: [row] });
  }
  return groups;
}
