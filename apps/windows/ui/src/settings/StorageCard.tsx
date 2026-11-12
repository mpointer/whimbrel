import { useCallback, useEffect, useState } from "react";
import { storageCompact, storageDeleteOldCaptures, storageDeleteOldRecordings, storageFreeUpOptions, storageOverview } from "../api";
import { describeError, isCommandError } from "../errors";
import type { FreeUpOptions, StorageView } from "../generated/types";
import { useL10n } from "../l10n";
import { compactLine, footprintLines, freedLine, freeUpRows, lowDiskLines, type FreeUpKind } from "./storageModel";

function message(e: unknown): string {
  return isCommandError(e) ? describeError(e) : String(e);
}

/**
 * Settings → Storage (plan 2026-11-12-storage-visibility, the Mac's Storage block): what Whimbrel
 * keeps on this PC, a low-disk warning, "Compact database" and "Free up space…". Free up space never
 * changes the retention windows and never touches the meeting being recorded.
 */
export function StorageCard() {
  const { t } = useL10n();
  const [view, setView] = useState<StorageView | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [working, setWorking] = useState<string | null>(null);
  const [result, setResult] = useState<string | null>(null);
  const [open, setOpen] = useState(false);
  const [days, setDays] = useState<number | null>(null);
  const [options, setOptions] = useState<FreeUpOptions | null>(null);
  const [asking, setAsking] = useState<FreeUpKind | null>(null);

  const load = useCallback(
    () =>
      storageOverview()
        .then((v) => {
          setView(v);
          return v;
        })
        .catch((e: unknown) => {
          setError(message(e));
          return null;
        }),
    [],
  );

  useEffect(() => {
    void load();
  }, [load]);

  const chosen = days ?? view?.defaultDays ?? 90;
  useEffect(() => {
    if (!open) return;
    setOptions(null);
    storageFreeUpOptions(chosen).then(setOptions, (e: unknown) => setError(message(e)));
  }, [open, chosen]);

  /** Runs one step, then measures again and says what it freed. */
  const run = async (label: string, work: () => Promise<string | null>) => {
    if (working) return;
    const before = view?.footprint.total ?? 0;
    setWorking(label);
    setError(null);
    setResult(null);
    setAsking(null);
    try {
      const said = await work();
      const after = await load();
      const freed = after ? freedLine(before, after.footprint.total) : null;
      setResult([said, freed].filter(Boolean).join(" ") || null);
      if (open) setOptions(await storageFreeUpOptions(chosen));
    } catch (e: unknown) {
      setError(message(e));
    } finally {
      setWorking(null);
    }
  };

  const compact = () => run(t("Compacting the database…"), () => storageCompact().then(compactLine));
  const act = (kind: FreeUpKind) => {
    if (kind === "compact") return compact();
    if (kind === "recordings") return run(t("Deleting old recordings…"), () => storageDeleteOldRecordings(chosen).then(() => null));
    return run(t("Deleting old screen captures…"), () => storageDeleteOldCaptures(chosen).then(() => null));
  };

  if (!view) return <section className="card about">{error ? <p role="alert">{error}</p> : t("Loading…")}</section>;
  const [total, parts] = footprintLines(view);

  return (
    <section className="card about" aria-labelledby="storage-heading">
      <h2 id="storage-heading" className="section-label">
        {t("Storage")}
      </h2>
      <p>{total}</p>
      <p className="muted small">{parts}</p>
      {lowDiskLines(view.lowDisks).map((line) => (
        <p key={line} role="alert" className="storage-low">
          {line}
        </p>
      ))}
      <div className="toolbar">
        <button className="mode" disabled={working !== null} onClick={() => void compact()}>
          {t("Compact database")}
        </button>
        <button className="mode" aria-expanded={open} aria-controls="free-up-space" onClick={() => setOpen(!open)}>
          {t("Free up space…")}
        </button>
      </div>
      {open && (
        <div id="free-up-space" className="field">
          <div className="field-row">
            <label htmlFor="free-up-days">{t("Older than")}</label>
            <select id="free-up-days" className="input short" value={chosen} onChange={(e) => setDays(Number(e.target.value))}>
              {view.days.map((d) => (
                <option key={d} value={d}>
                  {t("{days} days", { days: String(d) })}
                </option>
              ))}
            </select>
          </div>
          {options === null ? (
            <p className="muted small">{t("Loading…")}</p>
          ) : freeUpRows(options).length === 0 ? (
            <p className="muted small">{t("Nothing to free up at {days} days.", { days: String(chosen) })}</p>
          ) : (
            freeUpRows(options).map((row) => (
              <div key={row.kind} className="field-row storage-row">
                <span className="grow">{row.detail}</span>
                {asking === row.kind && row.confirm ? (
                  <span className="toolbar" role="group" aria-label={row.confirm}>
                    <span className="small">{row.confirm}</span>
                    <button className="mode mode--active" disabled={working !== null} onClick={() => void act(row.kind)}>
                      {t("Delete")}
                    </button>
                    <button className="mode" onClick={() => setAsking(null)}>
                      {t("Cancel")}
                    </button>
                  </span>
                ) : (
                  <button className="mode" disabled={working !== null} onClick={() => (row.confirm ? setAsking(row.kind) : void act(row.kind))}>
                    {row.button}
                  </button>
                )}
              </div>
            ))
          )}
          <p className="muted small">{t("This doesn't change how long Whimbrel keeps data, and never touches a meeting being recorded.")}</p>
        </div>
      )}
      {working && (
        <p className="muted small" role="status">
          {working}
        </p>
      )}
      {result && !working && (
        <p className="muted small" role="status">
          {result}
        </p>
      )}
      {error && <p role="alert">{error}</p>}
    </section>
  );
}
