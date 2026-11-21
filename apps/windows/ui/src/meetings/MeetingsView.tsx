import { useCallback, useEffect, useRef, useState } from "react";
import { importMarkdownFolder, importTranscripts, meetingList, meetingTotal } from "../api";
import { describeError, isCommandError } from "../errors";
import type { ImportView, MeetingRow } from "../generated/types";
import { t, useL10n } from "../l10n";
import { DeviceChip } from "../library/LibraryView";
import { PrepBriefPanel } from "../today/PrepBrief";
import { openTopic } from "../topics/TopicChip";
import { importSummary } from "./importSummary";
import { MeetingListRow } from "./MeetingCard";
import { forRecorded, groupByDay, type MeetingCardTarget } from "./meetingCardModel";
import { RecordingCard } from "./RecordingCard";
import { footer as pagingFooter, hasMore as pagingHasMore, nextLimit, PAGE_SIZE } from "./meetingsPaging";
import { meetingTitleLabel } from "./sentinels";
import { useMeetingChips } from "./useMeetingChips";
import { Workspace } from "./Workspace";
import { MeetingPane } from "./redesign/LiveGate";
import { useRedesign } from "../redesign";

function message(e: unknown): string {
  return isCommandError(e) ? describeError(e) : String(e);
}

/** How the meeting started, for its row; anything else (Away Mode's "away") reads as recorded. */
function triggerLabel(trigger: string | null | undefined): string {
  switch (trigger) {
    case "calendar":
      return t("Calendar");
    case "detected":
      return t("Detected");
    default:
      return t("Recorded");
  }
}

/** The Meetings tab: record, the meeting list, and the selected meeting's workspace. */
export function MeetingsView({ open, onOpenSettings }: { open?: { seq: number; meetingId: number | null } | null; onOpenSettings?: () => void } = {}) {
  const [rows, setRows] = useState<MeetingRow[]>([]);
  const [filter, setFilter] = useState("");
  const [selected, setSelected] = useState<number | null>(null);
  const [error, setError] = useState<string | null>(null);
  // What the last import did; the list itself refreshes as the summaries land.
  const [imported, setImported] = useState<string | null>(null);
  const [importing, setImporting] = useState(false);
  // A brief opened from a row's Prep chip, by its key, shown above the workspace.
  const [prepKey, setPrepKey] = useState<string | null>(null);
  const { locale } = useL10n();
  // Rows show at once; their Prep and Topic chips arrive from one lookup that reruns only when the meetings change.
  const chips = useMeetingChips(rows);
  // The hidden redesign switch: with it off nothing below differs from before.
  const redesign = useRedesign();
  // Redesign: a finished note wants the whole width, so the list and the record card give way to it until Back is pressed.
  const [noteFull, setNoteFull] = useState(false);
  const [listOpen, setListOpen] = useState(false);
  const solo = redesign && noteFull && !listOpen;
  const meetingsRef = useRef<HTMLDivElement>(null);
  const focusRow = useRef(false);
  // Back from a full-width note shows the list again and puts focus on the selected meeting's row, so the keyboard is not left on a button that went.
  const showList = useCallback(() => {
    focusRow.current = true;
    setListOpen(true);
  }, []);
  useEffect(() => {
    if (!listOpen || !focusRow.current) return;
    focusRow.current = false;
    const root = meetingsRef.current;
    (root?.querySelector<HTMLElement>('.meeting-list .meeting-item__hit[aria-current="true"]') ?? root?.querySelector<HTMLElement>(".meeting-list input"))?.focus();
  }, [listOpen]);
  // Another meeting starts from the list as it is: the last one's full-width state (and a list opened by Back) says nothing about it,
  // so a meeting still recording, or one not read yet, never makes the list flicker away.
  useEffect(() => {
    if (!redesign) return;
    setNoteFull(false);
    setListOpen(false);
  }, [selected, redesign]);

  // Pages (PR 924 on the Mac): the newest 200, then 200 more each time the end of the list is reached.
  // A search reads every meeting, so it finds the older ones too.
  const [limit, setLimit] = useState(PAGE_SIZE);
  const limitRef = useRef(PAGE_SIZE);
  // Every meeting in the store; null until counted, or when the count failed.
  const [total, setTotal] = useState<number | null>(null);
  const [loadingMore, setLoadingMore] = useState(false);
  // A bigger page that brought nothing new: a count that never matches can't keep "Show older meetings" up.
  const [exhausted, setExhausted] = useState(false);
  const loadedRef = useRef(0);
  // The next load is the one Show older meetings asked for.
  const growingRef = useRef(false);

  const load = useCallback((search: string) => {
    const asked = limitRef.current;
    return meetingList(search, asked)
      .then(async (list) => {
        if (asked !== limitRef.current) return;
        if (!search) {
          // A short page is everything; a full one asks how many there are.
          const counted = list.length < asked ? list.length : await meetingTotal().catch(() => null);
          if (asked !== limitRef.current) return;
          if (growingRef.current && list.length <= loadedRef.current) setExhausted(true);
          growingRef.current = false;
          loadedRef.current = list.length;
          setTotal(counted);
        }
        setLoadingMore(false);
        setRows(list);
        setSelected((current) => (current != null && list.some((r) => r.id === current) ? current : (list[0]?.id ?? null)));
      })
      .catch((e: unknown) => {
        setLoadingMore(false);
        setError(message(e));
      });
  }, []);

  const more = filter === "" && !exhausted && pagingHasMore(rows.length, limit, total);
  const loadMore = useCallback(() => {
    if (loadingMore) return;
    setLoadingMore(true);
    growingRef.current = true;
    limitRef.current = nextLimit(limitRef.current);
    setLimit(limitRef.current);
    void load("");
  }, [load, loadingMore]);
  // The end of the list in view loads the next page on its own; the button covers a window tall enough that it never scrolls.
  const footerRef = useRef<HTMLDivElement>(null);
  useEffect(() => {
    const el = footerRef.current;
    if (!more || loadingMore || !el || typeof IntersectionObserver === "undefined") return;
    const seen = new IntersectionObserver((entries) => {
      if (entries.some((e) => e.isIntersecting)) loadMore();
    });
    seen.observe(el);
    return () => seen.disconnect();
  }, [more, loadingMore, loadMore]);
  const footerText = filter === "" ? pagingFooter(rows.length, total, more) : null;

  // A question alert's Open meeting: select it, clearing a search that would hide it.
  useEffect(() => {
    if (open?.meetingId == null) return;
    setFilter("");
    setSelected(open.meetingId);
    // The deep link opens its meeting as a fresh pick, even when it is the one already selected.
    setListOpen(false);
  }, [open?.seq, open?.meetingId]);

  useEffect(() => {
    load(filter);
    // A finished recording and its summary both change the list, so it refreshes.
    const timer = setInterval(() => load(filter), 5_000);
    return () => clearInterval(timer);
  }, [filter, load]);

  const runImport = (pick: () => Promise<ImportView | null>) => {
    setImporting(true);
    setError(null);
    pick()
      .then((view) => {
        if (view) setImported(importSummary(view));
        return load(filter);
      })
      .catch((e: unknown) => setImported(isCommandError(e) ? describeError(e) : t("Import failed.")))
      .finally(() => setImporting(false));
  };

  // A row (or its Summary or action-items chip) selects the meeting beside the list; Prep and Topic open their own.
  const openTarget = (target: MeetingCardTarget) => {
    if (target.kind === "meeting") {
      setSelected(target.id);
      setListOpen(false);
    }
    else if (target.kind === "brief") setPrepKey(target.key);
    else if (target.kind === "topic") openTopic(target.id);
  };

  return (
    <div className="stack">
      {!solo && <RecordingCard onChanged={() => load(filter)} hideNotice={redesign} />}
      {error && <p role="alert">{error}</p>}
      <div ref={meetingsRef} className={solo ? "meetings th-fin-solo" : "meetings"}>
        <section className="card meeting-list" aria-label={t("Meetings")}>
          <input
            className="input"
            type="search"
            placeholder={t("Search meetings")}
            value={filter}
            onChange={(e) => setFilter(e.target.value)}
          />
          <details>
            <summary className="small">{t("Import")}</summary>
            <div className="toolbar">
              <button className="mode" disabled={importing} onClick={() => runImport(importTranscripts)}>
                {t("Transcripts…")}
              </button>
              <button className="mode" disabled={importing} onClick={() => runImport(importMarkdownFolder)}>
                {t("Markdown Folder…")}
              </button>
            </div>
          </details>
          {imported && (
            <p className="muted small" role="status">
              {imported}
            </p>
          )}
          {rows.length === 0 && filter !== "" ? (
            <p className="muted">{t("No meetings yet. Record one, and it appears here.")}</p>
          ) : rows.length === 0 ? (
            <div className="stack">
              <p className="row-title">{t("No meetings yet")}</p>
              <p className="muted">{t("Recorded meetings appear here. You can also bring in transcripts from Granola, Otter or files.")}</p>
              <div className="toolbar">
                <button className="mode" disabled={importing} onClick={() => runImport(importTranscripts)}>
                  {t("Import Transcripts…")}
                </button>
              </div>
            </div>
          ) : (
            <div className="meeting-days">
              {groupByDay(rows, Date.now(), locale).map((day) => (
                <section key={day.key} aria-label={day.title}>
                  <h3 className="day">{day.title}</h3>
                  <ul className="meeting-rows">
                    {day.rows.map((row) => {
                      const extra = chips.get(row.id);
                      const model = { ...forRecorded(row, extra?.prepKey, extra?.topic, locale), title: meetingTitleLabel(row.title) };
                      return (
                        <li key={row.id}>
                          <MeetingListRow
                            model={model}
                            active={row.id === selected}
                            onOpen={openTarget}
                            meta={
                              <>
                                <span>{triggerLabel(row.trigger)}</span>
                                <DeviceChip device={row.device} />
                              </>
                            }
                          />
                        </li>
                      );
                    })}
                  </ul>
                </section>
              ))}
              {(footerText || more) && (
                <div ref={footerRef} className="meeting-list-footer stack">
                  {footerText && <p className="muted small">{footerText}</p>}
                  {more && (
                    <button className="mode" disabled={loadingMore} onClick={loadMore}>
                      {t("Show older meetings")}
                    </button>
                  )}
                </div>
              )}
            </div>
          )}
        </section>
        <div className="meeting-workspace stack">
          {prepKey && <PrepBriefPanel prepKey={prepKey} onClose={() => setPrepKey(null)} onOpenMeeting={setSelected} />}
          {selected == null ? (
            <section className="card empty-state">{t("Select a meeting to see its notes, summary and transcript.")}</section>
          ) : (
            (() => {
              const workspace = <Workspace meetingId={selected} onChanged={() => load(filter)} onOpenSettings={onOpenSettings} />;
              // A meeting in progress is the live screen (redesign only).
              return redesign ? (
                <MeetingPane
                  meetingId={selected}
                  onOpenSettings={onOpenSettings}
                  onChanged={() => load(filter)}
                  onFullWidth={setNoteFull}
                  onBack={solo ? showList : undefined}
                >
                  {workspace}
                </MeetingPane>
              ) : (
                workspace
              );
            })()
          )}
        </div>
      </div>
    </div>
  );
}
