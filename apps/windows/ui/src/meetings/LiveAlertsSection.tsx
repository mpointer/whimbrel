import { useEffect, useState } from "react";
import ReactMarkdown from "react-markdown";
import { meetingLiveAlerts, meetingSettings } from "../api";
import { markdownComponents } from "../chat/AskView";
import { dateTimeLabel } from "../library/rows";
import type { LiveAlertView } from "../generated/types";
import { useL10n } from "../l10n";
import { shouldPoll } from "./liveAlertsModel";

/**
 * "Questions for you": the live question alerts heard in this meeting, each with its draft
 * answer, during the meeting and after it. Renders nothing when there are none. The Mac's
 * `LiveAlertsSection`, without Jump to moment (the workspace has no transcript seek yet).
 * Alerts this PC made list what their draft cites; another device's citations don't resolve
 * here, so those say where the sources are instead.
 */
export function LiveAlertsSection({
  meetingId,
  recording,
  endedUnixMs,
}: {
  meetingId: number;
  recording: boolean;
  endedUnixMs: number | null;
}) {
  const { locale, t } = useL10n();
  const [rows, setRows] = useState<LiveAlertView[]>([]);
  const [alertsOn, setAlertsOn] = useState(false);

  useEffect(() => {
    meetingSettings().then((s) => setAlertsOn(s.liveQuestionAlerts), () => {});
  }, []);

  useEffect(() => {
    let live = true;
    let latest: LiveAlertView[] = [];
    const load = () =>
      meetingLiveAlerts(meetingId).then((r) => {
        if (live) setRows((latest = r));
      }, () => {});
    load();
    const timer = setInterval(() => {
      if (shouldPoll({ alertsOn, recording, endedUnixMs, rows: latest, nowUnixMs: Date.now() })) load();
    }, 10_000);
    return () => {
      live = false;
      clearInterval(timer);
    };
  }, [meetingId, alertsOn, recording, endedUnixMs]);

  if (rows.length === 0) return null;
  return (
    <section className="field" aria-labelledby={`questions-${meetingId}`}>
      <span id={`questions-${meetingId}`} className="section-label">
        {t("Questions for you")}
      </span>
      {rows.map((row) => (
        <div key={row.id} className="card stack">
          <strong>{row.restatement}</strong>
          <p className="muted">
            <em>{`“${row.quote}”`}</em>
          </p>
          <p className="muted small">
            {row.asker ? t("{asker} asked at {time}", { asker: row.asker, time: row.askedAt }) : t("Asked at {time}", { time: row.askedAt })}
          </p>
          {row.draft ? (
            <div className="markdown">
              <ReactMarkdown components={markdownComponents}>{row.draft}</ReactMarkdown>
            </div>
          ) : (
            <p className="muted small">{t("No draft answer yet.")}</p>
          )}
          {row.evidence.length > 0 && (
            <div className="citations">
              <span className="muted small">{t("Sources")}</span>
              {row.evidence.map((e, i) => (
                <span key={i} className="chip">
                  {e.tsUnixMs == null ? e.label : `${e.label} · ${dateTimeLabel(e.tsUnixMs, new Date(), locale)}`}
                </span>
              ))}
            </div>
          )}
          {row.sourcesElsewhere && <p className="muted small">{t("Sources are on the device that recorded this meeting.")}</p>}
        </div>
      ))}
    </section>
  );
}
