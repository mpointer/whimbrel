import { useEffect, useState } from "react";
import { confirmMeetingSpeaker, meetingTranscript, setMeetingSpeaker } from "../api";
import { describeError, isCommandError } from "../errors";
import type { TranscriptView } from "../generated/types";
import { useL10n } from "../l10n";
import { SpeakerMenu } from "./SpeakerMenu";

function message(e: unknown): string {
  return isCommandError(e) ? describeError(e) : String(e);
}

/**
 * The meeting's transcript, loaded on demand: it can be thousands of lines. Speakers show
 * their names (a guess as "Name?"); a diarized speaker's menu confirms or fixes the name for
 * all of their lines, and `onNamesChanged` then lets the page offer a new summary.
 */
export function TranscriptPanel({
  meetingId,
  hasTranscript,
  onNamesChanged,
  openRequest = 0,
}: {
  meetingId: number;
  hasTranscript: boolean;
  onNamesChanged?: () => void;
  /** Bumped to open the panel from outside (a meeting Ask citation). */
  openRequest?: number;
}) {
  const [view, setView] = useState<TranscriptView | null>(null);
  const [open, setOpen] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const { t } = useL10n();

  const show = () => {
    setOpen(true);
    if (view) return;
    meetingTranscript(meetingId)
      .then(setView)
      .catch((e: unknown) => setError(message(e)));
  };

  const toggle = () => (open ? setOpen(false) : show());

  useEffect(() => {
    if (openRequest > 0 && !open) show();
    // Only a new request opens it; `show` and `open` change on every toggle.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [openRequest]);

  const rename = (change: Promise<TranscriptView>) => {
    setError(null);
    change
      .then((next) => {
        setView(next);
        onNamesChanged?.();
      })
      .catch((e: unknown) => setError(message(e)));
  };

  return (
    <div className="field">
      <div className="field-row">
        <span>{t("Transcript")}</span>
        <button className="mode" onClick={toggle} aria-expanded={open}>
          {open ? t("Hide") : t("Show")}
        </button>
      </div>
      {open && !view && !error && <p className="muted small">{t("Loading…")}</p>}
      {open && view && view.turns.length === 0 && (
        <p className="muted small">
          {hasTranscript ? t("The transcript file is empty.") : t("This meeting hasn't been transcribed.")}
        </p>
      )}
      {open && view && view.turns.length > 0 && (
        <>
          <ol className="transcript">
            {view.turns.map((turn, i) => {
              const speakerId = turn.speakerId;
              return (
                <li key={i}>
                  <span className="transcript-time mono small">{turn.timestamp}</span>
                  <SpeakerMenu
                    turn={turn}
                    invitees={view.invitees}
                    offered={view.canName && speakerId != null}
                    onConfirm={(personKey) => {
                      if (speakerId != null) rename(confirmMeetingSpeaker(meetingId, speakerId, personKey));
                    }}
                    onPick={(name, email) => {
                      if (speakerId != null) rename(setMeetingSpeaker(meetingId, speakerId, name, email));
                    }}
                  />
                  <span>{turn.text}</span>
                </li>
              );
            })}
          </ol>
          {view.more && <p className="muted small">{t("Showing the first part of a long transcript.")}</p>}
        </>
      )}
      {error && <p role="alert">{error}</p>}
    </div>
  );
}
