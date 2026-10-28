import { useEffect, useState } from "react";
import { retentionPreview, retentionRunNow, retentionSettings } from "../api";
import { describeError, isCommandError } from "../errors";
import type { RetentionPreview, RetentionSettings } from "../generated/types";
import { noticeLines, nothingToDelete } from "./retentionModel";

function message(e: unknown): string {
  return isCommandError(e) ? describeError(e) : String(e);
}

/**
 * The first-run retention notice: Whimbrel doesn't delete anything on this PC until the
 * user has been told what the windows are. It shows what the first sweep would remove,
 * then either runs it ("Keep current settings") or sends them to Settings to widen the
 * windows first — saving there counts as an answer too, so the notice goes away.
 */
export function RetentionNotice({ onChangeSettings }: { onChangeSettings: () => void }) {
  const [state, setState] = useState<{ settings: RetentionSettings; preview: RetentionPreview } | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    retentionSettings()
      .then(async (view) => {
        if (view.acknowledged || cancelled) return;
        const preview = await retentionPreview();
        if (!cancelled) setState({ settings: view.settings, preview });
      })
      .catch((e: unknown) => !cancelled && setError(message(e)));
    return () => {
      cancelled = true;
    };
  }, []);

  // A failed count would otherwise hide the notice and leave the sweep waiting for an
  // answer nobody can see, so say so instead.
  if (!state) {
    return error ? (
      <section className="card notice">
        <p role="alert">Whimbrel could not work out what its daily cleanup would delete: {error}</p>
      </section>
    ) : null;
  }
  const lines = noticeLines(state.settings, state.preview);

  return (
    <section className="card notice" aria-labelledby="retention-notice-heading" role="region">
      <h2 id="retention-notice-heading" className="section-label">
        Before Whimbrel deletes anything
      </h2>
      <p>
        Whimbrel keeps data for a while and then deletes it, once a day. The daily sweep is waiting for your answer; here
        is what it would remove under the current settings.
      </p>
      <ul>
        {lines.map((line) => (
          <li key={line.label}>
            {line.label}: <strong>{line.count}</strong>
          </li>
        ))}
      </ul>
      {nothingToDelete(state.preview) && <p className="muted small">Nothing has aged out yet, so this first sweep removes nothing.</p>}
      <div className="toolbar">
        <button
          className="mode"
          disabled={busy}
          onClick={() => {
            setBusy(true);
            setError(null);
            retentionRunNow()
              .then(() => setState(null))
              .catch((e: unknown) => setError(message(e)))
              .finally(() => setBusy(false));
          }}
        >
          Keep current settings
        </button>
        <button className="mode" disabled={busy} onClick={onChangeSettings}>
          Change settings…
        </button>
      </div>
      {error && <p role="alert">{error}</p>}
    </section>
  );
}
