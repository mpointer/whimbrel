// The pieces the six layouts share: text in the skin's faces, event and meeting rows, the prep line, the
// recap and the devices footer. Each layout places and styles them; none re-derives what a row shows.

import { useEffect, useRef, useState, type CSSProperties, type ReactNode } from "react";
import { skinImage } from "../../api";
import { DeviceChip } from "../../library/LibraryView";
import { t } from "../../l10n";
import { MeetingChips } from "../../meetings/MeetingCard";
import type { MeetingCardTarget, MeetingChip } from "../../meetings/meetingCardModel";
import {
  devicesLine,
  heldSeparator,
  thisDeviceLabel,
  type EventRow,
  type HeldRow,
  type Imagery,
  type ImagePlacement,
  type RecapModel,
  type ScheduleItem,
  type TodayModel,
} from "../layoutModel";

export interface TodayActions {
  start(eventId: string): void;
  openMeeting(meetingId: number): void;
  retryRecap(): void;
  toggleBrief(): void;
  /** A Prep chip: the brief stored under this key. */
  showPrepBrief(key: string): void;
  /** A topic chip: the topic's page. */
  openTopic(topicId: number): void;
}

/** A row's chips (Prep ready, Summary, action items, topic), routed through the layout's actions. */
export function RowChips({ chips, card, props }: { chips: MeetingChip[]; card: MeetingCardTarget; props: TodayLayoutProps }) {
  const open = (target: MeetingCardTarget) => {
    if (target.kind === "meeting") props.actions.openMeeting(target.id);
    else if (target.kind === "brief") props.actions.showPrepBrief(target.key);
    else if (target.kind === "topic") props.actions.openTopic(target.id);
  };
  return <MeetingChips chips={chips} card={card} onOpen={open} />;
}

const hasPrepChip = (chips: MeetingChip[]) => chips.some((c) => c.kind === "prep");

/** What every layout is given: Today's model plus the skin's voice, imagery and the live pieces (the composer
 *  and the next meeting's brief) that only TodayView can build. */
export interface TodayLayoutProps {
  model: TodayModel;
  voice: Record<string, string>;
  imagery: Imagery[];
  skinId: string;
  caps: string;
  composer: ReactNode;
  actions: TodayActions;
  /** The event whose Start is in flight, so its button reads "Starting…". */
  starting: string | null;
  brief: { open: boolean; card: ReactNode };
  /** Whether the skin asks for a stripes band under the greeting (Field). */
  stripes: boolean;
  /** The skin draws dividers between sections. */
  dividers: boolean;
  /** The skin's motion is `subtle`, which lets the Terminal cursor blink. */
  motion: boolean;
  /** The greeting in the skin's voice: the date over "Good morning, Mike", or the date alone with no name. */
  heading: { kicker: string | null; title: string };
}

/** Upper-cases a heading when the skin asks for it (`caps: upper`); small caps is drawn by the font. */
export const applyCaps = (caps: string, text: string): string => (caps === "upper" ? text.toUpperCase() : text);

// --- text -------------------------------------------------------------------------------------

export type TextRole = "display" | "body" | "numerals" | "label";

/**
 * Text in one of the skin's four faces. A line that does not fit is cut with an ellipsis and its full text
 * is in the tooltip, so nothing overruns its box. `lines` is how many lines it may take, or "max" for body
 * text that is never cut.
 */
export function SkinText({
  children,
  role = "body",
  lines = 1,
  size,
  smallCaps,
  className,
  as: Tag = "span",
}: {
  children: ReactNode;
  role?: TextRole;
  lines?: number | "max";
  size?: number;
  smallCaps?: boolean;
  className?: string;
  as?: "span" | "p" | "h1" | "h2" | "h3";
}) {
  const style = { "--lines": lines === "max" ? undefined : lines, fontSize: size ? `${size}px` : undefined, fontVariantCaps: smallCaps ? "small-caps" : undefined } as CSSProperties;
  const classes = ["skin-text", `skin-text--${role}`, lines === "max" ? "" : "skin-text--clamp", className ?? ""].filter(Boolean).join(" ");
  return (
    <Tag className={classes} style={style} title={typeof children === "string" && lines !== "max" ? children : undefined}>
      {children}
    </Tag>
  );
}

/** An action as a bracketed link: `[Open]`. The brackets are decoration; the accessible name is the label. */
export function BracketLink({ label, onClick, disabled, primary }: { label: string; onClick: () => void; disabled?: boolean; primary?: boolean }) {
  return (
    <button type="button" className={primary ? "bracket-link bracket-link--primary" : "bracket-link"} onClick={onClick} disabled={disabled} aria-label={label}>
      <span aria-hidden="true">[</span>
      <span className="bracket-link__label" aria-hidden="true">
        {label}
      </span>
      <span aria-hidden="true">]</span>
    </button>
  );
}

// --- the prep line ----------------------------------------------------------------------------

/** The next event's prep state: "No prep yet", or "Prep ready" with Open brief / Hide brief and the brief.
 *  With `chip` the row already shows a Prep ready chip, so only the toggle is drawn. */
export function PrepLine({ prep, props, bracket, chip }: { prep: "ready" | "none" | null; props: TodayLayoutProps; bracket?: boolean; chip?: boolean }) {
  if (prep === null) return null;
  if (prep === "none") return <SkinText role="label" className="tone-secondary">{t("No prep yet")}</SkinText>;
  const label = props.brief.open ? t("Hide brief") : t("Open brief");
  return (
    <>
      <span className="prep-line">
        {!chip && <SkinText role="label" className="tone-secondary">{t("Prep ready")}</SkinText>}
        {bracket ? (
          <BracketLink label={label} onClick={props.actions.toggleBrief} />
        ) : (
          <button type="button" className="link" onClick={props.actions.toggleBrief} aria-expanded={props.brief.open}>
            {label}
          </button>
        )}
      </span>
      {props.brief.open && <div className="prep-card">{props.brief.card}</div>}
    </>
  );
}

// --- rows -------------------------------------------------------------------------------------

export type RowShape = "grid" | "stack" | "inline";

function StartOrOpen({ row, props, bracket }: { row: EventRow; props: TodayLayoutProps; bracket?: boolean }) {
  if (row.action === "start") {
    const busy = props.starting !== null;
    const label = props.starting === row.id ? t("Starting…") : t("Start");
    return bracket ? (
      <BracketLink label={label} primary onClick={() => props.actions.start(row.id)} disabled={busy} />
    ) : (
      <button type="button" className="mode mode--active row-action" onClick={() => props.actions.start(row.id)} disabled={busy}>
        {label}
      </button>
    );
  }
  if (row.action === "open" && row.recordedMeetingId != null) {
    const id = row.recordedMeetingId;
    return bracket ? (
      <BracketLink label={t("Open")} onClick={() => props.actions.openMeeting(id)} />
    ) : (
      <button type="button" className="mode row-action" onClick={() => props.actions.openMeeting(id)}>
        {t("Open")}
      </button>
    );
  }
  return null;
}

/** One calendar event. `detail: false` leaves out the countdown and prep (a pitch's overflow list). */
export function EventLine({
  row,
  props,
  shape,
  accentTime,
  detail = true,
  bracket,
}: {
  row: EventRow;
  props: TodayLayoutProps;
  shape: RowShape;
  accentTime?: boolean;
  detail?: boolean;
  bracket?: boolean;
}) {
  const time = <SkinText role="numerals" className={accentTime ? "tone-accent line-time" : "tone-secondary line-time"}>{row.timeText}</SkinText>;
  const title = <SkinText lines={2} className="tone-primary">{row.title}</SkinText>;
  const extras = (
    <>
      {detail && row.relative && <SkinText role="label" className="tone-secondary">{row.relative}</SkinText>}
      {row.attendees && <SkinText role="label" className="tone-secondary">{row.attendees}</SkinText>}
      <RowChips chips={row.chips} card={row.card} props={props} />
      {detail && <PrepLine prep={row.prep} props={props} bracket={bracket} chip={hasPrepChip(row.chips)} />}
    </>
  );
  const action = <StartOrOpen row={row} props={props} bracket={bracket} />;
  if (shape === "stack") {
    return (
      <div className="line line--stack">
        {time}
        {title}
        {extras}
        {action}
      </div>
    );
  }
  if (shape === "inline") {
    return (
      <div className="line line--inline">
        <span className="line-head">
          {time}
          {title}
        </span>
        {extras}
        {action}
      </div>
    );
  }
  return (
    <div className="line line--grid">
      {time}
      <span className="line-main">
        {title}
        {extras}
      </span>
      {action}
    </div>
  );
}

/** A meeting already held. */
export function HeldLine({ row, props, shape, bracket }: { row: HeldRow; props: TodayLayoutProps; shape: RowShape; bracket?: boolean }) {
  const time = <SkinText role="numerals" className="tone-secondary line-time">{row.timeText}</SkinText>;
  const title = <SkinText lines={2} className="tone-primary">{row.title}</SkinText>;
  const sub = (
    <>
      {row.subtitle ? <SkinText role="label" className="tone-secondary">{row.subtitle}</SkinText> : null}
      <RowChips chips={row.chips} card={row.card} props={props} />
    </>
  );
  const action = bracket ? (
    <BracketLink label={t("Open")} onClick={() => props.actions.openMeeting(row.id)} />
  ) : (
    <button type="button" className="mode row-action" onClick={() => props.actions.openMeeting(row.id)}>
      {t("Open")}
    </button>
  );
  if (shape === "stack") {
    return (
      <div className="line line--stack">
        {time}
        {title}
        {sub}
        {action}
      </div>
    );
  }
  if (shape === "inline") {
    return (
      <div className="line line--inline">
        <span className="line-head">
          {time}
          {title}
        </span>
        {sub}
        {action}
      </div>
    );
  }
  return (
    <div className="line line--grid">
      {time}
      <span className="line-main">
        {title}
        {sub}
      </span>
      {action}
    </div>
  );
}

/** A schedule: events, the "Earlier today" label, then held meetings. */
export function ScheduleLines({
  items,
  props,
  shape,
  bracket,
  accentTime,
  detail = true,
}: {
  items: ScheduleItem[];
  props: TodayLayoutProps;
  shape: RowShape;
  bracket?: boolean;
  accentTime?: boolean;
  detail?: boolean;
}) {
  return (
    <>
      {items.map((item) => {
        switch (item.kind) {
          case "event":
            return <EventLine key={`e-${item.row.id}`} row={item.row} props={props} shape={shape} bracket={bracket} accentTime={accentTime} detail={detail} />;
          case "separator":
            return (
              <SkinText key="sep" role="label" className="tone-secondary line-separator">
                {heldSeparator()}
              </SkinText>
            );
          case "held":
            return <HeldLine key={`h-${item.row.id}`} row={item.row} props={props} shape={shape} bracket={bracket} />;
        }
      })}
    </>
  );
}

// --- recap, loops, devices --------------------------------------------------------------------

export function RecapLines({ recap, props, bracket, tone }: { recap: RecapModel; props: TodayLayoutProps; bracket?: boolean; tone: "primary" | "secondary" }) {
  const toneClass = tone === "primary" ? "tone-primary" : "tone-secondary";
  switch (recap.kind) {
    case "brief":
      return (
        <>
          <SkinText lines="max" className={`${toneClass} recap-text`}>{recap.text}</SkinText>
          {recap.origin && <SkinText role="label" className="tone-secondary">{t("From {device}", { device: recap.origin })}</SkinText>}
        </>
      );
    case "digest":
      return <SkinText lines="max" className={`${toneClass} recap-text`}>{recap.text}</SkinText>;
    case "writing":
      return (
        <SkinText role="label" className="tone-secondary">
          <span role="status">{t("Writing today's recap…")}</span>
        </SkinText>
      );
    case "failed":
      return (
        <>
          <SkinText lines={6} className="tone-error" as="p"><span role="alert">{recap.message}</span></SkinText>
          {bracket ? (
            <BracketLink label={t("Try again")} onClick={props.actions.retryRecap} disabled={!recap.retryEnabled} />
          ) : (
            <button type="button" className="mode" onClick={props.actions.retryRecap} disabled={!recap.retryEnabled}>
              {t("Try again")}
            </button>
          )}
        </>
      );
  }
}

export function OpenLoopLines({ props, marker, linkTone }: { props: TodayLayoutProps; marker?: string; linkTone?: boolean }) {
  return (
    <ul className="loops">
      {props.model.openLoops.map((loop) => (
        <li key={loop.id} className="loop">
          {marker && <span className="loop-marker tone-accent" aria-hidden="true">{marker}</span>}
          <SkinText lines={2} className={linkTone ? "tone-link" : "tone-primary"}>{loop.subject || t("(no subject)")}</SkinText>
        </li>
      ))}
    </ul>
  );
}

export function DevicesLine({ model, align }: { model: TodayModel; align?: "center" | "start" }) {
  const label = thisDeviceLabel(model.devices);
  const device = model.devices.find((d) => d.device.thisDevice) ?? model.devices[0];
  return (
    <div className={align === "center" ? "devices devices--center" : "devices"}>
      {device && label && <DeviceChip device={device.device} />}
      <SkinText role="label" className="tone-secondary" lines={2}>{devicesLine(model.devices, model.locale)}</SkinText>
    </div>
  );
}

// --- imagery ----------------------------------------------------------------------------------

/** An approved image of the skin as an object URL, or null while it loads or when it cannot be read. */
export function useSkinImage(skinId: string, asset: string | null): string | null {
  const [url, setUrl] = useState<string | null>(null);
  useEffect(() => {
    if (!asset) {
      setUrl(null);
      return;
    }
    let live = true;
    let made: string | null = null;
    skinImage(skinId, asset)
      .then((bytes) => {
        if (!live) return;
        made = URL.createObjectURL(new Blob([bytes]));
        setUrl(made);
      })
      .catch(() => live && setUrl(null));
    return () => {
      live = false;
      if (made) URL.revokeObjectURL(made);
    };
  }, [skinId, asset]);
  return url;
}

/** The skin's image for one placement: a header band 120 px tall, a 48 px badge, or a full-bleed background
 *  under a scrim. Draws nothing, and takes no space, when the skin has none or it cannot be read. */
export function SkinImage({ imagery, skinId, placement }: { imagery: Imagery[]; skinId: string; placement: ImagePlacement }) {
  const entry = imagery.find((i) => i.placement === placement);
  const url = useSkinImage(skinId, entry?.asset ?? null);
  if (!entry || !url) return null;
  return (
    <div className={`skin-image skin-image--${placement}`} aria-hidden="true">
      <img src={url} alt="" />
      {placement === "background" && <div className="skin-image__scrim" style={{ opacity: entry.scrim }} />}
    </div>
  );
}

// --- measuring --------------------------------------------------------------------------------

/** The width of an element, kept current. 0 until it has been measured. */
export function useWidth<T extends HTMLElement>(): [React.RefObject<T>, number] {
  const ref = useRef<T>(null);
  const [width, setWidth] = useState(0);
  useEffect(() => {
    const el = ref.current;
    if (!el) return;
    setWidth(el.getBoundingClientRect().width);
    if (typeof ResizeObserver === "undefined") return;
    const observer = new ResizeObserver((entries) => setWidth(entries[0]?.contentRect.width ?? 0));
    observer.observe(el);
    return () => observer.disconnect();
  }, []);
  return [ref, width];
}
