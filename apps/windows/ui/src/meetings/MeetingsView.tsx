import { useCallback, useEffect, useState } from "react";
import { importMarkdownFolder, importTranscripts, meetingList } from "../api";
import { describeError, isCommandError } from "../errors";
import type { ImportView, MeetingRow } from "../generated/types";
import { t, useL10n } from "../l10n";
import { DeviceChip } from "../library/LibraryView";
import { spanLabel } from "../library/rows";
import { importSummary } from "./importSummary";
import { RecordingCard } from "./RecordingCard";
import { meetingTitleLabel } from "./sentinels";
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
  const { locale } = useL10n();

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
            <ul className="rows">
              {rows.map((row) => (
                <li key={row.id}>
                  <button
                    className={row.id === selected ? "row-card row-card--active" : "row-card"}
                    onClick={() => setSelected(row.id)}
                    aria-current={row.id === selected}
                  >
                    <span className="row-title">{meetingTitleLabel(row.title)}</span>
                    <span className="row-meta">
                      {spanLabel(row.tsUnixMs, row.endUnixMs, { locale })} · {triggerLabel(row.trigger)}
                      {row.status === "recording" ? ` · ${t("Recording")}` : ""}
                      {!row.hasSummary && row.status === "completed" ? ` · ${t("No summary yet")}` : ""}
                    </span>
                    <DeviceChip device={row.device} />
                  </button>
                </li>
              ))}
            </ul>
          )}
        </section>
        <div className="meeting-workspace">
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
