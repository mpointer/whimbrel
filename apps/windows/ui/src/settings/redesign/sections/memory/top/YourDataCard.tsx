// The Your data card, in the spec's order: counts, export (Obsidian), the Local-only line, then delete everything. The
// Local-only line is read-only here: the switch lives in AI, under one stored key.

import { useEffect, useState } from "react";
import { aiSettings, captureStatus, chooseVaultFolder, setVaultEnabled, vaultSettings } from "../../../../../api";
import { describeError, isCommandError } from "../../../../../errors";
import type { CaptureStatus, VaultView } from "../../../../../generated/types";
import { useL10n } from "../../../../../l10n";
import { WButton, WNoticeBox, WSettingsCard, WSettingsControlRow, WSettingsLinkRow, WSettingsNote, WSettingsToggleRow, WSettingsValueList } from "../../../../../system";
import { localOnlyLine } from "../../../../aiModel";
import { relativeTime } from "../../../../statusModel";
import { useJump } from "../../../jump";
import { DeleteEverythingRows } from "./DeleteEverythingRedesign";
import "./memtop.css";

const message = (e: unknown): string => (isCommandError(e) ? describeError(e) : String(e));

/** What Memory has done, in four counts. The same numbers the old Capture card showed. */
function Counts() {
  const { t } = useL10n();
  const [status, setStatus] = useState<CaptureStatus | null>(null);
  useEffect(() => {
    const read = () => captureStatus().then(setStatus, () => {});
    void read();
    const timer = setInterval(read, 5000);
    return () => clearInterval(timer);
  }, []);
  if (!status) return null;
  return (
    <WSettingsValueList
      label={t("Memory counts")}
      items={[
        { key: "stored", label: t("Stored"), value: String(status.stored) },
        { key: "unchanged", label: t("Skipped (unchanged)"), value: String(status.unchanged) },
        { key: "excluded", label: t("Skipped (excluded apps)"), value: String(status.excluded) },
        { key: "ocr", label: t("OCR fallbacks"), value: String(status.ocrRuns) },
      ]}
    />
  );
}

/** Export: keep a vault folder up to date (Obsidian). Polls every 30 s because the loop writes in the background. */
function ObsidianRows() {
  const { t, locale } = useL10n();
  const [view, setView] = useState<VaultView | null>(null);
  const [error, setError] = useState<string | null>(null);
  const jumped = useJump("capture.obsidian");
  useEffect(() => {
    const load = () => vaultSettings().then(setView, (e: unknown) => setError(message(e)));
    void load();
    const timer = setInterval(load, 30_000);
    return () => clearInterval(timer);
  }, []);
  const run = (p: Promise<VaultView>) =>
    p.then(
      (v) => {
        setView(v);
        setError(null);
      },
      (e: unknown) => setError(message(e)),
    );
  const chosen = view !== null && view.path.trim() !== "";
  if (!view) return error ? <WNoticeBox tone="error">{error}</WNoticeBox> : null;
  return (
    <>
      <WSettingsToggleRow id="capture.obsidian" highlighted={jumped} label={t("Keep an Obsidian vault up to date")} checked={view.enabled} error={error} onToggle={(on) => void run(setVaultEnabled(on))} />
      <WSettingsControlRow label={t("Vault folder")}>
        <WButton onClick={() => void run(chooseVaultFolder())}>{t("Choose folder…")}</WButton>
      </WSettingsControlRow>
      {chosen ? <WSettingsValueList label={t("Vault folder")} items={[{ key: "path", label: t("Folder"), value: view.path, mono: true }]} /> : null}
      {view.enabled && chosen && view.lastUpdatedUnixMs != null ? (
        <WSettingsNote>{t("Last updated {time}", { time: relativeTime(view.lastUpdatedUnixMs, Date.now(), locale) })}</WSettingsNote>
      ) : null}
      {view.enabled && chosen && view.failed ? (
        <WNoticeBox tone="error">{t("Couldn't update the vault. Check that the folder exists and Whimbrel can write to it.")}</WNoticeBox>
      ) : null}
    </>
  );
}

/** Your data: counts, export, whether anything leaves this PC (Local-only mode, with the way to AI settings), and delete everything. */
export function YourDataCard({ onOpenAi }: { onOpenAi: () => void }) {
  const { t } = useL10n();
  const [localOnly, setLocalOnly] = useState<boolean | null>(null);
  const jumped = useJump("capture.localOnly");
  useEffect(() => {
    void aiSettings().then(
      (s) => setLocalOnly(s.routing.localOnly),
      () => {},
    );
  }, []);
  return (
    <WSettingsCard id="capture.yourData" headingLevel={3} title={t("Your data")} description={t("Counts, export, and deleting what Whimbrel keeps on this PC. Whether it sends anything to a hosted provider is set in AI.")}>
      <Counts />
      <ObsidianRows />
      {localOnly !== null ? (
        <WSettingsLinkRow id="capture.localOnly" highlighted={jumped} kind="section" label={localOnlyLine(localOnly)} linkText={t("Open AI settings")} onPress={onOpenAi} />
      ) : null}
      <DeleteEverythingRows />
    </WSettingsCard>
  );
}
