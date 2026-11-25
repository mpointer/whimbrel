import type { ForeignFilesView } from "../../../../../generated/types";
import { t } from "../../../../../l10n";
import { confirmLine, summaryLine } from "../../../../foreignFilesModel";

export const keptAsideExplainer = (): string => t("Files Whimbrel set aside beside your library while undoing or repairing encryption. Whimbrel does not use them.");
export const keptAsideSummary = (v: ForeignFilesView): string => summaryLine(v);
export const keptAsideConfirmBody = (v: ForeignFilesView): string => confirmLine(v);
