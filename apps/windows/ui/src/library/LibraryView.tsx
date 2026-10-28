import { useCallback, useEffect, useState } from "react";
import { libraryEpisodes, libraryRecent, librarySearch, librarySettings, setEpisodeAppHidden, setShowScreenCaptures } from "../api";
import { AskView } from "../chat/AskView";
import type { AskRequest } from "../chat/chatModel";
import { DigestView } from "../today/DigestView";
import { describeError, isCommandError } from "../errors";
import type { DeviceTag, LibraryRow } from "../generated/types";
import { Detail, type Selection } from "./Detail";
import { fullDateTimeLabel, groupByDay, headline, rowKey, rowTs, rowWhen } from "./rows";

type Mode = "recent" | "episodes" | "keyword" | "semantic" | "ask" | "digest";

const MODES: { id: Mode; label: string; comingIn?: string }[] = [
  { id: "recent", label: "Recent" },
  { id: "episodes", label: "Episodes" },
  { id: "keyword", label: "Keyword" },
  { id: "semantic", label: "Semantic" },
  { id: "ask", label: "Ask" },
  { id: "digest", label: "Digest" },
];

function message(e: unknown): string {
  return isCommandError(e) ? describeError(e) : String(e);
}

export function DeviceChip({ device }: { device: DeviceTag }) {
  return (
    <span className={device.thisDevice ? "chip chip--this" : "chip"} title={device.deviceId}>
      {device.label}
    </span>
  );
}

export function LibraryView({ askRequest, onAskHandled }: { askRequest?: AskRequest | null; onAskHandled?: () => void }) {
  // Opens on Ask when Today handed it a question (this view mounts fresh when the tab is shown).
  const [mode, setMode] = useState<Mode>(askRequest ? "ask" : "recent");
  const [rows, setRows] = useState<LibraryRow[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [query, setQuery] = useState("");
  const [includeHidden, setIncludeHidden] = useState(false);
  const [showCaptures, setShowCaptures] = useState(true);
  const [hiddenApps, setHiddenApps] = useState<string[]>([]);
  const [selected, setSelected] = useState<Selection | null>(null);

  const run = useCallback((load: () => Promise<LibraryRow[]>) => {
    setLoading(true);
    setError(null);
    load()
      .then(setRows)
      .catch((e: unknown) => setError(message(e)))
      .finally(() => setLoading(false));
  }, []);

  const loadSettings = useCallback(() => {
    librarySettings()
      .then((s) => {
        setShowCaptures(s.showScreenCaptures);
        setHiddenApps(s.hiddenEpisodeApps);
      })
      .catch(() => {});
  }, []);

  useEffect(loadSettings, [loadSettings]);

  const unhideApp = (app: string) =>
    setEpisodeAppHidden(app, false)
      .then((s) => {
        setHiddenApps(s.hiddenEpisodeApps);
        run(() => libraryEpisodes(includeHidden));
      })
      .catch((e: unknown) => setError(message(e)));

  // A detail pane can hide an app, so coming back re-reads the list and the settings.
  const back = () => {
    setSelected(null);
    loadSettings();
    if (mode === "episodes") run(() => libraryEpisodes(includeHidden));
  };

  useEffect(() => {
    setSelected(null);
    if (mode === "recent") run(libraryRecent);
    else if (mode === "episodes") run(() => libraryEpisodes(includeHidden));
    else setRows([]);
  }, [mode, includeHidden, run]);

  const toggleCaptures = (show: boolean) =>
    setShowScreenCaptures(show)
      .then((s) => {
        setShowCaptures(s.showScreenCaptures);
        run(libraryRecent);
      })
      .catch((e: unknown) => setError(message(e)));

  const search = (e: React.FormEvent) => {
    e.preventDefault();
    run(() => librarySearch(query, mode === "semantic"));
  };

  const current = MODES.find((m) => m.id === mode)!;

  if (selected) return <Detail selection={selected} onBack={back} />;

  return (
    <div className="library">
      <div className="toolbar">
        <div className="mode-picker" role="group" aria-label="Library view">
          {MODES.map((m) => (
            <button key={m.id} className={m.id === mode ? "mode mode--active" : "mode"} aria-pressed={m.id === mode} onClick={() => setMode(m.id)}>
              {m.label}
            </button>
          ))}
        </div>
        {mode === "recent" && (
          <>
            <label className="toggle">
              <input type="checkbox" checked={showCaptures} onChange={(e) => toggleCaptures(e.target.checked)} />
              Show screen captures
            </label>
            <button className="mode" onClick={() => run(libraryRecent)}>
              Refresh
            </button>
          </>
        )}
        {mode === "episodes" && (
          <label className="toggle">
            <input type="checkbox" checked={includeHidden} onChange={(e) => setIncludeHidden(e.target.checked)} />
            Show hidden episodes
          </label>
        )}
        {mode === "episodes" && hiddenApps.length > 0 && (
          <span className="row-meta small muted" aria-label="Hidden apps">
            Hidden apps:
            {hiddenApps.map((app) => (
              <button key={app} className="chip citation" title="Show this app's episodes again" onClick={() => unhideApp(app)}>
                {app} ×
              </button>
            ))}
          </span>
        )}
      </div>

      {(mode === "keyword" || mode === "semantic") && (
        <form className="search" onSubmit={search} role="search">
          <input
            type="search"
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            placeholder={mode === "keyword" ? "Words that appear in a capture or transcript" : "Describe what you're looking for"}
            aria-label={`${current.label} search`}
            autoFocus
          />
          <button type="submit" className="mode mode--active">
            Search
          </button>
        </form>
      )}
      {mode === "semantic" && (
        <p className="muted small">Semantic search needs the search model from Settings → Semantic search. Until it is downloaded it finds nothing.</p>
      )}

      {current.comingIn ? (
        <div className="card empty-state">Coming in {current.comingIn}.</div>
      ) : mode === "ask" ? (
        <AskView request={askRequest} onRequestHandled={onAskHandled} />
      ) : mode === "digest" ? (
        <DigestView />
      ) : error ? (
        <div role="alert" className="card">
          {error}
        </div>
      ) : loading ? (
        <p className="muted">Loading…</p>
      ) : rows.length === 0 ? (
        <div className="card empty-state">
          {mode === "recent"
            ? "Nothing captured yet. Captures appear here as you work, from this PC and, once sync is on, your other devices."
            : mode === "episodes"
              ? "No episodes yet. They form from a few minutes of captures in one app."
              : "No matches."}
        </div>
      ) : (
        groupByDay(rows).map((group) => (
          <section key={group.day} aria-label={group.day}>
            <h3 className="day">{group.day}</h3>
            <ul className="rows">
              {group.rows.map((row) => {
                const h = headline(row);
                const ts = rowTs(row);
                return (
                  <li key={rowKey(row)}>
                    <button
                      className="row-card"
                      onClick={() =>
                        setSelected(row.kind === "transcript" ? { kind: "meeting", id: row.meetingId } : { kind: row.kind, id: row.id })
                      }
                    >
                      <span className="row-top">
                        <span className="row-title">{h.title}</span>
                        {ts !== null && (
                          <time className="muted small" dateTime={new Date(ts).toISOString()} title={fullDateTimeLabel(ts)}>
                            {rowWhen(row)}
                          </time>
                        )}
                      </span>
                      <span className="row-meta muted small">
                        {h.meta}
                        {"device" in row && <DeviceChip device={row.device} />}
                      </span>
                      {"snippet" in row && row.snippet && <span className="row-snippet">{row.snippet}</span>}
                      {row.kind === "episode" && row.summary && <span className="row-snippet">{row.summary}</span>}
                    </button>
                  </li>
                );
              })}
            </ul>
          </section>
        ))
      )}
    </div>
  );
}
