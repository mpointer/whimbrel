import { useEffect, useRef, useState } from "react";
import {
  meetingSetKeepSnapshots,
  meetingSnapshotCopy,
  meetingSnapshotDelete,
  meetingSnapshotOpen,
  meetingSnapshots,
  meetingSnapshotsShow,
  meetingSnapshotThumbnail,
} from "../api";
import { describeError, isCommandError } from "../errors";
import type { SnapshotsView, SnapshotView, Thumbnail } from "../generated/types";
import { prunedThumbnails, slidesLabel, snapshotKey, thumbnailsToFetch, withoutSnapshot } from "./workspaceModel";

/** How long "Copied" stays up. */
const NOTICE_MS = 2_000;

function message(e: unknown): string {
  return isCommandError(e) ? describeError(e) : String(e);
}

/**
 * The workspace's "Slides (N)" strip (plan Task 7): one thumbnail per slide the snapshot
 * service kept, live while the meeting records. Hidden when there are none. `refresh`
 * is the workspace's 2 s poll, so a new slide or a build shows up without a second timer.
 */
export function SlidesStrip({ meetingId, refresh }: { meetingId: number; refresh: number }) {
  const [view, setView] = useState<SnapshotsView | null>(null);
  const [thumbnails, setThumbnails] = useState<Map<string, Thumbnail>>(new Map());
  const [notice, setNotice] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  // Keys already fetched or on their way, so a slow thumbnail isn't asked for twice.
  const requested = useRef(new Set<string>());

  useEffect(() => {
    setView(null);
    setThumbnails(new Map());
    requested.current = new Set();
    setError(null);
  }, [meetingId]);

  useEffect(() => {
    meetingSnapshots(meetingId)
      .then(setView)
      .catch((e: unknown) => setError(message(e)));
  }, [meetingId, refresh]);

  useEffect(() => {
    if (!view) return;
    setThumbnails((cache) => prunedThumbnails(cache, view.snapshots));
    const live = new Set(view.snapshots.map(snapshotKey));
    requested.current = new Set([...requested.current].filter((key) => live.has(key)));
    for (const snapshot of thumbnailsToFetch(view.snapshots, requested.current)) {
      const key = snapshotKey(snapshot);
      requested.current.add(key);
      meetingSnapshotThumbnail(meetingId, snapshot.id)
        .then((thumbnail) => setThumbnails((cache) => new Map(cache).set(key, thumbnail)))
        // A missing thumbnail leaves the placeholder; the slide still opens.
        .catch(() => {});
    }
  }, [meetingId, view]);

  useEffect(() => {
    if (!notice) return;
    const timer = setTimeout(() => setNotice(null), NOTICE_MS);
    return () => clearTimeout(timer);
  }, [notice]);

  if (!view || view.snapshots.length === 0) return null;

  const run = (work: () => Promise<unknown>, done?: string) => {
    setError(null);
    work()
      .then(() => done && setNotice(done))
      .catch((e: unknown) => setError(message(e)));
  };

  const setKeep = (keep: boolean) => {
    setView({ ...view, keep });
    meetingSetKeepSnapshots(meetingId, keep).catch((e: unknown) => {
      setView({ ...view });
      setError(message(e));
    });
  };

  const remove = (snapshot: SnapshotView) => {
    setView(withoutSnapshot(view, snapshot.id));
    meetingSnapshotDelete(meetingId, snapshot.id).catch((e: unknown) => setError(message(e)));
  };

  return (
    <section className="slides" aria-label="Slides">
      <div className="slides-header">
        <h4 className="section-label">{slidesLabel(view)}</h4>
        <label className="toggle small">
          <input type="checkbox" checked={view.keep} onChange={(e) => setKeep(e.target.checked)} />
          Keep snapshots
        </label>
        <button className="mode small" onClick={() => run(() => meetingSnapshotsShow(meetingId, null))}>
          Show in Explorer
        </button>
      </div>
      <ul className="slides-row">
        {view.snapshots.map((snapshot) => {
          const thumbnail = thumbnails.get(snapshotKey(snapshot));
          return (
            <li key={snapshot.id} className="slide">
              <button
                className="slide-image"
                title={snapshot.text || snapshot.fileName}
                aria-label={`Open slide ${snapshot.fileName}`}
                onClick={() => run(() => meetingSnapshotOpen(meetingId, snapshot.id))}
              >
                {thumbnail ? (
                  <img src={thumbnail.dataUrl} width={thumbnail.width} height={thumbnail.height} alt="" />
                ) : (
                  <span className="slide-placeholder" style={{ aspectRatio: `${snapshot.width} / ${snapshot.height}` }} />
                )}
              </button>
              <div className="slide-actions">
                <button
                  className="mode small"
                  aria-label={`Copy slide ${snapshot.fileName}`}
                  onClick={() => run(() => meetingSnapshotCopy(meetingId, snapshot.id), "Copied")}
                >
                  Copy image
                </button>
                <button
                  className="mode small"
                  aria-label={`Show slide ${snapshot.fileName} in Explorer`}
                  onClick={() => run(() => meetingSnapshotsShow(meetingId, snapshot.id))}
                >
                  Show
                </button>
                <button className="mode small" aria-label={`Delete slide ${snapshot.fileName}`} onClick={() => remove(snapshot)}>
                  Delete
                </button>
              </div>
            </li>
          );
        })}
      </ul>
      {notice && <p role="status" className="muted small">{notice}</p>}
      {error && <p role="alert">{error}</p>}
    </section>
  );
}
