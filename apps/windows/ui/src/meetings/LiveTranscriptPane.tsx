import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { meetingLiveTranscript } from "../api";
import type { LiveTranscriptView } from "../generated/types";
import { useL10n } from "../l10n";
import { followsNewLines, LIVE_POLL_MS, liveClock } from "./liveTranscriptModel";
import { speakerLabel } from "./sentinels";

/**
 * The live transcript while a meeting records (issue 246), the Mac's `LiveTranscriptPane`:
 * what the live transcriber has heard so far, provisional, replaced by the full transcript
 * after the meeting. Hidden while "Live transcript during meetings" is off.
 */
export function LiveTranscriptPane({ meetingId, onView }: { meetingId: number; onView?: (view: LiveTranscriptView) => void }) {
  const { t } = useL10n();
  const [view, setView] = useState<LiveTranscriptView | null>(null);
  const box = useRef<HTMLOListElement>(null);
  const follow = useRef(true);
  // The meeting Ask bar needs to know whether there are lines yet; a ref so a new callback doesn't restart the poll.
  const onViewRef = useRef(onView);
  onViewRef.current = onView;

  useEffect(() => {
    let live = true;
    const load = () =>
      meetingLiveTranscript(meetingId).then(
        (v) => {
          if (!live) return;
          if (box.current) follow.current = followsNewLines(box.current);
          setView(v);
          onViewRef.current?.(v);
        },
        () => {},
      );
    load();
    const timer = setInterval(load, LIVE_POLL_MS);
    return () => {
      live = false;
      clearInterval(timer);
    };
  }, [meetingId]);

  useLayoutEffect(() => {
    if (box.current && follow.current) box.current.scrollTop = box.current.scrollHeight;
  }, [view]);

  if (!view || !view.enabled) return null;
  return (
    <section className="field" aria-labelledby={`live-transcript-${meetingId}`}>
      <div className="field-row">
        <span id={`live-transcript-${meetingId}`} className="section-label">
          {t("Live transcript")}
        </span>
        <span className="muted small">{t("Provisional, about a minute behind")}</span>
      </div>
      {view.lines.length === 0 ? (
        <p className="muted small" role="status">
          {t("Listening. The transcript starts appearing after the first minute.")}
        </p>
      ) : (
        <ol ref={box} className="transcript" aria-live="polite">
          {view.lines.map((line) => (
            <li key={line.id}>
              <span className="transcript-time mono small">{liveClock(line.startCs)}</span>
              <strong className="small">{speakerLabel(line.speaker)}</strong>
              <span>{line.text}</span>
            </li>
          ))}
        </ol>
      )}
    </section>
  );
}
