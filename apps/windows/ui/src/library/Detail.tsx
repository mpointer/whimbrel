import { useEffect, useState } from "react";
import { libraryCapture, libraryEpisode, libraryMeeting, setEpisodeAppHidden } from "../api";
import { describeError, isCommandError } from "../errors";
import type { CaptureDetail, EpisodeRow, MeetingDetail } from "../generated/types";
import { DeviceChip } from "./LibraryView";
import { episodeSummaryText, fullDateTimeLabel, spanLabel } from "./rows";

export type Selection = { kind: "capture" | "meeting" | "episode"; id: number };

type Loaded =
  | { kind: "capture"; data: CaptureDetail }
  | { kind: "meeting"; data: MeetingDetail }
  | { kind: "episode"; data: EpisodeRow };

export function Detail({ selection, onBack }: { selection: Selection; onBack: () => void }) {
  const [loaded, setLoaded] = useState<Loaded | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    const fail = (e: unknown) => setError(isCommandError(e) ? describeError(e) : String(e));
    const missing = () => setError("This item no longer exists.");
    if (selection.kind === "capture")
      libraryCapture(selection.id).then((d) => (d ? setLoaded({ kind: "capture", data: d }) : missing()), fail);
    else if (selection.kind === "meeting")
      libraryMeeting(selection.id).then((d) => (d ? setLoaded({ kind: "meeting", data: d }) : missing()), fail);
    else libraryEpisode(selection.id).then((d) => (d ? setLoaded({ kind: "episode", data: d }) : missing()), fail);
  }, [selection]);

  return (
    <article className="detail">
      <button className="mode" onClick={onBack}>
        ← Back
      </button>
      {error && (
        <p role="alert" className="card">
          {error}
        </p>
      )}
      {!error && !loaded && <p className="muted">Loading…</p>}
      {loaded?.kind === "capture" && (
        <>
          <h2 className="display">{loaded.data.title || loaded.data.app}</h2>
          <p className="row-meta muted">
            {loaded.data.app} · {fullDateTimeLabel(loaded.data.tsUnixMs)} · {loaded.data.source === "ocr" ? "OCR fallback" : "Accessibility"}{" "}
            <DeviceChip device={loaded.data.device} />
          </p>
          {loaded.data.url && <p className="mono small">{loaded.data.url}</p>}
          <pre className="card capture-text">{loaded.data.text}</pre>
        </>
      )}
      {loaded?.kind === "meeting" && (
        <>
          <h2 className="display">{loaded.data.title || "Untitled meeting"}</h2>
          <p className="row-meta muted">
            {spanLabel(loaded.data.tsUnixMs, loaded.data.endUnixMs, { full: true })} · {loaded.data.status}{" "}
            <DeviceChip device={loaded.data.device} />
          </p>
          {loaded.data.attendees.length > 0 && <p className="muted small">With {loaded.data.attendees.join(", ")}</p>}
          {loaded.data.summaryTldr ? (
            <section className="card">
              <h3 className="section-label">Summary</h3>
              <p>{loaded.data.summaryTldr}</p>
              <List title="Decisions" items={loaded.data.decisions} />
              <List title="Action items" items={loaded.data.actionItems} />
              <List title="Open questions" items={loaded.data.openQuestions} />
            </section>
          ) : (
            <p className="muted">No summary yet. Recording and summaries on Windows arrive in M3.3.</p>
          )}
          {loaded.data.notes && (
            <section className="card">
              <h3 className="section-label">Notes</h3>
              <pre className="capture-text">{loaded.data.notes}</pre>
            </section>
          )}
        </>
      )}
      {loaded?.kind === "episode" && (
        <>
          <h2 className="display">{loaded.data.title || loaded.data.app}</h2>
          <p className="row-meta muted">
            {loaded.data.app} · {spanLabel(loaded.data.tsUnixMs, loaded.data.endTsUnixMs, { full: true })} · {loaded.data.itemCount} captures{" "}
            <DeviceChip device={loaded.data.device} />
          </p>
          <div className="card">{episodeSummaryText(loaded.data)}</div>
          <button
            className="mode"
            onClick={() => {
              const { app, appHidden } = loaded.data;
              setEpisodeAppHidden(app, !appHidden).then(
                () => setLoaded({ kind: "episode", data: { ...loaded.data, appHidden: !appHidden } }),
                (e: unknown) => setError(isCommandError(e) ? describeError(e) : String(e)),
              );
            }}
          >
            {loaded.data.appHidden ? `Show episodes from ${loaded.data.app} again` : `Hide episodes from ${loaded.data.app}`}
          </button>
          <p className="muted small">Hiding an app only tidies the Episodes list on this PC. Its captures stay searchable.</p>
        </>
      )}
    </article>
  );
}

function List({ title, items }: { title: string; items: string[] }) {
  if (items.length === 0) return null;
  return (
    <>
      <h4 className="section-label">{title}</h4>
      <ul>
        {items.map((item) => (
          <li key={item}>{item}</li>
        ))}
      </ul>
    </>
  );
}
