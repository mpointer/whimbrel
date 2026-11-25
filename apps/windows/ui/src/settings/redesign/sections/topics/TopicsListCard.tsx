import { Fragment, useCallback, useEffect, useState, type FormEvent } from "react";
import { acceptTopicSuggestion, dismissTopicSuggestion, mergeTopicSeeds, saveTopicSeed, topicNamingStatus, topicSeeds, topicSuggestions } from "../../../../api";
import type { TopicSeedView, TopicSuggestionView } from "../../../../generated/types";
import { t } from "../../../../l10n";
import { WButton, WSettingsCard, WSettingsList, WSettingsListRow } from "../../../../system";
import { itemCount, kindLabel, parseAliases, seedDetail, TOPIC_KINDS, TOPICS_REFRESH_MS } from "../../../topicsModel";
import "../meetings/meetops.css";
import { topicNameProblem } from "./topicsRedesignModel";

/** What the editor is editing: a new topic, or an existing one. */
type Editing = { seed: TopicSeedView | null };

/** Add or edit a topic: a name, other names separated by commas, and a kind. A name that another topic already has is refused in words. */
export function TopicEditor(props: {
  seed: TopicSeedView | null;
  seeds: readonly TopicSeedView[];
  onCancel: () => void;
  onSave: (input: { name: string; aliases: string[]; kind: string | null }) => void;
}) {
  const [name, setName] = useState(props.seed?.name ?? "");
  const [aliases, setAliases] = useState(props.seed?.aliases.join(", ") ?? "");
  const [kind, setKind] = useState(props.seed?.kind ?? "");
  const [problem, setProblem] = useState<string | null>(null);
  const submit = (e?: FormEvent) => {
    e?.preventDefault();
    if (!name.trim()) return;
    const bad = topicNameProblem(name, props.seeds, props.seed?.key ?? null);
    setProblem(bad);
    if (bad) return;
    props.onSave({ name: name.trim(), aliases: parseAliases(aliases), kind: kind || null });
  };
  return (
    <form className="rs-meetops-editor" onSubmit={submit}>
      <h4 className="w-setting-label">{props.seed ? t("Edit topic") : t("Add topic")}</h4>
      <label>
        <span className="w-setting-label">{t("Name")}</span>
        <input
          className="w-input"
          value={name}
          aria-invalid={problem ? true : undefined}
          onChange={(e) => {
            setName(e.target.value);
            setProblem(null);
          }}
        />
      </label>
      {problem ? (
        <p className="w-field-error" role="alert">
          {problem}
        </p>
      ) : null}
      <label>
        <span className="w-setting-label">{t("Aliases, separated by commas")}</span>
        <input className="w-input" value={aliases} onChange={(e) => setAliases(e.target.value)} />
      </label>
      <label>
        <span className="w-setting-label">{t("Kind")}</span>
        <select className="w-input w-select" value={kind} onChange={(e) => setKind(e.target.value)}>
          <option value="">{t("None")}</option>
          {TOPIC_KINDS.map((k) => (
            <option key={k} value={k}>
              {kindLabel(k)}
            </option>
          ))}
        </select>
      </label>
      <div className="rs-meetops-line">
        <WButton type="submit" variant="primary" disabled={!name.trim()}>
          {t("Save")}
        </WButton>
        <WButton onClick={props.onCancel}>{t("Cancel")}</WButton>
      </div>
    </form>
  );
}

/** Topics card: your topics (add, edit, merge, delete note) and the suggested ones. Same commands and conditions as TopicsCard. */
export function TopicsListCard() {
  const [seeds, setSeeds] = useState<TopicSeedView[]>([]);
  const [suggestions, setSuggestions] = useState<TopicSuggestionView[]>([]);
  const [placeholders, setPlaceholders] = useState(0);
  const [editing, setEditing] = useState<Editing | null>(null);
  const [deleting, setDeleting] = useState<string | null>(null);
  const [merging, setMerging] = useState<string | null>(null);
  const [mergeInto, setMergeInto] = useState("");
  const [names, setNames] = useState<Record<number, string>>({});
  /** A suggestion whose name another topic already has is refused in words, as Add and Edit do. */
  const [problems, setProblems] = useState<Record<number, string>>({});
  const [error, setError] = useState<string | null>(null);

  const reload = useCallback(() => {
    topicSeeds().then(setSeeds, () => {});
    topicSuggestions().then(setSuggestions, () => {});
    topicNamingStatus().then((v) => setPlaceholders(v.placeholderCount), () => {});
  }, []);
  useEffect(() => {
    reload();
    const timer = setInterval(reload, TOPICS_REFRESH_MS);
    return () => clearInterval(timer);
  }, [reload]);

  /** Runs a write, then reloads; shows `failure` if it fails. */
  const run = (work: Promise<unknown>, failure: string) =>
    work.then(
      () => {
        setError(null);
        reload();
      },
      () => {
        setError(failure);
        reload();
      },
    );

  return (
    <WSettingsCard
      id="topics.topics"
      title={t("Topics")}
      headingLevel={3}
      description={t("Name the customers, products and projects you care about. Whimbrel groups your meetings, emails and notes under them.")}
      error={error}
      actions={<WButton onClick={() => setEditing({ seed: null })}>{t("Add topic")}</WButton>}
    >
      {editing && !editing.seed ? (
        <TopicEditor
          seed={null}
          seeds={seeds}
          onCancel={() => setEditing(null)}
          onSave={(input) => {
            setEditing(null);
            run(saveTopicSeed({ key: null, ...input }), t("Could not save that topic."));
          }}
        />
      ) : null}
      <WSettingsList label={t("Your topics")} isEmpty={seeds.length === 0} empty={t("No topics yet. Add one, or accept a suggestion below.")}>
        {seeds.map((seed) => (
          <Fragment key={seed.key}>
            <WSettingsListRow
              title={seed.name}
              caption={seedDetail(seed) || undefined}
              chip={{ tone: "neutral", children: itemCount(seed.itemCount) }}
              actions={
                <>
                  <WButton onClick={() => setEditing({ seed })} aria-label={`${t("Edit")}, ${seed.name}`}>
                    {t("Edit")}
                  </WButton>
                  {seeds.length > 1 ? (
                    <WButton
                      aria-label={`${t("Merge into…")}, ${seed.name}`}
                      onClick={() => {
                        setMerging(seed.key);
                        setDeleting(null);
                        setMergeInto(seeds.find((s) => s.key !== seed.key)?.key ?? "");
                      }}
                    >
                      {t("Merge into…")}
                    </WButton>
                  ) : null}
                  <WButton
                    aria-label={`${t("Delete")}, ${seed.name}`}
                    onClick={() => {
                      setDeleting(seed.key);
                      setMerging(null);
                    }}
                  >
                    {t("Delete")}
                  </WButton>
                </>
              }
            />
            {editing?.seed?.key === seed.key ? (
              <li className="rs-meetops-li">
              <TopicEditor
                seed={seed}
                seeds={seeds}
                onCancel={() => setEditing(null)}
                onSave={(input) => {
                  setEditing(null);
                  run(saveTopicSeed({ key: seed.key, ...input }), t("Could not save that topic."));
                }}
              />
              </li>
            ) : null}
            {/* A topic you named is a client: deleting it here would keep its name and aliases, so this only points to the client's page. */}
            {deleting === seed.key ? (
              <li className="rs-meetops-panel" role="note">
                <span>{t("To delete a client and what Whimbrel holds on them, open the client's page and choose Delete this client.")}</span>
                <WButton onClick={() => setDeleting(null)}>{t("Close")}</WButton>
              </li>
            ) : null}
            {merging === seed.key ? (
              <li className="rs-meetops-panel">
                <span>{t("Merge {name} into", { name: seed.name })}</span>
                <select className="w-input w-select" aria-label={t("Merge into")} value={mergeInto} onChange={(e) => setMergeInto(e.target.value)}>
                  {seeds
                    .filter((s) => s.key !== seed.key)
                    .map((s) => (
                      <option key={s.key} value={s.key}>
                        {s.name}
                      </option>
                    ))}
                </select>
                <WButton
                  variant="primary"
                  disabled={!mergeInto}
                  onClick={() => {
                    const keep = mergeInto;
                    setMerging(null);
                    run(mergeTopicSeeds(keep, seed.key), t("Could not merge those topics."));
                  }}
                >
                  {t("Merge")}
                </WButton>
                <WButton onClick={() => setMerging(null)}>{t("Cancel")}</WButton>
              </li>
            ) : null}
          </Fragment>
        ))}
      </WSettingsList>
      <WSettingsList
        label={t("Suggested")}
        description={placeholders > 0 ? t("Some suggested topics have placeholder names until an AI model names them.") : undefined}
        isEmpty={suggestions.length === 0}
        empty={t("When a theme shows up across several of your meetings, emails and notes, Whimbrel suggests it here. Nothing is added until you press Add.")}
      >
        {suggestions.map((s) => {
          const name = names[s.topicId] ?? s.name;
          const problem = problems[s.topicId];
          const why = `rs-topic-sug-${s.topicId}-why`;
          return (
            <li key={s.topicId} className="rs-meetops-suggest">
              <input
                className="w-input"
                aria-label={t("Name")}
                value={name}
                aria-invalid={problem ? true : undefined}
                aria-describedby={problem ? `rs-topic-sug-${s.topicId}-err` : undefined}
                onChange={(e) => {
                  setNames({ ...names, [s.topicId]: e.target.value });
                  setProblems(({ [s.topicId]: _gone, ...rest }) => rest);
                }}
              />
              <span>{itemCount(s.itemCount)}</span>
              <WButton
                disabled={!name.trim()}
                aria-describedby={!name.trim() ? why : undefined}
                onClick={() => {
                  const bad = topicNameProblem(name, seeds, null);
                  if (bad) return setProblems({ ...problems, [s.topicId]: bad });
                  run(acceptTopicSuggestion(s.topicId, name.trim()), t("Could not add that topic."));
                }}
              >
                {t("Add")}
              </WButton>
              <span className="w-visually-hidden" id={why}>
                {t("Enter a name to add this topic.")}
              </span>
              <WButton onClick={() => run(dismissTopicSuggestion(s.topicId), t("Could not dismiss that suggestion."))}>{t("Dismiss")}</WButton>
              {problem ? (
                <p className="w-field-error rs-meetops-sug-error" id={`rs-topic-sug-${s.topicId}-err`} role="alert">
                  {problem}
                </p>
              ) : null}
            </li>
          );
        })}
      </WSettingsList>
    </WSettingsCard>
  );
}
