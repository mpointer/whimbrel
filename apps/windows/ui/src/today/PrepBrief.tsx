import { useEffect, useRef, useState } from "react";
import { meetingPrepBrief } from "../api";
import { describeError, isCommandError } from "../errors";
import type { NextMeetingView, PrepBriefView } from "../generated/types";
import { dateTimeLabel } from "../library/rows";
import { useL10n } from "../l10n";
import { citedSources, prepStatus } from "./todayModel";

/** One chip per source a claim cites. A meeting this PC has opens it; any other source is a plain label. */
function SourceChips({ brief, ids, onOpenMeeting }: { brief: PrepBriefView; ids: string[]; onOpenMeeting: (id: number) => void }) {
  const cited = citedSources(brief, ids);
  if (cited.length === 0) return null;
  return (
    <span className="source-chips">
      {cited.map((s) =>
        s.meetingId != null ? (
          <button key={s.id} type="button" className="chip source-chip" onClick={() => onOpenMeeting(s.meetingId as number)}>
            {s.title}
          </button>
        ) : (
          <span key={s.id} className="chip source-chip source-chip--label">
            {s.title}
          </span>
        ),
      )}
    </span>
  );
}

/** A Meeting Prep brief's summary, attendees, open items and open questions, each with its sources. */
export function PrepBriefBody({ brief, onOpenMeeting }: { brief: PrepBriefView; onOpenMeeting: (id: number) => void }) {
  const { t } = useL10n();
  const sources = (ids: string[]) => <SourceChips brief={brief} ids={ids} onOpenMeeting={onOpenMeeting} />;
  return (
    <div className="stack">
      <p>
        {brief.summary}
        {sources(brief.summarySources)}
      </p>
      {brief.attendees.length > 0 && (
        <ul className="rows">
          {brief.attendees.map((a) => (
            <li key={a.who} className="row-card static">
              <span className="row-title">{a.who}</span>
              <span className="row-snippet">{a.note}</span>
              {sources(a.sources)}
            </li>
          ))}
        </ul>
      )}
      {brief.openItems.length > 0 && (
        <>
          <h3 className="section-label">{t("Open items")}</h3>
          <ul>
            {brief.openItems.map((c) => (
              <li key={c.text}>
                {c.text}
                {sources(c.sources)}
              </li>
            ))}
          </ul>
        </>
      )}
      {brief.openQuestions.length > 0 && (
        <>
          <h3 className="section-label">{t("Open questions")}</h3>
          <ul>
            {brief.openQuestions.map((c) => (
              <li key={c.text}>
                {c.text}
                {sources(c.sources)}
              </li>
            ))}
          </ul>
        </>
      )}
    </div>
  );
}

/**
 * The brief a Prep chip (or an upcoming card) opened, by its key, shown in place under the list that opened
 * it, with Hide brief to close it. Nothing when the brief is gone.
 */
export function PrepBriefPanel({ prepKey, onClose, onOpenMeeting }: { prepKey: string; onClose: () => void; onOpenMeeting: (id: number) => void }) {
  const { locale, t } = useL10n();
  const [found, setFound] = useState<{ key: string; view: NextMeetingView | null; error: string | null } | null>(null);
  const panel = useRef<HTMLElement>(null);

  // The panel sits under the list that opened it, so bring it into view when a chip opens (or changes) it.
  useEffect(() => {
    panel.current?.scrollIntoView?.({ block: "nearest", behavior: "smooth" });
  }, [prepKey]);

  useEffect(() => {
    let live = true;
    meetingPrepBrief(prepKey).then(
      (view) => live && setFound({ key: prepKey, view: view ?? null, error: null }),
      (e: unknown) => live && setFound({ key: prepKey, view: null, error: isCommandError(e) ? describeError(e) : String(e) }),
    );
    return () => {
      live = false;
    };
  }, [prepKey]);

  const current = found?.key === prepKey ? found : null;
  const brief = current?.view?.brief ?? null;
  return (
    <section ref={panel} className="card prep-brief-panel" aria-labelledby="prep-brief-heading">
      <div className="section-head">
        <h2 id="prep-brief-heading" className="section-label">
          {t("Prep ready")}
        </h2>
        <button type="button" className="mode" onClick={onClose} aria-expanded={true}>
          {t("Hide brief")}
        </button>
      </div>
      {current?.error && <p role="alert">{current.error}</p>}
      {current?.view && (
        <p>
          <strong>{current.view.title}</strong> <span className="muted small">{dateTimeLabel(current.view.startUnixMs, new Date(), locale)}</span>
        </p>
      )}
      {brief && <PrepBriefBody brief={brief} onOpenMeeting={onOpenMeeting} />}
      {current && !current.error && <p className="muted small">{prepStatus(brief, locale)}</p>}
    </section>
  );
}
