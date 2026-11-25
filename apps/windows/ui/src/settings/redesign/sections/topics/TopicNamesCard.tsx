import { useCallback, useEffect, useState } from "react";
import { profileGet, setTopicNames, setTopicStepsEnabled, startTopicSteps, topicNames, topicStepsEstimate, topicStepsState } from "../../../../api";
import { describeError, isCommandError } from "../../../../errors";
import type { TopicSortEstimate, TopicStepsView } from "../../../../generated/types";
import { t } from "../../../../l10n";
import { WButton, WNoticeBox, WSettingsCard, WSettingsControlRow, WSettingsList, WSettingsListRow, WSettingsNote, WSettingsToggleRow } from "../../../../system";
import { nameChips, needsLocalModel, stepsPrompt } from "../../../topicStepsModel";
import { TOPICS_REFRESH_MS } from "../../../topicsModel";
import "../meetings/meetops.css";

function message(e: unknown): string {
  return isCommandError(e) ? describeError(e) : String(e);
}

/** "Names I go by" and "Find steps with TypeSafe". Same commands and conditions as TopicStepsCard. A name already in
 *  the list is refused in words by the list's add field. */
export function TopicNamesCard() {
  const [names, setNames] = useState<string[] | null>(null);
  const [suggested, setSuggested] = useState<string[]>([]);
  const [profileName, setProfileName] = useState("");
  const [view, setView] = useState<TopicStepsView | null>(null);
  const [estimate, setEstimate] = useState<TopicSortEstimate | null>(null);
  const [estimateFailed, setEstimateFailed] = useState(false);
  const [startFailed, setStartFailed] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const refresh = useCallback(() => {
    topicStepsState().then(setView, (e: unknown) => setError(message(e)));
  }, []);
  useEffect(() => {
    topicNames().then(
      (v) => {
        setNames(v.names);
        setSuggested(v.suggested);
      },
      (e: unknown) => setError(message(e)),
    );
    profileGet().then((p) => setProfileName(p.name), () => {});
    refresh();
    const timer = setInterval(refresh, TOPICS_REFRESH_MS);
    return () => clearInterval(timer);
  }, [refresh]);

  const awaitingStart = !!view && view.enabled && !view.started && view.hasKey && !view.localOnly;
  const fetchEstimate = useCallback(() => {
    topicStepsEstimate().then(
      (e) => {
        setEstimate(e);
        setEstimateFailed(false);
      },
      () => setEstimateFailed(true),
    );
  }, []);
  useEffect(() => {
    if (!awaitingStart) return;
    fetchEstimate();
    const timer = setInterval(fetchEstimate, TOPICS_REFRESH_MS);
    return () => clearInterval(timer);
  }, [awaitingStart, fetchEstimate]);

  /** Resolves with a sentence when the save failed, so the add field can show it. */
  const saveNames = (next: string[]): Promise<string | null> =>
    setTopicNames(next).then(
      (v) => {
        setNames(v.names);
        setSuggested(v.suggested);
        setError(null);
        return null;
      },
      (e: unknown) => {
        const text = message(e);
        setError(text);
        return text;
      },
    );

  const prompt = view ? stepsPrompt(view, estimate, estimateFailed) : { kind: "none" as const };
  const chips = names ? nameChips(names, suggested, profileName) : [];

  return (
    <WSettingsCard id="topics.names" title={t("Names I go by")} headingLevel={3} description={t("Whimbrel uses these to tell steps you owe from steps owed to you.")} error={error}>
      {names ? (
        <WSettingsList
          label={t("Your names")}
          isEmpty={names.length === 0}
          empty={t("No names yet.")}
          add={{ label: t("Add a name"), placeholder: t("Add a name"), existing: names, onAdd: (name) => saveNames([...names, name]) }}
          footer={
            chips.length > 0 ? (
              <div className="rs-meetops-chips">
                {chips.map((chip) => (
                  <WButton key={chip} onClick={() => void saveNames([...names, chip])}>
                    {`+ ${chip}`}
                  </WButton>
                ))}
              </div>
            ) : undefined
          }
        >
          {names.map((name) => (
            <WSettingsListRow key={name} title={name} actions={<WButton aria-label={`${t("Remove")}, ${name}`} onClick={() => void saveNames(names.filter((n) => n !== name))}>{t("Remove")}</WButton>} />
          ))}
        </WSettingsList>
      ) : null}
      {view ? (
        <>
          <WSettingsToggleRow
            label={t("Find steps with TypeSafe")}
            description={t(
              "Sends short excerpts of meeting action items and Teams screens to TypeSafe to find steps you owe and are owed. Gmail and Google Calendar items wait until TypeSafe confirms it doesn't train on them. Off: your local model does this.",
            )}
            disabled={!view.hasKey}
            disabledReason={t("Needs a TypeSafe key. Add one in Settings → AI.")}
            checked={view.enabled}
            onToggle={(on) => {
              setEstimate(null);
              setEstimateFailed(false);
              setStartFailed(false);
              setTopicStepsEnabled(on).then(setView, (err: unknown) => setError(message(err)));
            }}
          />
          {view.enabled && view.localOnly ? <WSettingsNote>{t("Local-only mode is on, so nothing is sent to TypeSafe.")}</WSettingsNote> : null}
          {prompt.kind === "failed" ? (
            <WNoticeBox tone="error" actions={<WButton onClick={fetchEstimate}>{t("Try again")}</WButton>}>
              {t("Couldn't get an estimate. Try again.")}
            </WNoticeBox>
          ) : null}
          {prompt.kind === "sorting" ? <WSettingsNote>{t("Still sorting your items. The estimate will appear when that's done.")}</WSettingsNote> : null}
          {prompt.kind === "estimate" ? (
            <WSettingsControlRow label={prompt.text}>
              <WButton
                variant="primary"
                onClick={() =>
                  startTopicSteps(prompt.tokens).then(
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
          ) : null}
          {startFailed ? <WNoticeBox tone="error">{t("Couldn't start finding steps. Try again.")}</WNoticeBox> : null}
          {view.keyRejected ? <WNoticeBox tone="error">{t("TypeSafe rejected this key. Check it and save it again.")}</WNoticeBox> : null}
          {view.overBudget ? <WNoticeBox tone="error">{t("The budget for finding steps is used up. Turn the switch off and on again to see a new estimate.")}</WNoticeBox> : null}
          {needsLocalModel(view) ? <WSettingsNote>{t("Finding steps needs a local model (Settings → AI).")}</WSettingsNote> : null}
        </>
      ) : null}
    </WSettingsCard>
  );
}
