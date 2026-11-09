import { useCallback, useEffect, useState } from "react";
import { profileGet, setTopicNames, setTopicStepsEnabled, startTopicSteps, topicNames, topicStepsEstimate, topicStepsState } from "../api";
import { describeError, isCommandError } from "../errors";
import type { TopicSortEstimate, TopicStepsView } from "../generated/types";
import { t } from "../l10n";
import { nameChips, needsLocalModel, stepsPrompt } from "./topicStepsModel";
import { TOPICS_REFRESH_MS } from "./topicsModel";

function message(e: unknown): string {
  return isCommandError(e) ? describeError(e) : String(e);
}

/**
 * Settings → Topics, below the topics (topics piece 2, the Mac's `TopicsSettingsPane`): "Names I go
 * by", which tells steps the user owes from steps owed to them, and "Find steps with TypeSafe". The
 * switch and Start stay on this PC; the names sync.
 */
export function TopicStepsCard() {
  const [names, setNames] = useState<string[] | null>(null);
  const [suggested, setSuggested] = useState<string[]>([]);
  const [profileName, setProfileName] = useState("");
  const [newName, setNewName] = useState("");
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

  const saveNames = (next: string[]) =>
    setTopicNames(next).then(
      (v) => {
        setNames(v.names);
        setSuggested(v.suggested);
        setError(null);
      },
      (e: unknown) => setError(message(e)),
    );
  const addName = () => {
    const name = newName.trim();
    if (!name || !names) return;
    setNewName("");
    void saveNames([...names, name]);
  };

  const prompt = view ? stepsPrompt(view, estimate, estimateFailed) : { kind: "none" as const };
  const chips = names ? nameChips(names, suggested, profileName) : [];

  return (
    <section className="card about" aria-labelledby="topic-steps-heading">
      <h2 id="topic-steps-heading" className="section-label">
        {t("Names I go by")}
      </h2>
      <p className="muted small">{t("Whimbrel uses these to tell steps you owe from steps owed to you.")}</p>
      {error && <p role="alert">{error}</p>}
      {names && (
        <ul className="topic-recent">
          {names.map((name) => (
            <li key={name} className="field-row">
              <span className="grow">{name}</span>
              <button className="mode" onClick={() => void saveNames(names.filter((n) => n !== name))}>
                {t("Remove")}
              </button>
            </li>
          ))}
        </ul>
      )}
      <form
        className="field-row"
        onSubmit={(e) => {
          e.preventDefault();
          addName();
        }}
      >
        <input className="input grow" value={newName} placeholder={t("Add a name")} aria-label={t("Add a name")} onChange={(e) => setNewName(e.target.value)} />
        <button type="submit" className="mode" disabled={!newName.trim() || !names}>
          {t("Add")}
        </button>
      </form>
      {chips.length > 0 && names && (
        <div className="chips">
          {chips.map((chip) => (
            <button key={chip} className="mode" onClick={() => void saveNames([...names, chip])}>
              {`+ ${chip}`}
            </button>
          ))}
        </div>
      )}

      {view && (
        <>
          <label className="toggle">
            <input
              type="checkbox"
              disabled={!view.hasKey}
              checked={view.enabled}
              onChange={(e) => {
                setEstimate(null);
                setEstimateFailed(false);
                setStartFailed(false);
                setTopicStepsEnabled(e.target.checked).then(setView, (err: unknown) => setError(message(err)));
              }}
            />
            {t("Find steps with TypeSafe")}
          </label>
          <p className="muted small">
            {t(
              "Sends short excerpts of topic emails, meeting action items and Teams screens to TypeSafe to find steps you owe and are owed. Off: your local model does this.",
            )}
          </p>
          {view.enabled && view.localOnly && <p className="muted small">{t("Local-only mode is on, so nothing is sent to TypeSafe.")}</p>}
          {prompt.kind === "failed" && (
            <div className="field-row">
              <span role="alert" className="small">
                {t("Couldn't get an estimate. Try again.")}
              </span>
              <button type="button" className="mode" onClick={fetchEstimate}>
                {t("Try again")}
              </button>
            </div>
          )}
          {prompt.kind === "sorting" && <p className="muted small">{t("Still sorting your items. The estimate will appear when that's done.")}</p>}
          {prompt.kind === "estimate" && (
            <div className="field-row">
              <span className="muted small">{prompt.text}</span>
              <button
                type="button"
                className="mode"
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
              </button>
            </div>
          )}
          {startFailed && (
            <p role="alert" className="small">
              {t("Couldn't start finding steps. Try again.")}
            </p>
          )}
          {view.keyRejected && (
            <p role="alert" className="small">
              {t("TypeSafe rejected this key. Check it and save it again.")}
            </p>
          )}
          {view.overBudget && (
            <p role="alert" className="small">
              {t("The budget for finding steps is used up. Turn the switch off and on again to see a new estimate.")}
            </p>
          )}
          {needsLocalModel(view) && <p className="muted small">{t("Finding steps needs a local model (Settings → AI).")}</p>}
        </>
      )}
    </section>
  );
}
