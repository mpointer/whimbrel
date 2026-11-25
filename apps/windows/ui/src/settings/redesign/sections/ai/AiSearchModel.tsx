import { useEffect } from "react";
import { t } from "../../../../l10n";
import { WButton, WNoticeBox, WSettingsCard, WSettingsControlRow, WSettingsProgressRow, WSettingsStatusRow, WSettingsValueList } from "../../../../system";
import { percent, searchModelState } from "../../../searchModel";
import { searchModelDownload } from "../../../searchModelDownload";

/** The multilingual embedding model: downloaded on first run, not bundled. Its download is shared with the launch download. */
export function AiSearchModelCard() {
  const { status, job, error } = searchModelDownload.use();
  // Whether it has loaded since the last read (it loads on the first search after the download).
  useEffect(() => {
    void searchModelDownload.refresh();
  }, []);

  if (!status) return <WSettingsCard id="ai.semanticSearch" title={t("Semantic search")} headingLevel={3} loading={!error} error={error} />;
  const state = searchModelState(status, job);
  return (
    <WSettingsCard id="ai.semanticSearch" title={t("Semantic search")} headingLevel={3} error={error}>
      {state === "ready" && (
        <WSettingsStatusRow
          label={t("Search model")}
          description={t("The search model is ready. Library search can match by meaning, across languages.")}
          status={{ tone: "good", children: t("Ready") }}
        />
      )}
      {(state === "missing" || state === "failed") && (
        <>
          {job?.error && <WNoticeBox tone="error">{job.error}</WNoticeBox>}
          <WSettingsControlRow
            label={t("Search model")}
            description={t(
              "Searching by meaning needs a multilingual model ({model}, about 470 MB), downloaded once from Hugging Face. Keyword search works without it.",
              { model: "paraphrase-multilingual-MiniLM-L12-v2" },
            )}
          >
            <WButton variant="primary" onClick={() => void searchModelDownload.start()}>
              {state === "failed" ? t("Try again") : t("Download search model")}
            </WButton>
          </WSettingsControlRow>
        </>
      )}
      {state === "downloading" && job && (
        <WSettingsProgressRow label={t("Search model")} progress={job.fraction} valueText={`${job.status} ${percent(job.fraction)}`} onCancel={searchModelDownload.cancel} />
      )}
      <WSettingsValueList label={t("Search model")} items={[{ key: "dir", label: t("Folder"), value: status.dir, mono: true, copy: true }]} />
    </WSettingsCard>
  );
}
