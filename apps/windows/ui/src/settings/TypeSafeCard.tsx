import { useCallback, useEffect, useState } from "react";
import { setTopicSortEnabled, setTypesafeKey, startTopicSort, topicSortEstimate, topicSortState } from "../api";
import { describeError, isCommandError } from "../errors";
import type { TopicSortEstimate, TopicSortView } from "../generated/types";
import { t } from "../l10n";
import { keyLine } from "./aiModel";
import { sortPrompt, TOPICS_REFRESH_MS } from "./topicsModel";

function message(e: unknown): string {
  return isCommandError(e) ? describeError(e) : String(e);
}

/** Settings → AI → TypeSafe: the key and "Sort items into topics with TypeSafe" (the Mac's
 *  AIProvidersSettingsPane). The key and the switch stay on this PC. */
export function TypeSafeCard() {
  const [view, setView] = useState<TopicSortView | null>(null);
  const [estimate, setEstimate] = useState<TopicSortEstimate | null>(null);
  const [estimateFailed, setEstimateFailed] = useState(false);
  const [startFailed, setStartFailed] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [draft, setDraft] = useState("");

  const refresh = useCallback(() => {
    topicSortState().then(setView, (e: unknown) => setError(message(e)));
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

  if (!view) return <section className="card about">{error ? <p role="alert">{error}</p> : t("Loading…")}</section>;

  const done = (p: Promise<TopicSortView>) =>
    p.then(
      (v) => {
        setView(v);
        setError(null);
      },
      (e: unknown) => setError(message(e)),
    );
  const prompt = sortPrompt(view, estimate, estimateFailed);

  return (
    <section className="card about" aria-labelledby="typesafe-heading">
      <h2 id="typesafe-heading" className="section-label">
        {t("TypeSafe")}
      </h2>
      {error && <p role="alert">{error}</p>}
      <form
        className="field-row key-row"
        onSubmit={(e) => {
          e.preventDefault();
          if (draft.trim()) done(setTypesafeKey(draft)).then(() => setDraft(""));
        }}
      >
        <span className="key-name">{t("TypeSafe API Key")}</span>
        <span className="muted small mono key-line">{keyLine({ masked: view.key })}</span>
        <input
          className="input grow"
          type="password"
          autoComplete="off"
          placeholder={view.key ? t("Replace key") : t("TypeSafe API Key")}
          aria-label={t("TypeSafe API Key")}
          value={draft}
          onChange={(e) => setDraft(e.target.value)}
        />
        <button type="submit" className="mode" disabled={!draft.trim()}>
          {t("Save")}
        </button>
        {view.key && (
          <button type="button" className="mode" onClick={() => done(setTypesafeKey(null))}>
            {t("Remove")}
          </button>
        )}
      </form>
      <label className="toggle">
        <input
          type="checkbox"
          disabled={!view.key}
          checked={view.enabled}
          onChange={(e) => {
            setEstimate(null);
            setEstimateFailed(false);
            setStartFailed(false);
            done(setTopicSortEnabled(e.target.checked));
          }}
        />
        {t("Sort items into topics with TypeSafe")}
      </label>
      <p className="muted small">
        {t("Sends short excerpts of emails, meetings, calendar events and screen activity to TypeSafe to choose a topic.")}
      </p>
      {view.enabled && view.localOnly && <p className="muted small">{t("Local-only mode is on, so nothing is sent to TypeSafe.")}</p>}
      {prompt.kind === "failed" && (
        <div className="field-row">
          <span role="alert" className="small">{t("Couldn't get an estimate. Try again.")}</span>
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
          </button>
        </div>
      )}
      {startFailed && <p role="alert" className="small">{t("Couldn't start topic sorting. Try again.")}</p>}
      {view.keyRejected && <p role="alert" className="small">{t("TypeSafe rejected this key. Check it and save it again.")}</p>}
      {view.overBudget && (
        <p role="alert" className="small">{t("The topic sorting budget is used up. Turn the switch off and on again to see a new estimate.")}</p>
      )}
    </section>
  );
}
