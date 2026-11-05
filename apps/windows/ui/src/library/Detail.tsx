import { useEffect, useState } from "react";
import { libraryCapture, libraryEpisode, meetingWorkspace, setEpisodeAppHidden } from "../api";
import { describeError, isCommandError } from "../errors";
import type { CaptureDetail, EpisodeRow } from "../generated/types";
import { t, useL10n } from "../l10n";
import { Workspace } from "../meetings/Workspace";
import { DeviceChip } from "./LibraryView";
import { episodeSummaryText, fullDateTimeLabel, spanLabel } from "./rows";

export type Selection = { kind: "capture" | "meeting" | "episode"; id: number };

type Loaded =
  | { kind: "capture"; data: CaptureDetail }
  | { kind: "episode"; data: EpisodeRow };

export function Detail({ selection, onBack }: { selection: Selection; onBack: () => void }) {
  const [loaded, setLoaded] = useState<Loaded | null>(null);
  const [error, setError] = useState<string | null>(null);
  const { locale } = useL10n();

  useEffect(() => {
    // A meeting opens its workspace, which loads itself.
    if (selection.kind === "meeting") return;
    const fail = (e: unknown) => setError(isCommandError(e) ? describeError(e) : String(e));
    const missing = () => setError(t("This item no longer exists."));
    if (selection.kind === "capture")
      libraryCapture(selection.id).then((d) => (d ? setLoaded({ kind: "capture", data: d }) : missing()), fail);
    else libraryEpisode(selection.id).then((d) => (d ? setLoaded({ kind: "episode", data: d }) : missing()), fail);
  }, [selection]);

  // The Meetings tab's view: its stage, why it has no transcript (with Transcribe again), the summary.
  if (selection.kind === "meeting")
    return (
      <article className="detail">
        <button className="mode" onClick={onBack}>
          {t("← Back")}
        </button>
        {/* The list behind reloads on Back; a forgotten meeting goes back to it. */}
        <Workspace
          meetingId={selection.id}
          onChanged={() => meetingWorkspace(selection.id).then((view) => view ?? onBack(), () => {})}
        />
      </article>
    );

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
