// Home in the Thread look, behind the redesign flag (docs/ui-redesign/screens/home.md). TodayView keeps all of its
// loading and hands what it has loaded to this; this adds only the reads in useHomeData and draws one column: the
// greeting, the search and ask bar, the next meeting, the time line for the day, then the sections that have content.

import { Fragment, useEffect, useState, type ReactNode } from "react";
import type { DayBlock } from "../../api";
import type { MeetingRow, TodayOverview, TodayRecap, UpcomingView } from "../../generated/types";
import { useL10n } from "../../l10n";
import { requestFindWith } from "../../library/redesign/findStore";
import { timeLabel } from "../../library/rows";
import {
  captureLabel,
  WButton,
  WEmptyState,
  WErrorState,
  WLoadingState,
  WMeetingCard,
  WMemoryBlock,
  WNextMeeting,
  WRecordingIndicator,
  WSectionLabel,
  WSpine,
  WSpineNow,
  WSpineStop,
  WIcon,
} from "../../system";
import { setThreadParts, threadAnchorId } from "../../shell/redesign/outline";
import { pauseEndLabel } from "../../shell/redesign/captureModel";
import { elapsedSeconds } from "../../shell/redesign/recordingBar";
import { useCapture } from "../../shell/redesign/useCapture";
import { useSkin } from "../../useSkin";
import { waitingLabel } from "../../automations/automationsModel";
import { SkinImage } from "../skinImage";
import { BriefRecapBody } from "../BriefRecapBody";
import { RecapBody } from "../DigestView";
import { localizedVoice, recapModel, voiceText } from "../layoutModel";
import { threadLine } from "../todayModel";
import type { ScheduleState } from "../useSchedule";
import { UpcomingBody, upcomingShows } from "../UpcomingCard";
import { elementsFromOptions } from "./elements";
import {
  arrangeTimeline,
  earlierGroups,
  blockItems,
  blockRange,
  buildTimeline,
  captureLine,
  dayIsEmpty,
  heroOf,
  minutesLabel,
  NO_FILTER,
  soonLabel,
  type EarlierGroup,
  type Hero,
  type MeetingStop,
} from "./homeModel";
import { useApprovals, useCalendarConnected, useDayBlocks } from "./useHomeData";
import { withDividers } from "./withDividers";
import "./home.css";

export type HomeSection = "connectors" | "capture" | "automations" | "library";

export interface RedesignedHomeProps {
  overview: TodayOverview | null;
  error: string | null;
  onRetry: () => void;
  sched: ScheduleState;
  /** Meetings from before today, newest first (the rows Yesterday and Last week fold). */
  earlier: MeetingRow[];
  upcoming: UpcomingView | null;
  /** The meeting the shell says matters now (the recording one). */
  relevantMeeting: number | null;
  recap: TodayRecap | null;
  writing: boolean;
  recapError: string | null;
  onRefreshRecap: () => void;
  onOpenMeeting: (id: number) => void;
  /** Opens a meeting's prep brief by its key; the brief itself comes back as `prep`. */
  onOpenPrep: (key: string) => void;
  prep: ReactNode;
  onOpenSection: (target: HomeSection) => void;
}

export { withDividers };

export function RedesignedHome(p: RedesignedHomeProps) {
  const { t, locale, lang } = useL10n();
  const skin = useSkin();
  const el = elementsFromOptions(skin.skin.today.options, skin.skin.today.archetype);
  const voice = localizedVoice(skin.skin, lang);
  const { sched } = p;
  const now = sched.now;
  const capture = useCapture(true);
  const { blocks } = useDayBlocks(sched.schedule, now);
  const calendarConnected = useCalendarConnected();
  const approvals = useApprovals();
  const [openGroup, setOpenGroup] = useState<EarlierGroup["id"] | null>(null);
  const links = el.actions === "links";
  // A skin with a divider style draws a rule above each section after the first (decoration.dividers; Thread has none).
  const dividers = skin.skin.decoration.dividers !== "none";
  const section = (id: string, words: string) => (
    <WSectionLabel id={id} variant={el.labels}>
      {words}
    </WSectionLabel>
  );
  const groups = earlierGroups(p.earlier, now, locale);

  const hero = heroOf({ now, schedule: sched.schedule, relevantMeeting: p.relevantMeeting, calendarConnected });
  const stops = buildTimeline({
    now,
    locale,
    schedule: sched.schedule,
    heroEventId: hero.kind === "next" ? hero.event.eventId : null,
    heroMeetingId: hero.kind === "live" ? hero.meetingId : null,
    blocks,
  });
  const dayEmpty = (hero.kind === "noCalendar" || hero.kind === "noMeetings") && dayIsEmpty(stops);

  // The sidebar's Thread list: the parts of the line that have content.
  const partsKey = `${p.overview ? (dayEmpty ? "-" : "t") : "-"}|${groups.map((g) => `${g.id}:${g.note}`).join(",")}`;
  useEffect(() => {
    const parts = [
      ...(p.overview && !dayEmpty ? [{ id: "today" as const, label: t("Today"), note: timeLabel(now, locale) }] : []),
      ...groups.map((g) => ({ id: g.id, label: g.id === "yesterday" ? t("Yesterday") : t("Earlier"), note: g.note })),
    ];
    setThreadParts(parts);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [partsKey]);
  useEffect(() => () => setThreadParts([]), []);

  const wrap = (children: ReactNode) => (
    <div className="today th-home" data-next={el.nextMeeting} data-timeline={el.timeline} data-labels={el.labels} data-actions={el.actions} data-dividers={dividers ? skin.skin.decoration.dividers : undefined}>
      <SkinImage imagery={skin.imagery} skinId={skin.skin.id} placement="badge" />
      {children}
    </div>
  );
  if (!p.overview) {
    return wrap(p.error ? <WErrorState title={t("Today could not be loaded.")} help={p.error} onAction={p.onRetry} /> : <WLoadingState title={t("Loading…")} />);
  }
  const overview = p.overview;

  // --- the next meeting, in the line ---
  const open = (id: number | null) => id != null && p.onOpenMeeting(id);
  const heroBlock = (h: Hero) => {
    if (h.kind === "live") {
      return (
        <WNextMeeting
          variant={el.nextMeeting}
          links={links}
          title={h.title}
          time={h.since != null ? timeLabel(h.since, locale) : ""}
          tag={t("In progress")}
          attendees={[]}
          indicator={<WRecordingIndicator size="pill" state="recording" elapsedSeconds={h.since != null ? elapsedSeconds(h.since, now) : undefined} />}
          primaryLabel={h.meetingId != null ? t("Open") : undefined}
          onPrimary={() => open(h.meetingId)}
        />
      );
    }
    if (h.kind === "next") {
      const e = h.event;
      const time = timeLabel(e.startUnixMs, locale);
      return (
        <WNextMeeting
          variant={el.nextMeeting}
          links={links}
          title={e.title}
          time={time}
          soon={soonLabel(e.startUnixMs, now)}
          when={t("Today, {time}", { time })}
          attendees={e.attendees.slice(0, 4)}
          primaryLabel={sched.schedule?.recording ? undefined : sched.starting === e.eventId ? t("Starting…") : t("Start")}
          onPrimary={() => sched.start(e.eventId)}
          secondaryLabel={e.prepKey ? t("Open brief") : undefined}
          onSecondary={() => e.prepKey && p.onOpenPrep(e.prepKey)}
        />
      );
    }
    if (h.kind === "noCalendar") {
      return (
        <p className="th-quiet">
          {t("No calendar is connected.")}{" "}
          <WButton link={links} variant="quiet" onClick={() => p.onOpenSection("connectors")}>
            {t("Connect a calendar")}
          </WButton>
        </p>
      );
    }
    return <p className="th-quiet">{h.kind === "noMeetings" ? t("No meetings today.") : t("Nothing else on your calendar today.")}</p>;
  };
  const heroStop = (last: boolean) => {
    const live = hero.kind === "live";
    const time = hero.kind === "next" ? timeLabel(hero.event.startUnixMs, locale) : live ? timeLabel(hero.since ?? now, locale) : "";
    const name = hero.kind === "next" ? [time, hero.event.title, soonLabel(hero.event.startUnixMs, now)].join(", ") : live ? [time, hero.title].filter(Boolean).join(", ") : undefined;
    return (
      <WSpineStop key="hero" time={time} node={live ? "recording" : "upcoming"} label={name} last={last} hero>
        <div data-section="upNext">{heroBlock(hero)}</div>
        {p.prep}
      </WSpineStop>
    );
  };

  // --- the rest of the line ---
  const renderMeeting = (s: MeetingStop, last: boolean) => (
    <WSpineStop key={s.key} time={s.timeText} node={s.node} label={s.name} last={last} connector={s.node === "past" ? "solid" : "dashed"}>
      <WMeetingCard
        variant={s.node === "past" ? "past" : "upcoming"}
        links={links}
        title={s.title}
        attendees={s.attendees}
        soon={s.soon ?? undefined}
        primaryLabel={s.action === "open" ? t("Open") : s.action === "start" ? (sched.starting === s.eventId ? t("Starting…") : t("Start")) : undefined}
        onPrimary={() => (s.action === "open" ? open(s.meetingId) : s.eventId && sched.start(s.eventId))}
      />
    </WSpineStop>
  );
  // Open on a memory block: Find, with the block's time as the date filter, listing the captures in it by time.
  const openBlock = (block: DayBlock, label: string) => {
    requestFindWith("", "search", { ...NO_FILTER, dates: { kind: "span", start: block.start_unix_ms, end: block.end_unix_ms + 1, label } });
    p.onOpenSection("library");
  };
  const renderStop = (s: (typeof stops)[number], last: boolean) => {
    if (s.kind === "meeting") return renderMeeting(s, last);
    if (s.kind !== "memory") return null;
    const range = blockRange(s.block, locale);
    return (
      <WSpineStop key={s.key} time={timeLabel(s.at, locale)} timeNote={minutesLabel(s.block) ?? undefined} node="past" connector="solid" last={last}>
        <div className="th-block">
          <WMemoryBlock range={range} items={blockItems(s.block)} />
          <WButton variant="quiet" link={links} aria-label={t("Open memory, {range}", { range })} onClick={() => openBlock(s.block, range)}>
            {t("Open")}
          </WButton>
        </div>
      </WSpineStop>
    );
  };
  const { ahead, behind } = arrangeTimeline(stops);
  const note = captureLine(capture.state, now, locale);
  const nowText = capture.state ? captureLabel(capture.state.kind, capture.state.kind === "paused" ? pauseEndLabel(capture.state.resumeAt, now, locale) : undefined) : "";
  const label = (g: EarlierGroup) => (g.id === "yesterday" ? t("Yesterday") : t("Earlier"));
  const renderGroup = (g: EarlierGroup, last: boolean) => {
    const expanded = openGroup === g.id;
    return (
      <WSpineStop key={g.id} time="" node="upcoming" last={last}>
        <div id={threadAnchorId(g.id)} tabIndex={-1} className="th-earlier">
          <button type="button" className="th-earlier-head" aria-expanded={expanded} onClick={() => setOpenGroup(expanded ? null : g.id)}>
            <WIcon name="chevron" size={16} />
            <b>{label(g)}</b>
            <span className="th-earlier-note">{g.note}</span>
            {!expanded
              ? g.meetings.slice(0, 3).map((m) => (
                  <span key={m.id} className="w-meeting-tag">
                    {m.title}
                  </span>
                ))
              : null}
          </button>
          {expanded ? (
            <ul className="th-earlier-list">
              {g.meetings.map((m) => (
                <li key={m.id}>
                  <WMeetingCard variant="past" links={links} title={m.title} attendees={[]} primaryLabel={t("Open")} onPrimary={() => open(m.id)} />
                </li>
              ))}
            </ul>
          ) : null}
        </div>
      </WSpineStop>
    );
  };
  const lineItems = [
    ...ahead.map((s, i) => renderStop(s, false) ?? <Fragment key={`a${i}`} />),
    heroStop(false),
    ...(note
      ? [
          <WSpineStop key="note" time="" node="past" connector="dashed" label={note}>
            <p className="th-capture-note">{note}</p>
          </WSpineStop>,
        ]
      : []),
  ];
  const tail = [...behind.map((s) => renderStop(s, false)), ...groups.map((g) => renderGroup(g, false))];
  const timeline = (
    <section id={threadAnchorId("today")} tabIndex={-1} className="th-timeline" data-section="schedule" aria-labelledby="th-timeline-label">
      {section("th-timeline-label", t("Today"))}
      <WSpine label={t("Today, in time order")} variant={el.timeline}>
        {lineItems}
        <WSpineNow key={timeLabel(now, locale)} time={timeLabel(now, locale)} text={nowText} last={tail.length === 0} />
        {tail}
      </WSpine>
    </section>
  );

  const recapState = recapModel(overview, p.recap, p.writing, p.recapError);
  const emptyDay = (
    <>
      <WEmptyState
        icon="calendar"
        title={t("Nothing on today yet.")}
        actionLabel={hero.kind === "noCalendar" ? t("Connect a calendar") : undefined}
        onAction={hero.kind === "noCalendar" ? () => p.onOpenSection("connectors") : undefined}
      />
      {note ? (
        <p className="th-capture-note" role="status">
          {note}
        </p>
      ) : null}
      {groups.length > 0 ? (
        <WSpine label={t("Earlier")} variant={el.timeline}>
          {groups.map((g, i) => renderGroup(g, i === groups.length - 1))}
        </WSpine>
      ) : null}
    </>
  );
  // The parts of Home below the greeting, in order. A skin's divider goes above each one after the first.
  const parts: ReactNode[] = [
    dayEmpty ? emptyDay : timeline,

    approvals > 0 ? (
      <section className="th-section" data-section="approvals" aria-labelledby="th-approvals">
        {section("th-approvals", t("Waiting for approval"))}
        <div className="th-row">
          <span>{waitingLabel(approvals)}</span>
          <WButton link={links} onClick={() => p.onOpenSection("automations")}>
            {t("Open")}
          </WButton>
        </div>
      </section>
    ) : null,

    overview.openLoops.length > 0 ? (
      <section className="th-section" data-section="threads" aria-labelledby="th-threads">
        {section("th-threads", t("Open threads"))}
        <ul className="th-list">
          {overview.openLoops.map((loop) => (
            <li key={loop.id}>
              <span className="th-list-title">{loop.subject || t("(no subject)")}</span>
              <span className="th-list-meta">{threadLine(loop)}</span>
              {loop.summary ? <span className="th-list-text">{loop.summary}</span> : null}
            </li>
          ))}
        </ul>
      </section>
    ) : null,

    recapState || overview.briefRecap || p.recap ? (
      <section className="th-section" data-section="recap" aria-labelledby="th-recap">
        <div className="th-section-head">
          {section("th-recap", voiceText(voice, "recapHeader"))}
          {!overview.briefRecap ? (
            <WButton variant="quiet" link={links} disabled={p.writing} onClick={p.onRefreshRecap}>
              {p.writing ? t("Writing…") : t("Refresh")}
            </WButton>
          ) : null}
        </div>
        {overview.briefRecap ? <BriefRecapBody brief={overview.briefRecap} formatted /> : p.recap ? <RecapBody recap={p.recap} formatted /> : p.writing ? <p role="status">{t("Writing today's recap…")}</p> : null}
        {p.recapError ? <p role="alert">{p.recapError}</p> : null}
      </section>
    ) : null,

    upcomingShows(p.upcoming) && p.upcoming ? (
      <section className="th-section" data-section="upcoming" aria-labelledby="th-week">
        {section("th-week", t("This week"))}
        <UpcomingBody view={p.upcoming} />
      </section>
    ) : null,
  ];
  return wrap(
    <>
      {sched.error ? (
        <p role="alert" className="th-quiet">
          {sched.error}
        </p>
      ) : null}
      {withDividers(parts, dividers)}
    </>,
  );
}
