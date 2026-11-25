// The Storage card's words. The old model's lines (storageModel.ts) are used where they name no database; the three that
// do (the parts line, "Frees ... inside the database file", the compact label) have their own redesign keys that say
// "your library". Nothing here rewrites English text, so every language gets the same words.

import type { CompactView, CompressView, FreeUpOptions, StorageView } from "../../../../../generated/types";
import { t } from "../../../../../l10n";
import { compactLine, compressLine, footprintLines, freeUpRows, lowDiskLines, type FreeUpRow } from "../../../../storageModel";
import { diskSize } from "../../../../statusModel";

export function storageSummary(view: StorageView): { total: string; parts: string; lowDisks: string[] } {
  const f = view.footprint;
  const [total] = footprintLines(view);
  const parts = t("Your library {library} · Recordings {recordings} · Speech models {models} · Other {other}", {
    library: diskSize(f.database),
    recordings: diskSize(f.recordings),
    models: diskSize(f.models),
    other: diskSize(f.other),
  });
  return { total, parts, lowDisks: lowDiskLines(view.lowDisks) };
}

export const compactLabel = () => t("Compact your library");
export const compactingLabel = () => t("Compacting your library…");
export const compactFrees = (bytes: number) => t("Frees {free} inside your library file.", { free: diskSize(bytes) });

export const redesignedFreeUpRows = (options: FreeUpOptions): FreeUpRow[] =>
  freeUpRows(options).map((row) => (row.kind === "compact" ? { ...row, detail: compactFrees(options.compactBytes ?? 0) } : row));

export const compactResult = (r: CompactView): string => compactLine(r);
export const compressResult = (r: CompressView): string => compressLine(r);
