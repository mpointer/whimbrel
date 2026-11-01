import { useEffect, useState } from "react";
import { libraryCapture, libraryEpisode, libraryMeeting, setEpisodeAppHidden } from "../api";
import { describeError, isCommandError } from "../errors";
import type { CaptureDetail, EpisodeRow, MeetingDetail } from "../generated/types";
import { t, useL10n } from "../l10n";
import { meetingTitleLabel } from "../meetings/sentinels";
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
  const { locale } = useL10n();

  useEffect(() => {
    const fail = (e: unknown) => setError(isCommandError(e) ? describeError(e) : String(e));
    const missing = () => setError(t("This item no longer exists."));
    if (selection.kind === "capture")
      libraryCapture(selection.id).then((d) => (d ? setLoaded({ kind: "capture", data: d }) : missing()), fail);
    else if (selection.kind === "meeting")
      libraryMeeting(selection.id).then((d) => (d ? setLoaded({ kind: "meeting", data: d }) : missing()), fail);
    else libraryEpisode(selection.id).then((d) => (d ? setLoaded({ kind: "episode", data: d }) : missing()), fail);
  }, [selection]);

  return (
    <article className="detail">
      <button className="mode" onClick={onBack}>
        {t("← Back")}
      </button>
      {error && (
        <p role="alert" className="card">
          {error}
        </p>
      )}
      {!error && !loaded && <p className="muted">{t("Loading…")}</p>}
      {loaded?.kind === "capture" && (
        <>
          <h2 className="display">{loaded.data.title || loaded.data.app}</h2>
          <p className="row-meta muted">
            {loaded.data.app} · {fullDateTimeLabel(loaded.data.tsUnixMs, locale)} ·{" "}
            {loaded.data.source === "ocr" ? t("OCR fallback") : t("Accessibility")}{" "}
            <DeviceChip device={loaded.data.device} />
          </p>
          {loaded.data.url && <p className="mono small">{loaded.data.url}</p>}
          <pre className="card capture-text">{loaded.data.text}</pre>
        </>
      )}
      {loaded?.kind === "meeting" && (
        <>
          <h2 className="display">{loaded.data.title ? meetingTitleLabel(loaded.data.title) : t("Untitled meeting")}</h2>
          <p className="row-meta muted">
            {spanLabel(loaded.data.tsUnixMs, loaded.data.endUnixMs, { full: true, locale })} · {loaded.data.status}{" "}
            <DeviceChip device={loaded.data.device} />
          </p>
          {loaded.data.attendees.length > 0 && <p className="muted small">{t("With {names}", { names: loaded.data.attendees.join(", ") })}</p>}
          {loaded.data.summaryTldr ? (
            <section className="card">
              <h3 className="section-label">{t("Summary")}</h3>
              <p>{loaded.data.summaryTldr}</p>
              <List title={t("Decisions")} items={loaded.data.decisions} />
              <List title={t("Action items")} items={loaded.data.actionItems} />
              <List title={t("Open questions")} items={loaded.data.openQuestions} />
            </section>
          ) : (
            <p className="muted">{t("No summary yet. Recording and summaries on Windows arrive in M3.3.")}</p>
          )}
          {loaded.data.notes && (
            <section className="card">
              <h3 className="section-label">{t("Notes")}</h3>
              <pre className="capture-text">{loaded.data.notes}</pre>
            </section>
          )}
        </>
      )}
      {loaded?.kind === "episode" && (
        <>
          <h2 className="display">{loaded.data.title || loaded.data.app}</h2>
          <p className="row-meta muted">
            {loaded.data.app} · {spanLabel(loaded.data.tsUnixMs, loaded.data.endTsUnixMs, { full: true, locale })} ·{" "}
            {t("{count} captures", { count: loaded.data.itemCount })}{" "}
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
            {loaded.data.appHidden
              ? t("Show episodes from {app} again", { app: loaded.data.app })
              : t("Hide episodes from {app}", { app: loaded.data.app })}
          </button>
          <p className="muted small">{t("Hiding an app only tidies the Episodes list on this PC. Its captures stay searchable.")}</p>
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
