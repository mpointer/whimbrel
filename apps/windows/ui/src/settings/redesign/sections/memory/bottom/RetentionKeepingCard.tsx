import { Fragment, useCallback, useEffect, useState } from "react";
import { retentionPreview, retentionRunNow, retentionSettings, setRetentionSettings } from "../../../../../api";
import { describeError, isCommandError } from "../../../../../errors";
import type { RetentionPreview, RetentionView } from "../../../../../generated/types";
import { useL10n } from "../../../../../l10n";
import { timeLabel } from "../../../../../library/rows";
import { WButton, WConfirmDialog, WSettingsCard, WSettingsNote, WSettingsNumberRow, WSettingsRow } from "../../../../../system";
import { retentionRowLabels } from "../../../names";
import { RETENTION_KEYS, type RetentionKey } from "./retentionKeys";
import { retentionDecision, RETENTION_MAX_DAYS, RETENTION_MIN_DAYS, shownDays, type RetentionDecision, type RetentionIntent } from "./retentionDecision";
import { deleteNowCopy, lastRunLine, periodConfirmCopy } from "./retentionWording";
import "./bottom.css";

function message(e: unknown): string {
  return isCommandError(e) ? describeError(e) : String(e);
}

interface Asking {
  key: RetentionKey;
  days: number;
  from: number | null;
}

/**
 * Keeping data: how long each kind of data is kept. The safe rules (retentionDecision.ts): unticking "Keep forever"
 * saves nothing, a shorter period (or any period after forever) asks first and says what will be deleted, and
 * "Delete old data now" asks first. A save still tells the app (`onSaved`) so the first-run notice recounts.
 */
export function RetentionKeepingCard({ onSaved }: { onSaved: () => void }) {
  const { t, locale } = useL10n();
  const [view, setView] = useState<RetentionView | null>(null);
  const [error, setError] = useState<string | null>(null);
  /** The shell's own words for a failure, under the plain sentence. */
  const [detail, setDetail] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  /** The last stored number of each window, kept here because the shell stores null for "forever". */
  const [remembered, setRemembered] = useState<Partial<Record<RetentionKey, number>>>({});
  /** "Keep forever" was unticked and no period has been committed yet. Nothing is stored for these. */
  const [unticked, setUnticked] = useState<Partial<Record<RetentionKey, boolean>>>({});
  const [asking, setAsking] = useState<Asking | null>(null);
  const [deleteNow, setDeleteNow] = useState<{ preview: RetentionPreview | null } | null>(null);

  const accept = useCallback((next: RetentionView) => {
    setView(next);
    setRemembered((r) => {
      const out = { ...r };
      for (const key of RETENTION_KEYS) {
        const days = next.settings[key];
        if (days != null) out[key] = days;
      }
      return out;
    });
  }, []);

  useEffect(() => {
    retentionSettings().then(accept, (e: unknown) => setError(message(e)));
  }, [accept]);

  const labels = retentionRowLabels();

  /**
   * Writes one window's period. Retention syncs between computers, so the settings read at mount may be old: they are read
   * again right before the write, only this key changes on top of the fresh values, and if this key's stored value is no
   * longer the one the decision compared against, the decision is made again (a save that is now a shortening asks again).
   */
  const save = (key: RetentionKey, days: number | null, comparedTo: number | null) => {
    if (!view || busy) return;
    setBusy(true);
    setError(null);
    setDetail(null);
    // A failed re-read says so in plain words: nothing was written, whatever the shell's message is.
    retentionSettings()
      .catch((e: unknown) => {
        setError(t("Could not check the current setting, so nothing was saved."));
        setDetail(message(e));
        return null;
      })
      .then((fresh) => {
        if (!fresh) return null;
        const nowStored = fresh.settings[key] ?? null;
        if (nowStored !== comparedTo) {
          accept(fresh);
          const again = retentionDecision(nowStored, shownDays(nowStored, fresh.settings[key] ?? remembered[key]), days === null ? { kind: "tickForever" } : { kind: "commit", value: days });
          if (again.kind === "confirm") {
            setAsking({ key, days: again.days, from: again.from });
            return null;
          }
          if (again.kind !== "save") return null;
        }
        return setRetentionSettings({ ...fresh.settings, [key]: days });
      })
      .then((saved) => {
        if (!saved) return;
        accept(saved);
        setUnticked((u) => ({ ...u, [key]: false }));
        onSaved();
      })
      .catch((e: unknown) => setError(message(e)))
      .finally(() => setBusy(false));
  };

  const decide = (key: RetentionKey, intent: RetentionIntent): RetentionDecision | null => {
    if (!view) return null;
    const stored = view.settings[key] ?? null;
    const result = retentionDecision(stored, shownDays(stored, remembered[key]), intent);
    if (result.kind === "save") save(key, result.days, stored);
    else if (result.kind === "confirm") setAsking({ key, days: result.days, from: result.from });
    else if (result.kind === "none" && intent.kind === "untick") setUnticked((u) => ({ ...u, [key]: true }));
    return result;
  };

  const askDeleteNow = () => {
    setDeleteNow({ preview: null });
    // The counts come from the same command the first-run notice uses. Without them the dialog still says what goes.
    retentionPreview().then(
      (preview) => setDeleteNow((d) => (d ? { preview } : d)),
      () => {},
    );
  };
  const runNow = () => {
    setDeleteNow(null);
    setBusy(true);
    setError(null);
    setDetail(null);
    retentionRunNow()
      .then(accept)
      .catch((e: unknown) => setError(message(e)))
      .finally(() => setBusy(false));
  };

  const settings = view?.settings;
  const copy = view && deleteNow ? deleteNowCopy(view.settings, deleteNow.preview) : null;
  const askCopy = view && asking ? periodConfirmCopy(asking.key, asking.days, asking.from, view.acknowledged) : null;

  return (
    <WSettingsCard id="capture.retention" title={t("Keeping data")} headingLevel={3} loading={!view && !error} error={[error, detail]}>
      {view &&
        settings &&
        RETENTION_KEYS.map((key) => {
          const stored = settings[key] ?? null;
          const pending = stored === null && !!unticked[key];
          const shown = shownDays(stored, remembered[key]);
          return (
            <Fragment key={key}>
              <WSettingsNumberRow
                label={labels[key]}
                value={shown}
                min={RETENTION_MIN_DAYS}
                max={RETENTION_MAX_DAYS}
                unit={t("days")}
                disabled={busy}
                disabledReason={t("Saving…")}
                nothingStored={pending}
                companion={{
                  label: t("Keep forever"),
                  checked: stored === null && !pending,
                  onChange: (next) => {
                    if (next) {
                      if (stored === null) setUnticked((u) => ({ ...u, [key]: false }));
                      else decide(key, { kind: "tickForever" });
                    } else decide(key, { kind: "untick" });
                  },
                }}
                onCommit={(n) => {
                  if (!asking) decide(key, { kind: "commit", value: n });
                }}
              />
              {pending && (
                <div className="rs-membottom-pending">
                  <WSettingsNote>{t("Not saved yet. Nothing is deleted until you choose a period. Type the days and press Return, or use the button.")}</WSettingsNote>
                  <WButton disabled={busy} onClick={() => !asking && decide(key, { kind: "commit", value: shown })}>
                    {t("Keep {days} days", { days: shown })}
                  </WButton>
                </div>
              )}
            </Fragment>
          );
        })}
      {view && (
        <>
          <WSettingsNote>
            {t(
              "Anything older is deleted once a day, along with the recordings that belong to it. A meeting being recorded is never touched.",
            )}
          </WSettingsNote>
          {!view.acknowledged && (
            <WSettingsNote>
              {t("The daily sweep is waiting for the notice on the main screen. \"Delete old data now\" works regardless.")}
            </WSettingsNote>
          )}
          <WSettingsRow variant="destructive" label={t("Old data")} description={t("Anything older than the periods above.")} actionLabel={t("Delete old data now")} disabled={busy} disabledReason={t("Saving…")} onPress={askDeleteNow} />
          {view.lastRun && (
            <div role="status" className="rs-lastrun">
              <WSettingsNote>{lastRunLine(view.lastRun, timeLabel(view.lastRun.finishedUnixMs, locale))}</WSettingsNote>
            </div>
          )}
        </>
      )}
      {askCopy && asking && (
        <WConfirmDialog
          title={askCopy.title}
          body={askCopy.body}
          confirmLabel={askCopy.confirmLabel}
          onCancel={() => {
            // Cancel leaves what is stored. A row that was forever goes back to ticked.
            setUnticked((u) => ({ ...u, [asking.key]: false }));
            setAsking(null);
          }}
          onConfirm={() => {
            setAsking(null);
            save(asking.key, asking.days, asking.from);
          }}
        />
      )}
      {copy && (
        <WConfirmDialog
          title={copy.title}
          body={copy.body}
          confirmLabel={copy.confirmLabel}
          fields={
            copy.lines.length > 0 ? (
              <ul className="rs-membottom-list">
                {copy.lines.map((line) => (
                  <li key={line}>{line}</li>
                ))}
              </ul>
            ) : undefined
          }
          onCancel={() => setDeleteNow(null)}
          onConfirm={runNow}
        />
      )}
    </WSettingsCard>
  );
}
