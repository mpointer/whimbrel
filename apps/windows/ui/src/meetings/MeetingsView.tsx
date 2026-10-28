import { useCallback, useEffect, useState } from "react";
import { meetingList } from "../api";
import { describeError, isCommandError } from "../errors";
import type { MeetingRow } from "../generated/types";
import { DeviceChip } from "../library/LibraryView";
import { spanLabel } from "../library/rows";
import { RecordingCard } from "./RecordingCard";
import { Workspace } from "./Workspace";

function message(e: unknown): string {
  return isCommandError(e) ? describeError(e) : String(e);
}

const TRIGGER_LABEL: Record<string, string> = { calendar: "Calendar", detected: "Detected", manual: "Recorded" };

/** The Meetings tab: record, the meeting list, and the selected meeting's workspace. */
export function MeetingsView() {
  const [rows, setRows] = useState<MeetingRow[]>([]);
  const [filter, setFilter] = useState("");
  const [selected, setSelected] = useState<number | null>(null);
  const [error, setError] = useState<string | null>(null);

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

  useEffect(() => {
    load(filter);
    // A finished recording and its summary both change the list, so it refreshes.
    const timer = setInterval(() => load(filter), 5_000);
    return () => clearInterval(timer);
  }, [filter, load]);

  return (
    <div className="stack">
      <RecordingCard onChanged={() => load(filter)} />
      {error && <p role="alert">{error}</p>}
      <div className="meetings">
        <section className="card meeting-list" aria-label="Meetings">
          <input
            className="input"
            type="search"
            placeholder="Search meetings"
            value={filter}
            onChange={(e) => setFilter(e.target.value)}
          />
          {rows.length === 0 ? (
            <p className="muted">No meetings yet. Record one, and it appears here.</p>
          ) : (
            <ul className="rows">
              {rows.map((row) => (
                <li key={row.id}>
                  <button
                    className={row.id === selected ? "row-card row-card--active" : "row-card"}
                    onClick={() => setSelected(row.id)}
                    aria-current={row.id === selected}
                  >
                    <span className="row-title">{row.title}</span>
                    <span className="row-meta">
                      {spanLabel(row.tsUnixMs, row.endUnixMs)} · {TRIGGER_LABEL[row.trigger ?? "manual"] ?? "Recorded"}
                      {row.status === "recording" ? " · Recording" : ""}
                      {!row.hasSummary && row.status === "completed" ? " · No summary yet" : ""}
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
            <section className="card empty-state">Select a meeting to see its notes, summary and transcript.</section>
          ) : (
            <Workspace meetingId={selected} onChanged={() => load(filter)} />
          )}
        </div>
      </div>
    </div>
  );
}
