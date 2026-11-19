import type { CompactView, CompressView, FreeUpOptions, LowDisk, StorageView } from "../generated/types";
import { t } from "../l10n";
import { diskSize } from "./statusModel";

/** "Whimbrel uses X on this PC." and the line per part. */
export function footprintLines(view: StorageView): string[] {
  const f = view.footprint;
  return [
    t("Whimbrel uses {total} on this PC.", { total: diskSize(f.total) }),
    t("Database {database} · Recordings {recordings} · Speech models {models} · Other {other}", {
      database: diskSize(f.database),
      recordings: diskSize(f.recordings),
      models: diskSize(f.models),
      other: diskSize(f.other),
    }),
  ];
}

/** One warning per drive under 10 GB free. Recording isn't blocked; this says why to act. */
export function lowDiskLines(disks: LowDisk[]): string[] {
  return disks.map((d) =>
    t("Only {free} free on {volume}. Whimbrel needs room to record. Free some space or move the data folder.", {
      free: diskSize(d.freeBytes),
      volume: d.volume,
    }),
  );
}

/** What a compaction did, as the card says it. */
export function compactLine(result: CompactView): string {
  if (result.ran && result.spacePending) return t("Compacted. The freed space returns to the disk shortly.");
  if (result.ran) return t("Compacted: {before} → {after}.", { before: diskSize(result.fileBytesBefore), after: diskSize(result.fileBytesAfter) });
  if (result.reason === "not_enough_disk") return t("Not enough free disk to compact. Free {need} and try again.", { need: diskSize(result.needBytes ?? 0) });
  if (result.reason === "recording") return t("Not compacted while a meeting is recording. Try again when it ends.");
  return t("Nothing to compact yet.");
}

export type FreeUpKind = "compact" | "compress" | "recordings" | "captures";

export interface FreeUpRow {
  kind: FreeUpKind;
  detail: string;
  button: string;
  /** The question asked before it runs; none for compacting, which deletes nothing. */
  confirm: string | null;
}

/** The Free up space rows at the chosen days, leaving out any that would free nothing. */
export function freeUpRows(options: FreeUpOptions): FreeUpRow[] {
  const rows: FreeUpRow[] = [];
  const days = String(options.days);
  if (options.compactBytes) {
    rows.push({ kind: "compact", detail: t("Frees {free} inside the database file.", { free: diskSize(options.compactBytes) }), button: t("Compact"), confirm: null });
  }
  if (options.compressBytes) {
    rows.push({
      kind: "compress",
      detail: t("{bytes} of recordings are full quality. Compressing keeps them for transcribing again at about a tenth of the size.", {
        bytes: diskSize(options.compressBytes),
      }),
      button: t("Compress all"),
      confirm: null,
    });
  }
  if (options.recordingsBytes) {
    rows.push({
      kind: "recordings",
      detail: t("Recordings older than {days} days: {bytes}. Transcripts, notes and summaries stay.", { days, bytes: diskSize(options.recordingsBytes) }),
      button: t("Delete…"),
      confirm: t("Delete the audio of meetings older than {days} days? This can't be undone.", { days }),
    });
  }
  if (options.capturesCount) {
    rows.push({
      kind: "captures",
      detail: t("{count} screen captures older than {days} days (about {bytes} of text).", {
        count: options.capturesCount,
        days,
        bytes: diskSize(options.capturesBytes ?? 0),
      }),
      button: t("Delete…"),
      confirm: t("Delete screen captures older than {days} days from this PC and your other devices? This can't be undone.", { days }),
    });
  }
  return rows;
}

/** What "Compress all" did. Any it couldn't compress are left as they were. */
export function compressLine(result: CompressView): string {
  const parts: string[] = [];
  if (result.compressed > 0) parts.push(t("Meetings compressed: {count}.", { count: result.compressed }));
  if (result.failed > 0) {
    parts.push(
      result.error
        ? t("Meetings left unchanged: {count}. {error}", { count: result.failed, error: result.error })
        : t("Meetings left unchanged: {count}.", { count: result.failed }),
    );
  }
  return parts.join(" ") || t("Nothing to compress.");
}

/** "Freed X" from the footprint before and after a cleanup; nothing when it didn't shrink. */
export function freedLine(before: number, after: number): string | null {
  const freed = before - after;
  return freed > 0 ? t("Freed {bytes}.", { bytes: diskSize(freed) }) : null;
}
