import { useCallback, useEffect, useRef, useState } from "react";
import { meetingList, meetingMostRelevant, profileGet, todayOverview, todayRecap } from "../api";
import { describeError, isCommandError } from "../errors";
import type { MeetingRow, TodayOverview, TodayRecap } from "../generated/types";
import { Detail, type Selection } from "../library/Detail";
import { DeviceChip } from "../library/LibraryView";
import { dateTimeLabel } from "../library/rows";
import { Workspace } from "../meetings/Workspace";
import { BriefRecapBody } from "./BriefRecapBody";
import { RecapBody } from "./DigestView";
import { NextMeetingCard } from "./NextMeetingCard";
import { PrepBriefPanel } from "./PrepBrief";
import { TodayTimeline } from "./TodayTimeline";
import type { MeetingCardTarget } from "../meetings/meetingCardModel";
import { openTopic, useTopicLookup } from "../topics/TopicChip";
import { useMeetingChips } from "../meetings/useMeetingChips";
import type { TopicLookup } from "../generated/types";
import { UpcomingCard, useUpcoming } from "./UpcomingCard";
import { TodayComposer } from "./TodayComposer";
import { SkinImage, type TodayActions } from "./archetypes/parts";
import { earlierMeetings, greetingFor, localizedVoice, recapModel, voiceText, type TodayModel } from "./layoutModel";
import { useL10n } from "../l10n";
import { TodayLayout } from "./TodayLayout";
import { useSchedule } from "./useSchedule";
import { useSkin } from "../useSkin";
import { deviceLine, recapFor, shouldGenerate, threadLine } from "./todayModel";

function message(e: unknown): string {
  return isCommandError(e) ? describeError(e) : String(e);
}

/** The Today tab (plan Task 9). Refreshes when shown and on window focus; no timer. */
export function TodayView({ onAsk, onOpenSettings }: { onAsk: (prompt: string) => void; onOpenSettings: () => void }) {
  const [overview, setOverview] = useState<TodayOverview | null>(null);
  const [recap, setRecap] = useState<TodayRecap | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [recapError, setRecapError] = useState<string | null>(null);
  const [writing, setWriting] = useState(false);
  const [selected, setSelected] = useState<Selection | null>(null);
  const [profileName, setProfileName] = useState("");
  // Meetings from earlier days, for the skinned layouts' Earlier section.
  const [earlier, setEarlier] = useState<MeetingRow[]>([]);
  const skin = useSkin();
  const { lang, locale, t } = useL10n();
  const voice = localizedVoice(skin.skin, lang);
  // The next meeting's full brief, opened from its schedule row.
  const [briefOpen, setBriefOpen] = useState(false);
  // Any other brief, opened from a card's Prep chip (or an upcoming card), by its key.
  const [prepKey, setPrepKey] = useState<string | null>(null);
  /// Which meeting Today shows: the shell picks it (recording, else today's next, else latest).
  const [relevantMeeting, setRelevantMeeting] = useState<number | null>(null);
  // A recap holds the store for a model call, so never two at once, and at most one
  // automatic attempt per day (see shouldGenerate).
  const inFlight = useRef(false);
  const attemptedDay = useRef<string | null>(null);

  const loadOverview = useCallback(() => {
    meetingMostRelevant()
      .then(setRelevantMeeting)
      .catch(() => {});
    meetingList("")
      .then((rows) => setEarlier(earlierMeetings(rows, Date.now())))
      .catch(() => setEarlier([]));
    // The greeting uses the Profile name; if it cannot be read, the heading is just the date.
    profileGet()
      .then((p) => setProfileName(p.name))
      .catch(() => setProfileName(""));
    todayOverview()
      .then((o) => {
        setOverview(o);
        setError(null);
        setRecap((current) => recapFor(o, current));
      })
      .catch((e: unknown) => setError(message(e)));
  }, []);

  const generate = useCallback((refresh: boolean) => {
    if (inFlight.current) return;
    inFlight.current = true;
    setWriting(true);
    setRecapError(null);
    todayRecap(refresh)
      .then(setRecap)
      .catch((e: unknown) => setRecapError(message(e)))
      .finally(() => {
        inFlight.current = false;
        setWriting(false);
      });
  }, []);

  const sched = useSchedule(loadOverview);
  const upcoming = useUpcoming();
  // The cards' chips: a topic per schedule event (a recorded one by its meeting, else by the event), and each
  // held meeting's Prep key and topic from the lookup the Meetings list uses. Both run only when the rows change.
  const eventTopics = useTopicLookup(
    (sched.schedule?.events ?? []).map((e): TopicLookup =>
      e.recordedMeetingId != null ? { kind: "meeting", id: e.recordedMeetingId } : { kind: "event", id: e.eventId, title: e.title },
    ),
  );
  const heldChips = useMeetingChips([...(sched.schedule?.earlierToday ?? []), ...earlier]);

  useEffect(() => {
    loadOverview();
    // Focus re-reads the overview only; it never regenerates the recap on its own.
    window.addEventListener("focus", loadOverview);
    return () => window.removeEventListener("focus", loadOverview);
  }, [loadOverview]);

  useEffect(() => {
    if (overview && shouldGenerate(overview, attemptedDay.current, recap) && !inFlight.current) {
      attemptedDay.current = overview.day;
      generate(false);
    }
  }, [overview, recap, generate]);

  if (selected) return <Detail selection={selected} onBack={() => setSelected(null)} />;
  if (!overview) {
    return error ? (
      <div role="alert" className="card">
        {error}
      </div>
    ) : (
      <p className="muted">{t("Loading…")}</p>
    );
  }

  const heading = greetingFor(profileName, new Date(), voice, locale);
  const openMeeting = (id: number) => setSelected({ kind: "meeting", id });
  const openTarget = (target: MeetingCardTarget) => {
    if (target.kind === "meeting") openMeeting(target.id);
    else if (target.kind === "brief") setPrepKey(target.key);
    else if (target.kind === "topic") openTopic(target.id);
  };
  const prepPanel = prepKey ? <PrepBriefPanel prepKey={prepKey} onClose={() => setPrepKey(null)} onOpenMeeting={openMeeting} /> : null;
  const composer = <TodayComposer onAsk={onAsk} onOpenSettings={onOpenSettings} placeholder={voiceText(voice, "askPlaceholder")} />;
  const card = <NextMeetingCard onOpenMeeting={openMeeting} />;

  // The meeting that matters now: the one recording, else today's next, else the most recent (the Mac's
  // TodayView choice) — its workspace, not a link to it.
  const workspace =
    relevantMeeting != null ? (
      <Workspace meetingId={relevantMeeting} onChanged={loadOverview} />
    ) : (
      <section className="card" aria-labelledby="meeting-heading">
        <h2 id="meeting-heading" className="section-label">
          {t("Meeting")}
        </h2>
        <p className="muted">
          {voiceText(voice, "emptyDay")}. {t("Record one in the Meetings tab, or they arrive from your Mac through sync.")}
        </p>
      </section>
    );

  if (skin.archetype !== "standard") {
    const model: TodayModel = {
      now: sched.now,
      locale,
      name: profileName,
      schedule: sched.schedule,
      next: sched.next,
      earlier,
      openLoops: overview.openLoops,
      recap: recapModel(overview, recap, writing, recapError),
      devices: overview.devices,
      upcoming,
      chips: { eventTopics, held: heldChips },
    };
    const actions: TodayActions = {
      start: sched.start,
      openMeeting,
      retryRecap: () => generate(true),
      toggleBrief: () => setBriefOpen((o) => !o),
      showPrepBrief: setPrepKey,
      openTopic,
    };
    return (
      <div className="today stack">
        <TodayLayout
          archetype={skin.archetype}
          model={model}
          voice={voice}
          imagery={skin.imagery}
          skinId={skin.skin.id}
          caps={skin.skin.typography.caps}
          composer={composer}
          actions={actions}
          starting={sched.starting}
          brief={{ open: briefOpen, card }}
          stripes={skin.skin.decoration.headerBand === "stripes-diagonal"}
          dividers={skin.skin.decoration.dividers !== "none"}
          motion={skin.skin.motion === "subtle"}
          heading={heading}
        />
        {sched.error && <p role="alert">{sched.error}</p>}
        {prepPanel}
        {workspace}
      </div>
    );
  }

  return (
    <div className="today stack">
      <header className="page-heading standard-header">
        <div className="standard-header__text">
          {heading.kicker && <p className="page-date">{heading.kicker}</p>}
          <h1 className="page-title">{heading.title}</h1>
        </div>
        <SkinImage imagery={skin.imagery} skinId={skin.skin.id} placement="badge" />
      </header>

      {composer}

      <TodayTimeline
        state={sched}
        title={voiceText(voice, "upcomingHeader")}
        briefOpen={briefOpen}
        brief={card}
        onToggleBrief={() => setBriefOpen((o) => !o)}
        onOpenMeeting={openMeeting}
        onOpen={openTarget}
        eventTopics={eventTopics}
        heldChips={heldChips}
      />

      {prepPanel}

      <UpcomingCard view={upcoming} />

      <section className="card" aria-labelledby="recap-heading">
        <div className="section-head">
          <h2 id="recap-heading" className="section-label">
            {voiceText(voice, "recapHeader")}
          </h2>
          {!overview.briefRecap && (
            <button className="mode" onClick={() => generate(true)} disabled={writing}>
              {writing ? t("Writing…") : t("Refresh")}
            </button>
          )}
        </div>
        {overview.briefRecap ? (
          <BriefRecapBody brief={overview.briefRecap} />
        ) : recap ? (
          <RecapBody recap={recap} />
        ) : writing ? (
          <p className="muted" role="status">
            {t("Writing today's recap…")}
          </p>
        ) : null}
        {recapError && <p role="alert">{recapError}</p>}
      </section>

      <section className="card" aria-labelledby="loops-heading">
        <h2 id="loops-heading" className="section-label">
          {t("Open loops")}
        </h2>
        {overview.openLoops.length === 0 ? (
          <p className="muted">
            {t("No threads need you right now. Email threads arrive with Gmail on Windows (M3.4), or from your other devices through sync.")}
          </p>
        ) : (
          <ul className="rows">
            {overview.openLoops.map((loop) => (
              <li key={loop.id} className="row-card static">
                <span className="row-top">
                  <span className="row-title">{loop.subject || t("(no subject)")}</span>
                  <time className="muted small">{dateTimeLabel(loop.lastActivityTsUnixMs, new Date(), locale)}</time>
                </span>
                <span className="row-meta muted small">{threadLine(loop)}</span>
                {loop.summary && <span className="row-snippet">{loop.summary}</span>}
              </li>
            ))}
          </ul>
        )}
      </section>

      {workspace}

      <section className="card" aria-labelledby="devices-heading">
        <h2 id="devices-heading" className="section-label">
          {t("Across your devices today")}
        </h2>
        <ul className="device-counts">
          {overview.devices.map((d) => (
            <li key={d.device.deviceId}>
              <DeviceChip device={d.device} />
              <span className="small">{deviceLine(d, locale)}</span>
            </li>
          ))}
        </ul>
      </section>
    </div>
  );
}
