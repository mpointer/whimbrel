import { useCallback, useEffect, useState } from "react";
import { setTopicSortEnabled, setTypesafeKey, startTopicSort, topicSortEstimate, topicSortState } from "../../../../api";
import type { TopicSortEstimate, TopicSortView } from "../../../../generated/types";
import { t } from "../../../../l10n";
import { WButton, WNoticeBox, WSettingsCard, WSettingsControlRow, WSettingsNote, WSettingsSecretRow, WSettingsToggleRow } from "../../../../system";
import { sortPrompt, TOPICS_REFRESH_MS } from "../../../topicsModel";
import { aiMessage } from "./useAi";

/** TypeSafe: its key and "Sort items into topics with TypeSafe". The key and the switch stay on this PC. */
export function AiTypeSafeCard() {
  const [view, setView] = useState<TopicSortView | null>(null);
  const [estimate, setEstimate] = useState<TopicSortEstimate | null>(null);
  const [estimateFailed, setEstimateFailed] = useState(false);
  const [startFailed, setStartFailed] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const refresh = useCallback(() => {
    topicSortState().then(setView, (e: unknown) => setError(aiMessage(e)));
  }, []);
  useEffect(() => {
    refresh();
    const timer = setInterval(refresh, TOPICS_REFRESH_MS);
    return () => clearInterval(timer);
  }, [refresh]);

  const awaitingStart = !!view && view.enabled && !view.started && !!view.key && !view.localOnly;
  const fetchEstimate = useCallback(() => {
    topicSortEstimate().then(
      (e) => {
        setEstimate(e);
        setEstimateFailed(false);
      },
      () => setEstimateFailed(true),
    );
  }, []);
  // While waiting on Start, ask again every 30 s so "Still sorting" turns into a price.
  useEffect(() => {
    if (!awaitingStart) return;
    fetchEstimate();
    const timer = setInterval(fetchEstimate, TOPICS_REFRESH_MS);
    return () => clearInterval(timer);
  }, [awaitingStart, fetchEstimate]);

  if (!view) return <WSettingsCard id="ai.typesafe" title={t("TypeSafe")} headingLevel={3} loading={!error} error={error} />;

  const done = (p: Promise<TopicSortView>): Promise<boolean> =>
    p.then(
      (v) => {
        setView(v);
        setError(null);
        return true;
      },
      (e: unknown) => {
        setError(aiMessage(e));
        return false;
      },
    );
  const prompt = sortPrompt(view, estimate, estimateFailed);
  const saveKey = (key: string | null): Promise<boolean> => {
    setBusy(true);
    return done(setTypesafeKey(key)).finally(() => setBusy(false));
  };

  return (
    <WSettingsCard id="ai.typesafe" title={t("TypeSafe")} headingLevel={3} error={error}>
      <WSettingsSecretRow
        label={t("TypeSafe API Key")}
        saved={!!view.key}
        onSave={saveKey}
        onRemove={() => void saveKey(null)}
        removeQuestion={t("Remove the TypeSafe key from this PC?")}
        placeholder={t("TypeSafe API Key")}
        busy={busy}
      />
      <WSettingsToggleRow
        label={t("Sort items into topics with TypeSafe")}
        description={t(
          "Sends short excerpts of meetings and screen activity to TypeSafe to choose a topic. Gmail and Google Calendar items wait until TypeSafe confirms it doesn't train on them.",
        )}
        checked={view.enabled}
        disabled={!view.key}
        disabledReason={t("Save a TypeSafe API key first.")}
        onToggle={(next) => {
          setEstimate(null);
          setEstimateFailed(false);
          setStartFailed(false);
          void done(setTopicSortEnabled(next));
        }}
      />
      {view.enabled && view.localOnly && <WSettingsNote>{t("Local-only mode is on, so nothing is sent to TypeSafe.")}</WSettingsNote>}
      {prompt.kind === "failed" && (
        <WSettingsControlRow label={t("Topic estimate")} error={t("Couldn't get an estimate. Try again.")}>
          <WButton onClick={fetchEstimate}>{t("Try again")}</WButton>
        </WSettingsControlRow>
      )}
      {prompt.kind === "sorting" && <WSettingsNote>{t("Still sorting your items. The estimate will appear when that's done.")}</WSettingsNote>}
      {prompt.kind === "estimate" && (
        <WSettingsControlRow label={t("Topic estimate")} description={prompt.text}>
          <WButton
            variant="primary"
            onClick={() =>
              startTopicSort(prompt.tokens).then(
                (v) => {
                  setView(v);
                  setStartFailed(false);
                  setEstimate(null);
                },
                () => setStartFailed(true),
              )
            }
          >
            {t("Start")}
          </WButton>
        </WSettingsControlRow>
      )}
      {startFailed && <WNoticeBox tone="error">{t("Couldn't start topic sorting. Try again.")}</WNoticeBox>}
      {view.keyRejected && <WNoticeBox tone="error">{t("TypeSafe rejected this key. Check it and save it again.")}</WNoticeBox>}
      {view.overBudget && <WNoticeBox tone="error">{t("The topic sorting budget is used up. Turn the switch off and on again to see a new estimate.")}</WNoticeBox>}
    </WSettingsCard>
  );
}
