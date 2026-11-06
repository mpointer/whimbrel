import { useState } from "react";
import { meetingTrim } from "../api";
import { describeError, isCommandError } from "../errors";
import { t } from "../l10n";

const MINUTE = 60_000;

/** "1:05" for 65 minutes: the sheet's own clock, hours and minutes. */
export function hmm(ms: number): string {
  const minutes = Math.floor(Math.max(0, ms) / MINUTE);
  return `${Math.floor(minutes / 60)}:${String(minutes % 60).padStart(2, "0")}`;
}

/** The keep time typed as hours and minutes, in ms; `null` when it isn't a time. */
export function keepMs(hours: string, minutes: string): number | null {
  const h = hours.trim() === "" ? 0 : Number(hours);
  const m = minutes.trim() === "" ? 0 : Number(minutes);
  if (!Number.isInteger(h) || !Number.isInteger(m) || h < 0 || m < 0 || m > 59) return null;
  return (h * 60 + m) * MINUTE;
}

/** "Removes 0:55 after 1:05", or `null` when the keep time removes nothing or nothing is kept. */
export function removesLine(lengthMs: number, keep: number | null): string | null {
  if (keep === null || keep <= 0 || keep >= lengthMs) return null;
  return t("Removes {duration} after {time}", { duration: hmm(lengthMs - keep), time: hmm(keep) });
}

/**
 * "Trim recording": keep the first H:MM of the meeting; everything after it goes on every device.
 * `onTrimmed` runs after a trim (the page then regenerates the summary as Regenerate does).
 */
export function TrimSheet({
  meetingId,
  lengthMs,
  onTrimmed,
  onCancel,
}: {
  meetingId: number;
  lengthMs: number;
  onTrimmed: () => void;
  onCancel: () => void;
}) {
  const whole = Math.floor(lengthMs / MINUTE);
  const [hours, setHours] = useState(String(Math.floor(whole / 60)));
  const [minutes, setMinutes] = useState(String(whole % 60));
  const [busy, setBusy] = useState(false);
  const [note, setNote] = useState<string | null>(null);
  const keep = keepMs(hours, minutes);
  const removes = removesLine(lengthMs, keep);

  const trim = () => {
    if (keep === null || !removes) return;
    setBusy(true);
    setNote(null);
    meetingTrim(meetingId, keep)
      .then((trimmed) => (trimmed ? onTrimmed() : setNote(t("Nothing to trim."))))
      .catch((e: unknown) => setNote(isCommandError(e) ? describeError(e) : t("Couldn't trim the recording")))
      .finally(() => setBusy(false));
  };

  return (
    <div className="card confirm" role="alertdialog" aria-label={t("Trim recording")}>
      <p>
        <strong>{t("Trim recording")}</strong>
      </p>
      <div className="field-row">
        <span>{t("Keep the first")}</span>
        <input className="input short" inputMode="numeric" aria-label={t("Keep the first")} value={hours} onChange={(e) => setHours(e.target.value)} />
        <span>{":"}</span>
        <input className="input short" inputMode="numeric" aria-label={t("Keep the first")} value={minutes} onChange={(e) => setMinutes(e.target.value)} />
      </div>
      {removes && <p className="small">{removes}</p>}
      <p className="muted small">{t("This can't be undone.")}</p>
      {note && <p role="status">{note}</p>}
      <div className="toolbar">
        <button className="mode mode--active" disabled={busy || !removes} onClick={trim}>
          {t("Trim")}
        </button>
        <button className="mode" disabled={busy} onClick={onCancel}>
          {t("Cancel")}
        </button>
      </div>
    </div>
  );
}
