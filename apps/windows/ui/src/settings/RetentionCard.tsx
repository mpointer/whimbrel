import { useEffect, useState } from "react";
import { retentionRunNow, retentionSettings, setRetentionSettings } from "../api";
import { describeError, isCommandError } from "../errors";
import type { RetentionSettings, RetentionView } from "../generated/types";
import { useL10n } from "../l10n";
import { timeLabel } from "../library/rows";

function message(e: unknown): string {
  return isCommandError(e) ? describeError(e) : String(e);
}

type Window = "captureDays" | "meetingDays" | "conversationDays";

function windows(t: (key: string) => string): { key: Window; label: string }[] {
  return [
    { key: "captureDays", label: t("Screen text") },
    { key: "meetingDays", label: t("Meetings and recordings") },
    { key: "conversationDays", label: t("Chats") },
  ];
}

/**
 * Settings → Retention (M3.3 Task 8): how long each kind of data is kept. A save tells
 * the app (`onSaved`) so the first-run notice, if it is up, recounts against the new
 * windows — saving is not itself an answer to it.
 */
export function RetentionCard({ onSaved }: { onSaved?: () => void } = {}) {
  const { t, locale } = useL10n();
  const [view, setView] = useState<RetentionView | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    retentionSettings()
      .then(setView)
      .catch((e: unknown) => setError(message(e)));
  }, []);

  const save = (settings: RetentionSettings) => {
    setView((current) => (current ? { ...current, settings } : current));
    setError(null);
    setRetentionSettings(settings)
      .then((saved) => {
        setView(saved);
        onSaved?.();
      })
      .catch((e: unknown) => setError(message(e)));
  };

  if (!view) return <section className="card about">{error ? <p role="alert">{error}</p> : t("Loading…")}</section>;
  const { settings, lastRun } = view;

  return (
    <section className="card about" aria-labelledby="retention-heading">
      <h2 id="retention-heading" className="section-label">
        {t("Keeping data")}
      </h2>
      {windows(t).map(({ key, label }) => {
        const days = settings[key];
        return (
          <div className="field-row" key={key}>
            <span>{label}</span>
            <label className="toggle">
              <input
                type="checkbox"
                checked={days == null}
                onChange={(e) => save({ ...settings, [key]: e.target.checked ? null : 30 })}
              />
              {t("Keep forever")}
            </label>
            {days != null && (
              <>
                <input
                  className="input short"
                  type="number"
                  min={1}
                  max={3650}
                  value={days}
                  aria-label={t("{label}: days to keep", { label })}
                  onChange={(e) => save({ ...settings, [key]: Math.max(1, Number(e.target.value)) })}
                />
                <span className="muted small">{t("days")}</span>
              </>
            )}
          </div>
        );
      })}
      <p className="muted small">
        {t(
          "Anything older is deleted once a day, along with the recordings that belong to it. A meeting being recorded is never touched.",
        )}
      </p>
      {!view.acknowledged && (
        <p className="muted small">
          {t("The daily sweep is waiting for the notice on the main screen. \"Delete old data now\" works regardless.")}
        </p>
      )}
      <div className="toolbar">
        <button
          className="mode"
          disabled={busy}
          onClick={() => {
            setBusy(true);
            setError(null);
            retentionRunNow()
              .then(setView)
              .catch((e: unknown) => setError(message(e)))
              .finally(() => setBusy(false));
          }}
        >
          {t("Delete old data now")}
        </button>
      </div>
      {lastRun && (
        <p className="muted small" role="status">
          {t("Last run {time}: {captures} screen captures, {meetings} meetings, {chats} chats, {orphans} leftover recordings deleted.", {
            time: timeLabel(lastRun.finishedUnixMs, locale),
            captures: lastRun.capturesDeleted,
            meetings: lastRun.meetingsDeleted,
            chats: lastRun.conversationsDeleted,
            orphans: lastRun.orphansDeleted,
          })}
        </p>
      )}
      {error && <p role="alert">{error}</p>}
    </section>
  );
}
