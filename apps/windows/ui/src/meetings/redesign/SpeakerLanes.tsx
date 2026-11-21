// Speaker lanes above the transcript (redesign flag on only): one lane per speaker, a span for each line, the marks as ticks.
// The lanes are one tab stop: arrows move between spans (left and right in time, up and down across lanes) and Enter opens the
// line. Each span is a button named by its speaker and time.

import { useRef, useState, type KeyboardEvent } from "react";
import { useL10n } from "../../l10n";
import { neighborSpan, otherLaneSpan, percent, type LaneModel } from "./lanesModel";
import { offsetClock } from "./noteModel";

export function SpeakerLanes({ model, highlightId, onOpen }: { model: LaneModel; highlightId: number | null; onOpen: (lineId: number) => void }) {
  const { t } = useL10n();
  const [current, setCurrent] = useState<number | null>(null);
  const refs = useRef(new Map<number, HTMLButtonElement>());
  const stop = current ?? highlightId ?? neighborSpan(model, null, 1);
  const go = (id: number | null) => {
    if (id == null) return;
    setCurrent(id);
    refs.current.get(id)?.focus();
  };
  const onKey = (e: KeyboardEvent) => {
    const from = current ?? stop;
    const next =
      e.key === "ArrowRight" ? neighborSpan(model, from, 1) : e.key === "ArrowLeft" ? neighborSpan(model, from, -1) : e.key === "ArrowDown" ? otherLaneSpan(model, from, 1) : e.key === "ArrowUp" ? otherLaneSpan(model, from, -1) : e.key === "Home" ? neighborSpan(model, null, 1) : e.key === "End" ? neighborSpan(model, null, -1) : undefined;
    if (next === undefined) return;
    e.preventDefault();
    go(next);
  };
  return (
    <div className="th-fin-lanes" role="group" aria-label={t("Speaker lanes")} onKeyDown={onKey}>
      {model.lanes.map((lane) => (
        <div key={lane.key} className="th-fin-lane" data-lane={model.lanes.indexOf(lane) % 5}>
          <span className="th-fin-lane-name">{lane.speaker}</span>
          <div className="th-fin-lane-track">
            {lane.spans.map((span) => (
              <button
                key={span.id}
                ref={(el) => {
                  if (el) refs.current.set(span.id, el);
                  else refs.current.delete(span.id);
                }}
                type="button"
                className="th-fin-span"
                tabIndex={span.id === stop ? 0 : -1}
                aria-current={span.id === highlightId ? "true" : undefined}
                aria-label={t("{speaker}, {time}", { speaker: span.speaker, time: span.time })}
                style={{ left: `${percent(span.startCs, model.endCs)}%`, width: `${Math.max(percent(span.endCs - span.startCs, model.endCs), 0.4)}%` }}
                onFocus={() => setCurrent(span.id)}
                onClick={() => onOpen(span.id)}
              />
            ))}
          </div>
        </div>
      ))}
      {model.ticks.length > 0 ? (
        <div className="th-fin-lane th-fin-lane-marks">
          <span className="th-fin-lane-name">{t("Moments you marked")}</span>
          <div className="th-fin-lane-track" aria-hidden="true">
            {model.ticks.map((tick) => (
              <span key={tick.id} className="th-fin-tick" style={{ left: `${percent(tick.atCs, model.endCs)}%` }} title={`${offsetClock(tick.atCs)} ${tick.label}`} />
            ))}
          </div>
        </div>
      ) : null}
      <div className="th-fin-lane th-fin-lane-axis" aria-hidden="true">
        <span className="th-fin-lane-name" />
        <div className="th-fin-lane-track">
          <span>0:00</span>
          <span>{offsetClock(model.endCs)}</span>
        </div>
      </div>
    </div>
  );
}
