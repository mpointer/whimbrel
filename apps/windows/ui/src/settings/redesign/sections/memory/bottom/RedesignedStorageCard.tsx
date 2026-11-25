import { useCallback, useEffect, useState } from "react";
import { storageCompact, storageCompressRecordings, storageDeleteOldCaptures, storageDeleteOldRecordings, storageFreeUpOptions, storageOverview } from "../../../../../api";
import { describeError, isCommandError } from "../../../../../errors";
import type { FreeUpOptions, StorageView } from "../../../../../generated/types";
import { useL10n } from "../../../../../l10n";
import { WButton, WInlineConfirm, WNoticeBox, WSettingsCard, WSettingsControlRow, WSettingsDisclosure, WSettingsNote, WSettingsProgressRow, WSettingsRow, WSettingsSelectRow } from "../../../../../system";
import { freedLine, type FreeUpKind } from "../../../../storageModel";
import { compactFrees, compactingLabel, compactLabel, compactResult, compressResult, redesignedFreeUpRows, storageSummary } from "./storageWording";
import "./bottom.css";

function message(e: unknown): string {
  return isCommandError(e) ? describeError(e) : String(e);
}

/**
 * Storage: what Whimbrel keeps on this PC, a low-disk warning, "Compact your library" and "Free up space". Free up space
 * never changes the periods in Keeping data and never touches the meeting being recorded. The two deletes keep their
 * inline question.
 */
export function RedesignedStorageCard() {
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

  const compact = () => run(compactingLabel(), () => storageCompact().then(compactResult));
  const act = (kind: FreeUpKind) => {
    if (kind === "compact") return compact();
    if (kind === "compress") return run(t("Compressing recordings… This can take a few minutes."), () => storageCompressRecordings().then(compressResult));
    if (kind === "recordings") return run(t("Deleting old recordings…"), () => storageDeleteOldRecordings(chosen).then(() => null));
    return run(t("Deleting old screen captures…"), () => storageDeleteOldCaptures(chosen).then(() => null));
  };

  const summary = view ? storageSummary(view) : null;
  const rows = options ? redesignedFreeUpRows(options) : null;
  const busyReason = working ?? undefined;

  return (
    <WSettingsCard id="capture.storage" title={t("Storage")} headingLevel={3} loading={!view && !error} error={error}>
      {view && summary && (
        <>
          <WSettingsControlRow label={summary.total} description={summary.parts}>
            {null}
          </WSettingsControlRow>
          {summary.lowDisks.map((line) => (
            <div className="rs-membottom-block" key={line}>
              <WNoticeBox tone="warning" live>
                {line}
              </WNoticeBox>
            </div>
          ))}
          <WSettingsRow
            variant="button"
            label={compactLabel()}
            description={view.dbFreeBytes > 0 ? compactFrees(view.dbFreeBytes) : undefined}
            actionLabel={t("Compact")}
            disabled={working !== null}
            disabledReason={busyReason}
            onPress={() => void compact()}
          />
          <WSettingsDisclosure title={t("Free up space")} open={open} onToggle={setOpen}>
            <WSettingsSelectRow
              label={t("Older than")}
              value={String(chosen)}
              options={view.days.map((d) => ({ value: String(d), label: t("{days} days", { days: String(d) }) }))}
              onChange={(v) => setDays(Number(v))}
            />
            {rows === null ? (
              <WSettingsNote>{t("Loading…")}</WSettingsNote>
            ) : rows.length === 0 ? (
              <WSettingsNote>{t("Nothing to free up at {days} days.", { days: String(chosen) })}</WSettingsNote>
            ) : (
              rows.map((row) => (
                <WSettingsControlRow key={row.kind} label={row.detail} stacked={asking === row.kind}>
                  {asking === row.kind && row.confirm ? (
                    <WInlineConfirm
                      question={row.confirm}
                      confirmLabel={t("Delete")}
                      busy={working !== null}
                      returnFocusTo={`rs-membottom-free-${row.kind}`}
                      onConfirm={() => void act(row.kind)}
                      onCancel={() => setAsking(null)}
                    />
                  ) : (
                    <WButton
                      id={`rs-membottom-free-${row.kind}`}
                      aria-label={`${row.button}: ${row.detail}`}
                      disabled={working !== null}
                      onClick={() => (row.confirm ? setAsking(row.kind) : void act(row.kind))}
                    >
                      {row.button}
                    </WButton>
                  )}
                </WSettingsControlRow>
              ))
            )}
            <WSettingsNote>{t("This doesn't change how long Whimbrel keeps data, and never touches a meeting being recorded.")}</WSettingsNote>
          </WSettingsDisclosure>
          {working && <WSettingsProgressRow label={working} progress={null} />}
          {result && !working && (
            <div role="status">
              <WSettingsNote>{result}</WSettingsNote>
            </div>
          )}
        </>
      )}
    </WSettingsCard>
  );
}
