import { useCallback, useEffect, useState } from "react";
import { importMarkdownFolder, importTranscripts, meetingList } from "../api";
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
import { meetingTitleLabel } from "./sentinels";
import { useMeetingChips } from "./useMeetingChips";
import { Workspace } from "./Workspace";

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
export function MeetingsView({ open }: { open?: { seq: number; meetingId: number | null } | null } = {}) {
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

  const load = useCallback(
    (search: string) =>
      meetingList(search)
        .then((list) => {
          setRows(list);
          setSelected((current) => (current != null && list.some((r) => r.id === current) ? current : (list[0]?.id ?? null)));
        })
        .catch((e: unknown) => setError(message(e))),
    [],
  );

  // A question alert's Open meeting: select it, clearing a search that would hide it.
  useEffect(() => {
    if (open?.meetingId == null) return;
    setFilter("");
    setSelected(open.meetingId);
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
    if (target.kind === "meeting") setSelected(target.id);
    else if (target.kind === "brief") setPrepKey(target.key);
    else if (target.kind === "topic") openTopic(target.id);
  };

  return (
    <div className="stack">
      <RecordingCard onChanged={() => load(filter)} />
      {error && <p role="alert">{error}</p>}
      <div className="meetings">
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
            </div>
          )}
        </section>
        <div className="meeting-workspace stack">
          {prepKey && <PrepBriefPanel prepKey={prepKey} onClose={() => setPrepKey(null)} onOpenMeeting={setSelected} />}
          {selected == null ? (
            <section className="card empty-state">{t("Select a meeting to see its notes, summary and transcript.")}</section>
          ) : (
            <Workspace meetingId={selected} onChanged={() => load(filter)} />
          )}
        </div>
      </div>
    </div>
  );
}
