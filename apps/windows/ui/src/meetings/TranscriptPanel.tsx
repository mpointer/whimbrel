import { useState } from "react";
import { meetingTranscript } from "../api";
import { describeError, isCommandError } from "../errors";
import type { TranscriptView } from "../generated/types";

function message(e: unknown): string {
  return isCommandError(e) ? describeError(e) : String(e);
}

/** The meeting's transcript, loaded on demand: it can be thousands of lines. */
export function TranscriptPanel({ meetingId, hasTranscript }: { meetingId: number; hasTranscript: boolean }) {
  const [view, setView] = useState<TranscriptView | null>(null);
  const [open, setOpen] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const toggle = () => {
    if (open) {
      setOpen(false);
      return;
    }
    setOpen(true);
    if (view) return;
    meetingTranscript(meetingId)
      .then(setView)
      .catch((e: unknown) => setError(message(e)));
  };

  return (
    <div className="field">
      <div className="field-row">
        <span>Transcript</span>
        <button className="mode" onClick={toggle} aria-expanded={open}>
          {open ? "Hide" : "Show"}
        </button>
      </div>
      {open && !view && !error && <p className="muted small">Loading…</p>}
      {open && view && view.turns.length === 0 && (
        <p className="muted small">
          {hasTranscript ? "The transcript file is empty." : "This meeting hasn't been transcribed."}
        </p>
      )}
      {open && view && view.turns.length > 0 && (
        <>
          <ol className="transcript">
            {view.turns.map((turn, i) => (
              <li key={i}>
                <span className="transcript-time mono small">{turn.timestamp}</span>
                <span className="transcript-speaker">{turn.speaker}</span>
                <span>{turn.text}</span>
              </li>
            ))}
          </ol>
          {view.more && <p className="muted small">Showing the first part of a long transcript.</p>}
        </>
      )}
      {error && <p role="alert">{error}</p>}
    </div>
  );
}
