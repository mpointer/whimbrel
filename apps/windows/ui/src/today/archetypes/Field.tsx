import { useMemo, type ReactNode } from "react";
import {
  FIELD,
  FIELD_LINE_OPACITY,
  FIELD_STRIPE_OPACITY,
  earlierRows,
  fieldDay,
  fieldSlots,
  markerLabel,
  nonEmptySections,
  scheduleItems,
  upNext,
  voiceText,
  type EventRow,
  type FieldMarker,
} from "../layoutModel";
import { HeldLine, PrepLine, ScheduleLines, SkinImage, SkinText, applyCaps, useWidth, type TodayLayoutProps } from "./parts";

const PITCH_PADDING = 12;
const LABEL_HEIGHT = 36;
/** Space above and below a ball, so the up-next ring (3 px out) never touches a label. */
const RING_SLACK = 4;
const PITCH_HEIGHT = PITCH_PADDING * 2 + LABEL_HEIGHT * 2 + FIELD.markerDiameter + RING_SLACK * 2;

// Segoe icon glyphs for the markers: Play and CheckMark.
const PLAY = "";
const CHECK = "";

/** A pitch with today's meetings as markers along it, left to right by start time, a scoreboard for up next,
 *  and the meetings held on earlier days as a results list. Places greeting, composer, up next, schedule and
 *  earlier; the rest follows as a plain list. */
export function Field(props: TodayLayoutProps) {
  const { model, voice, caps, heading } = props;
  const present = nonEmptySections(model);
  const next = upNext(model);
  const day = useMemo(() => fieldDay(model), [model]);

  const chalk = props.dividers ? <hr className="field-chalk" aria-hidden="true" /> : null;
  const section = (id: string, title: string, rows: ReactNode) => (
    <section key={id} data-section={id} className="field-section" aria-label={title}>
      {chalk}
      <SkinText role="label" size={12} className="tone-secondary">{applyCaps(caps, title)}</SkinText>
      {rows}
    </section>
  );

  return (
    <div className="arch arch--field">
      <header className="arch-header field-header">
        <div className="arch-header__text">
          {heading.kicker && <SkinText role="label" size={12} className="tone-secondary">{applyCaps(caps, heading.kicker)}</SkinText>}
          <SkinText role="display" lines={3} size={34} as="h1" className="tone-primary arch-title">{heading.title}</SkinText>
          {props.stripes && <div className="stripes-band" aria-hidden="true" />}
        </div>
        <SkinImage imagery={props.imagery} skinId={props.skinId} placement="badge" />
      </header>
      {props.composer}
      {present.includes("upNext") && next && <Scoreboard row={next} props={props} title={applyCaps(caps, voiceText(voice, "upNextHeader"))} />}
      {present.includes("schedule") &&
        day.markers.length > 0 &&
        section(
          "schedule",
          voiceText(voice, "upcomingHeader"),
          <>
            <Pitch markers={day.markers} props={props} />
            {day.moreLabel && (
              <>
                <SkinText role="label" size={12} className="tone-secondary">{day.moreLabel}</SkinText>
                <ScheduleLines items={scheduleItems(day.overflow)} props={props} shape="grid" detail={false} />
              </>
            )}
          </>,
        )}
      {present.includes("earlier") &&
        section(
          "earlier",
          voiceText(voice, "earlierHeader"),
          earlierRows(model).map((row) => <HeldLine key={row.id} row={row} props={props} shape="grid" />),
        )}
    </div>
  );
}

/** Up next: the start time on split-flap tiles (the skin's numerals face), then the meeting and Start. */
function Scoreboard({ row, props, title }: { row: EventRow; props: TodayLayoutProps; title: string }) {
  return (
    <section data-section="upNext" className="field-scoreboard" aria-label={title}>
      <SkinText role="label" size={12} className="tone-secondary">{title}</SkinText>
      <div className="field-scoreboard__body">
        <Digits time={row.timeText} />
        <div className="field-scoreboard__main">
          <SkinText role="display" lines={2} size={20} className="tone-primary">{row.title}</SkinText>
          {row.relative && <SkinText role="label" className="tone-secondary">{row.relative}</SkinText>}
          {row.attendees && <SkinText role="label" className="tone-secondary">{row.attendees}</SkinText>}
          <PrepLine prep={row.prep} props={props} />
        </div>
        <EventAction row={row} props={props} />
      </div>
    </section>
  );
}

function EventAction({ row, props }: { row: EventRow; props: TodayLayoutProps }) {
  // The scoreboard has only its Start (up next is never recorded), so reuse the row's own button.
  if (row.action !== "start") return null;
  const label = props.starting === row.id ? "Starting…" : "Start";
  return (
    <button type="button" className="mode mode--active row-action" onClick={() => props.actions.start(row.id)} disabled={props.starting !== null}>
      {label}
    </button>
  );
}

/** "11:00 AM" as a tile per digit, the colon, and the rest ("AM") in small type. One element to a screen reader. */
function Digits({ time }: { time: string }) {
  const [clock, ...rest] = time.split(/[\s ]+/);
  return (
    <span className="field-digits" role="img" aria-label={time}>
      <span className="field-digits__tiles" aria-hidden="true">
        {[...clock].map((char, i) =>
          /\d/.test(char) ? (
            <span key={i} className="field-tile">
              {char}
            </span>
          ) : (
            <span key={i} className="field-colon">
              {char}
            </span>
          ),
        )}
      </span>
      {rest.length > 0 && (
        <span className="field-digits__rest" aria-hidden="true">
          {rest.join(" ")}
        </span>
      )}
    </span>
  );
}

/** The markings: mowing stripes, the touchlines and halfway line, the centre circle and both penalty boxes,
 *  in the palette's accent and border. Drawn, never a bitmap. */
function PitchLines({ width, height }: { width: number; height: number }) {
  const stripes = 10;
  const band = width / stripes;
  const inset = 6;
  const field = { x: inset, y: inset, w: width - inset * 2, h: height - inset * 2 };
  const boxHeight = field.h * 0.62;
  const radius = Math.min(field.h * 0.28, 34);
  const midX = field.x + field.w / 2;
  const midY = field.y + field.h / 2;
  return (
    <svg className="field-lines" width={width} height={height} viewBox={`0 0 ${width} ${height}`} aria-hidden="true" focusable="false">
      {Array.from({ length: stripes / 2 }, (_, k) => (
        <rect key={k} x={k * 2 * band} y={0} width={band} height={height} fill="var(--color-accent-primary)" fillOpacity={FIELD_STRIPE_OPACITY} />
      ))}
      <g fill="none" stroke="var(--color-border)" strokeOpacity={FIELD_LINE_OPACITY} strokeWidth={1.5}>
        <rect x={field.x} y={field.y} width={field.w} height={field.h} />
        <line x1={midX} y1={field.y} x2={midX} y2={field.y + field.h} />
        <circle cx={midX} cy={midY} r={radius} />
        <rect x={field.x} y={midY - boxHeight / 2} width={56} height={boxHeight} />
        <rect x={field.x + field.w - 56} y={midY - boxHeight / 2} width={56} height={boxHeight} />
      </g>
    </svg>
  );
}

function Pitch({ markers, props }: { markers: FieldMarker[]; props: TodayLayoutProps }) {
  const [ref, width] = useWidth<HTMLDivElement>();
  const inner = Math.max(0, width - PITCH_PADDING * 2);
  const slots = useMemo(() => fieldSlots(markers.map((m) => m.startMs), inner), [markers, inner]);
  return (
    <div ref={ref} className="field-pitch" style={{ height: PITCH_HEIGHT }} role="group" aria-label="Today's meetings, by start time">
      {width > 0 && <PitchLines width={width} height={PITCH_HEIGHT} />}
      {width > 0 &&
        markers.map((marker, i) => {
          const slot = slots[i];
          if (!slot) return null;
          const label = (
            <span className="field-label" style={{ width: slot.labelWidth, height: LABEL_HEIGHT, justifyContent: slot.row === 0 ? "flex-end" : "flex-start" }}>
              <SkinText role="numerals" size={11} className="tone-secondary">{marker.timeText}</SkinText>
              <SkinText size={12} className={marker.isHeld ? "tone-secondary" : "tone-primary"}>{marker.title}</SkinText>
            </span>
          );
          const body = (
            <>
              {slot.row === 0 && label}
              <span className={`field-ball${marker.isHeld ? " field-ball--held" : ""}${marker.isNext ? " field-ball--next" : ""}`} aria-hidden="true">
                {marker.isHeld ? CHECK : PLAY}
              </span>
              {slot.row === 1 && label}
            </>
          );
          const style = { left: PITCH_PADDING + slot.labelX, top: PITCH_PADDING + (slot.row === 0 ? 0 : LABEL_HEIGHT), width: slot.labelWidth };
          const action = marker.action;
          if (action?.kind === "start") {
            return (
              <button key={marker.id} type="button" className="field-marker" style={style} aria-label={markerLabel(marker)} disabled={props.starting !== null} onClick={() => props.actions.start(action.eventId)}>
                {body}
              </button>
            );
          }
          if (action?.kind === "open") {
            return (
              <button key={marker.id} type="button" className="field-marker" style={style} aria-label={markerLabel(marker)} onClick={() => props.actions.openMeeting(action.meetingId)}>
                {body}
              </button>
            );
          }
          return (
            <div key={marker.id} className="field-marker" style={style} role="img" aria-label={markerLabel(marker)}>
              {body}
            </div>
          );
        })}
    </div>
  );
}
